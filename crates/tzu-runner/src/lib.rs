use std::env;
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use sqlx::postgres::PgPoolOptions;
use tracing;
use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
use sqlx::{PgPool, Row, SqlitePool};
use thiserror::Error;
use tokio::sync::{mpsc, oneshot};
use tzu_acp::{AcpAgentConfig, AcpAgentProcess, ProjectScopedPermissionHandler};
use tzu_core::{
    AgentCandidateGeneration, AgentGenStatus, CodingContextRootSummary, CodingContextSummary,
    CodingDomainAdapter, ContextTraversalSummary, DomainAdapter, DomainKind, GenericDomainAdapter,
    HarnessPlanMetadata, HarnessPlanner, Planner, PlanningRun, ProjectState, RunReport,
    TaskStatus, inspect_goal_prompt, ordered_tasks, parse_plan_candidate_json,
    score_candidates, select_candidate_frontier, static_validator_outcome,
    validate_candidate_common, validate_plan, CandidateDescriptor, CandidateScore, FrontierPolicy,
    SketchStatus,
};
use tzu_repo::{InspectOptions, ProjectContextSnapshot, RepoState, inspect_context, inspect_repo};

const STATE_ID: &str = "project";
const ACTOR_MAILBOX_CAPACITY: usize = 32;

#[derive(Debug, Error)]
pub enum RunnerError {
    #[error("database `{url}` is unavailable: {message}")]
    DatabaseUnavailable { url: String, message: String },
    #[error("database: {0}")]
    Database(#[from] sqlx::Error),
    #[error("json: {0}")]
    Json(#[from] serde_json::Error),
    #[error("planning: {0}")]
    Planning(#[from] tzu_core::PlanError),
    #[error("repo: {0}")]
    Repo(#[from] tzu_repo::RepoError),
    #[error("acp: {0}")]
    Acp(#[from] tzu_acp::AcpError),
    #[error("actor `{0}` is unavailable")]
    ActorUnavailable(&'static str),
    #[error("task `{0}` was not found in the current plan")]
    MissingTask(String),
    #[error("no current plan; run `tzu plan \"<goal>\"` first")]
    MissingPlan,
    #[error("task `{task_id}` is blocked by unfinished dependencies: {unmet}")]
    TaskBlocked {
        task_id: String,
        unmet: String,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum RunMode {
    Mock,
    Real,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum PlanningDomain {
    Generic,
    Coding,
}

impl PlanningDomain {
    #[must_use]
    pub fn kind(self) -> DomainKind {
        match self {
            Self::Generic => DomainKind::Generic,
            Self::Coding => DomainKind::Coding,
        }
    }
}

impl RunMode {
    #[must_use]
    pub fn from_env() -> Self {
        match env::var("TZU_RUN_MODE") {
            Ok(value) if value == "real" => Self::Real,
            _ => Self::Mock,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TzuRunReport {
    pub task_id: String,
    pub status: TaskStatus,
    pub summary: String,
    pub changed_files: Vec<String>,
    pub events: Vec<String>,
    pub generated_at_unix_secs: u64,
}

#[derive(Debug)]
pub struct Init;

impl thespis::Message for Init {
    type Return = Result<ProjectState, RunnerError>;
}

#[derive(Debug)]
pub struct Status;

impl thespis::Message for Status {
    type Return = Result<ProjectState, RunnerError>;
}

#[derive(Debug)]
pub struct CreatePlan {
    pub goal: String,
    pub planning_goal: Option<String>,
    pub domain: PlanningDomain,
    pub context_roots: Vec<PathBuf>,
    pub include_nested_contexts: bool,
}

impl thespis::Message for CreatePlan {
    type Return = Result<ProjectState, RunnerError>;
}

#[derive(Debug)]
pub struct RunTask {
    pub task_id: String,
    pub mode: RunMode,
}

impl thespis::Message for RunTask {
    type Return = Result<TzuRunReport, RunnerError>;
}

#[derive(Debug)]
pub struct InspectRepo;

impl thespis::Message for InspectRepo {
    type Return = Result<RepoState, RunnerError>;
}

pub struct TzuRunner {
    actor: RunnerActorHandle,
}

impl TzuRunner {
    pub async fn connect(
        root: impl Into<PathBuf>,
        database_url: &str,
    ) -> Result<Self, RunnerError> {
        let root = root.into();
        let store = Store::connect(database_url).await?;
        store.migrate().await?;
        Ok(Self {
            actor: RunnerActorHandle::spawn(root, store),
        })
    }

    pub async fn init(&self) -> Result<ProjectState, RunnerError> {
        self.actor.init().await
    }

    pub async fn plan(&self, goal: &str) -> Result<ProjectState, RunnerError> {
        self.plan_with_domain(goal, PlanningDomain::Generic).await
    }

    pub async fn plan_with_domain(
        &self,
        goal: &str,
        domain: PlanningDomain,
    ) -> Result<ProjectState, RunnerError> {
        self.plan_with_context(goal, domain, Vec::new(), false)
            .await
    }

    pub async fn plan_with_context(
        &self,
        goal: &str,
        domain: PlanningDomain,
        context_roots: Vec<PathBuf>,
        include_nested_contexts: bool,
    ) -> Result<ProjectState, RunnerError> {
        self.actor
            .create_plan(CreatePlan {
                goal: goal.to_string(),
                planning_goal: None,
                domain,
                context_roots,
                include_nested_contexts,
            })
            .await
    }

    pub async fn status(&self) -> Result<ProjectState, RunnerError> {
        self.actor.status().await
    }

    pub async fn run_task(
        &self,
        task_id: &str,
        mode: RunMode,
    ) -> Result<TzuRunReport, RunnerError> {
        self.actor
            .run_task(RunTask {
                task_id: task_id.to_string(),
                mode,
            })
            .await
    }

    pub async fn repo_state(&self) -> Result<RepoState, RunnerError> {
        self.actor.inspect_repo().await
    }

    #[must_use]
    pub fn actor(&self) -> RunnerActorHandle {
        self.actor.clone()
    }
}

#[derive(Clone)]
pub struct RunnerActorHandle {
    sender: mpsc::Sender<RunnerCommand>,
}

impl RunnerActorHandle {
    fn spawn(root: PathBuf, store: Store) -> Self {
        let (sender, receiver) = mpsc::channel(ACTOR_MAILBOX_CAPACITY);
        let store = StoreActorHandle::spawn(store);
        let repo = RepoActorHandle::spawn(root.clone());
        let acp = AcpActorHandle::spawn(root.clone());
        tokio::spawn(
            RunnerActor {
                root,
                store,
                repo,
                acp,
                receiver,
            }
            .run(),
        );
        Self { sender }
    }

    pub async fn init(&self) -> Result<ProjectState, RunnerError> {
        self.call(|reply| RunnerCommand::Init { reply }).await
    }

    pub async fn status(&self) -> Result<ProjectState, RunnerError> {
        self.call(|reply| RunnerCommand::Status { reply }).await
    }

    pub async fn create_plan(&self, msg: CreatePlan) -> Result<ProjectState, RunnerError> {
        self.call(|reply| RunnerCommand::CreatePlan { msg, reply })
            .await
    }

    pub async fn run_task(&self, msg: RunTask) -> Result<TzuRunReport, RunnerError> {
        self.call(|reply| RunnerCommand::RunTask { msg, reply })
            .await
    }

    pub async fn inspect_repo(&self) -> Result<RepoState, RunnerError> {
        self.call(|reply| RunnerCommand::InspectRepo { reply })
            .await
    }

    async fn call<T>(
        &self,
        build: impl FnOnce(oneshot::Sender<Result<T, RunnerError>>) -> RunnerCommand,
    ) -> Result<T, RunnerError> {
        let (reply, receive) = oneshot::channel();
        self.sender
            .send(build(reply))
            .await
            .map_err(|_| RunnerError::ActorUnavailable("runner"))?;
        receive
            .await
            .map_err(|_| RunnerError::ActorUnavailable("runner"))?
    }
}

enum RunnerCommand {
    Init {
        reply: oneshot::Sender<Result<ProjectState, RunnerError>>,
    },
    Status {
        reply: oneshot::Sender<Result<ProjectState, RunnerError>>,
    },
    CreatePlan {
        msg: CreatePlan,
        reply: oneshot::Sender<Result<ProjectState, RunnerError>>,
    },
    RunTask {
        msg: RunTask,
        reply: oneshot::Sender<Result<TzuRunReport, RunnerError>>,
    },
    InspectRepo {
        reply: oneshot::Sender<Result<RepoState, RunnerError>>,
    },
}

#[derive(thespis::Actor)]
struct RunnerActor {
    root: PathBuf,
    store: StoreActorHandle,
    repo: RepoActorHandle,
    acp: AcpActorHandle,
    receiver: mpsc::Receiver<RunnerCommand>,
}

impl RunnerActor {
    async fn run(mut self) {
        while let Some(command) = self.receiver.recv().await {
            match command {
                RunnerCommand::Init { reply } => {
                    let _ = reply.send(self.init().await);
                }
                RunnerCommand::Status { reply } => {
                    let _ = reply.send(self.status().await);
                }
                RunnerCommand::CreatePlan { msg, reply } => {
                    let _ = reply.send(self.create_plan(msg).await);
                }
                RunnerCommand::RunTask { msg, reply } => {
                    let _ = reply.send(self.run_task(msg).await);
                }
                RunnerCommand::InspectRepo { reply } => {
                    let _ = reply.send(self.repo.inspect().await);
                }
            }
        }
    }

    async fn init(&self) -> Result<ProjectState, RunnerError> {
        let state = self
            .store
            .load(&self.root)
            .await?
            .unwrap_or_else(|| ProjectState::new(self.root.display().to_string()));
        self.store.save(state.clone()).await?;
        Ok(state)
    }

    async fn status(&self) -> Result<ProjectState, RunnerError> {
        Ok(self
            .store
            .load(&self.root)
            .await?
            .unwrap_or_else(|| ProjectState::new(self.root.display().to_string())))
    }

    async fn create_plan(&self, msg: CreatePlan) -> Result<ProjectState, RunnerError> {
        let planning_goal = msg.planning_goal.as_deref().unwrap_or(&msg.goal);
        let prompt_inspection = inspect_goal_prompt(planning_goal, msg.domain.kind());
        if prompt_inspection.needs_improvement() {
            return Err(
                tzu_core::PlanError::PromptNeedsImprovement(Box::new(prompt_inspection)).into(),
            );
        }
        let context_snapshot = if msg.domain == PlanningDomain::Coding {
            Some(self.context_snapshot(&msg).await?)
        } else {
            None
        };

        let agent_prompt = match msg.domain {
            PlanningDomain::Generic => {
                let adapter = GenericDomainAdapter;
                let spec = adapter.build_spec(planning_goal);
                adapter.generate_candidate_prompt(&spec)
            }
            PlanningDomain::Coding => {
                let snapshot = context_snapshot
                    .as_ref()
                    .expect("coding context snapshot exists for coding plan");
                let adapter = CodingDomainAdapter {
                    project_root: self.root.display().to_string(),
                    context: coding_context_summary(snapshot),
                };
                let spec = adapter.build_spec(planning_goal);
                adapter.generate_candidate_prompt(&spec)
            }
        };

        let mut plan = match msg.domain {
            PlanningDomain::Generic => {
                let planner = HarnessPlanner::new(GenericDomainAdapter);
                planner.create_plan(planning_goal).await?
            }
            PlanningDomain::Coding => {
                let snapshot = context_snapshot
                    .as_ref()
                    .expect("coding context snapshot exists for coding plan");
                let planner = HarnessPlanner::new(CodingDomainAdapter {
                    project_root: self.root.display().to_string(),
                    context: coding_context_summary(snapshot),
                });
                planner.create_plan(planning_goal).await?
            }
        };
        plan.goal = msg.goal.clone();
        validate_plan(&plan)?;

        let mut state = self
            .store
            .load(&self.root)
            .await?
            .unwrap_or_else(|| ProjectState::new(self.root.display().to_string()));
        let planning_run = plan.harness.as_ref().map(|harness| PlanningRun {
            id: next_planning_run_id(&state),
            domain: msg.domain.kind(),
            problem_spec: harness.problem_spec.clone(),
            selected_candidate_id: harness.selected_candidate_id.clone(),
            candidate_count: harness.candidates.len(),
        });
        if plan.harness.is_some() {
            state.planning_runs.push(
                planning_run
                    .as_ref()
                    .expect("planning run exists when harness exists")
                    .clone(),
            );
        }
        state.current_plan = Some(plan);

        if agent_prompt.is_some() {
            if let Some(harness) = state
                .current_plan
                .as_mut()
                .and_then(|plan| plan.harness.as_mut())
            {
                let batch_id = format!("agent-{}", now_unix_secs());
                harness.agent_generation = Some(AgentCandidateGeneration {
                    batch_id: batch_id.clone(),
                    status: AgentGenStatus::InProgress,
                    started_at_unix_secs: now_unix_secs(),
                });
            }
        }
        self.store.save(state.clone()).await?;
        if let (Some(run), Some(harness)) = (
            planning_run,
            state
                .current_plan
                .as_ref()
                .and_then(|plan| plan.harness.clone()),
        ) {
            let run_id = run.id.clone();
            self.store.save_planning_artifacts(run, harness).await?;
            if let Some(snapshot) = context_snapshot {
                self.store.save_context_snapshot(&run_id, snapshot).await?;
            }
        }

        if let Some(prompt) = agent_prompt {
            let store = self.store.clone();
            let acp = self.acp.clone();
            let root = self.root.clone();
            let batch_id = state
                .current_plan
                .as_ref()
                .and_then(|plan| plan.harness.as_ref())
                .and_then(|harness| harness.agent_generation.as_ref())
                .map(|g| g.batch_id.clone())
                .unwrap_or_default();
            tokio::spawn(async move {
                let _ = Self::generate_agent_candidates(store, acp, root, prompt, batch_id)
                    .await;
            });
        }

        Ok(state)
    }

    async fn generate_agent_candidates(
        store: StoreActorHandle,
        acp: AcpActorHandle,
        root: PathBuf,
        prompt: String,
        batch_id: String,
    ) -> Result<(), RunnerError> {
        let output = match acp.prompt(prompt.clone()).await {
            Ok(output) => output,
            Err(error) => {
                Self::mark_agent_generation_failed(&store, &root, &batch_id, &error).await;
                return Err(error);
            }
        };

        let candidates = parse_plan_candidate_json(&output.text);
        if candidates.is_empty() {
            Self::mark_agent_generation_failed(
                &store,
                &root,
                &batch_id,
                &RunnerError::Acp(tzu_acp::AcpError::Transport(
                    "agent returned no parseable plan candidates".to_string(),
                )),
            )
            .await;
            return Ok(());
        }

        let mut state = store
            .load(&root)
            .await?
            .unwrap_or_else(|| ProjectState::new(root.display().to_string()));

        let Some(plan) = state.current_plan.as_mut() else {
            return Ok(());
        };
        let Some(ref harness) = plan.harness.clone() else {
            return Ok(());
        };
        let spec = &harness.problem_spec;
        let existing_count = harness.candidates.len();

        let mut agent_sketches: Vec<tzu_core::PlanSketch> = candidates
            .into_iter()
            .enumerate()
            .map(|(idx, candidate)| {
                let validation = validate_candidate_common(spec, &candidate);
                let status = if validation.is_valid() {
                    SketchStatus::Valid
                } else {
                    SketchStatus::Invalid
                };
                tzu_core::PlanSketch {
                    id: format!("candidate-{}", existing_count + idx + 1),
                    problem_id: spec.id.clone(),
                    parent_ids: Vec::new(),
                    candidate,
                    status,
                    validation,
                    score: CandidateScore::default(),
                    descriptor: CandidateDescriptor::default(),
                    created_by: "agent".to_string(),
                }
            })
            .collect();

        if let Some(harness) = plan.harness.as_mut() {
            score_candidates(&mut agent_sketches);
            harness.candidates.append(&mut agent_sketches);
            match select_candidate_frontier(&mut harness.candidates, FrontierPolicy::default()) {
                Ok(frontier) => {
                    harness.frontier = frontier;
                    harness.selected_candidate_id = harness.frontier.selected_candidate_id.clone();
                }
                Err(_) => {}
            }
            if let Some(selected) = harness
                .candidates
                .iter()
                .find(|candidate| candidate.id == harness.selected_candidate_id)
            {
                plan.tasks = selected.candidate.tasks.clone();
            }
            harness.agent_generation = Some(AgentCandidateGeneration {
                batch_id,
                status: AgentGenStatus::Complete,
                started_at_unix_secs: now_unix_secs(),
            });
        }

        store.save(state).await
    }

    async fn mark_agent_generation_failed(
        store: &StoreActorHandle,
        root: &Path,
        batch_id: &str,
        error: &RunnerError,
    ) {
        if let Ok(Some(mut state)) = store.load(root).await {
            if let Some(plan) = state.current_plan.as_mut() {
                if let Some(harness) = plan.harness.as_mut() {
                    if let Some(g) = harness.agent_generation.as_mut() {
                        g.status = AgentGenStatus::Failed;
                    }
                }
            }
            let _ = store.save(state).await;
        }
        tracing::error!(?error, %batch_id, "agent candidate generation failed");
    }

    async fn context_snapshot(
        &self,
        msg: &CreatePlan,
    ) -> Result<ProjectContextSnapshot, RunnerError> {
        let context_roots = if msg.context_roots.is_empty() {
            vec![self.root.clone()]
        } else {
            msg.context_roots.clone()
        };
        inspect_context(
            &self.root,
            InspectOptions {
                context_roots,
                include_nested_contexts: msg.include_nested_contexts,
                ..InspectOptions::default()
            },
        )
        .await
        .map_err(RunnerError::Repo)
    }

    async fn run_task(&self, msg: RunTask) -> Result<TzuRunReport, RunnerError> {
        let mut state = self.status().await?;
        let plan = state.current_plan.clone().ok_or(RunnerError::MissingPlan)?;
        let ordered = ordered_tasks(&plan)?;
        let task = ordered
            .into_iter()
            .find(|task| task.id == msg.task_id)
            .ok_or_else(|| RunnerError::MissingTask(msg.task_id.clone()))?;

        let unmet: Vec<String> = task
            .depends_on
            .iter()
            .filter(|dep_id| {
                plan.tasks
                    .iter()
                    .find(|t| t.id == **dep_id)
                    .is_none_or(|t| t.status != TaskStatus::Completed)
            })
            .cloned()
            .collect();
        if !unmet.is_empty() {
            return Err(RunnerError::TaskBlocked {
                task_id: task.id.clone(),
                unmet: unmet.join(", "),
            });
        }

        let repo = self.repo.inspect().await?;
        let report = match msg.mode {
            RunMode::Mock => TzuRunReport {
                task_id: task.id.clone(),
                status: TaskStatus::Completed,
                summary: format!(
                    "Mocked ACP run completed for `{}` with {} indexed files.",
                    task.id,
                    repo.files.len()
                ),
                changed_files: Vec::new(),
                events: vec![
                    "mock-acp:start".to_string(),
                    format!("mock-acp:repo-files={}", repo.files.len()),
                    "mock-acp:complete".to_string(),
                ],
                generated_at_unix_secs: now_unix_secs(),
            },
            RunMode::Real => {
                let output = self.acp.prompt(task.description.clone()).await?;
                TzuRunReport {
                    task_id: task.id.clone(),
                    status: TaskStatus::Completed,
                    summary: output.text,
                    changed_files: Vec::new(),
                    events: output
                        .events
                        .into_iter()
                        .map(|event| event.method)
                        .collect(),
                    generated_at_unix_secs: now_unix_secs(),
                }
            }
        };

        if let Some(plan) = state.current_plan.as_mut()
            && let Some(task) = plan.tasks.iter_mut().find(|task| task.id == msg.task_id)
        {
            task.status = report.status;
        }
        state.run_reports.push(RunReport {
            task_id: report.task_id.clone(),
            status: report.status,
            summary: report.summary.clone(),
            changed_files: report.changed_files.clone(),
            events: report.events.clone(),
        });
        self.store.save(state).await?;
        Ok(report)
    }
}

#[derive(Clone)]
struct StoreActorHandle {
    sender: mpsc::Sender<StoreCommand>,
}

impl StoreActorHandle {
    fn spawn(store: Store) -> Self {
        let (sender, receiver) = mpsc::channel(ACTOR_MAILBOX_CAPACITY);
        tokio::spawn(StoreActor { store, receiver }.run());
        Self { sender }
    }

    async fn load(&self, root: &Path) -> Result<Option<ProjectState>, RunnerError> {
        let root = root.to_path_buf();
        self.call(|reply| StoreCommand::Load { root, reply }).await
    }

    async fn save(&self, state: ProjectState) -> Result<(), RunnerError> {
        self.call(|reply| StoreCommand::Save {
            state: Box::new(state),
            reply,
        })
        .await
    }

    async fn save_planning_artifacts(
        &self,
        run: PlanningRun,
        harness: HarnessPlanMetadata,
    ) -> Result<(), RunnerError> {
        self.call(|reply| StoreCommand::SavePlanningArtifacts {
            run: Box::new(run),
            harness: Box::new(harness),
            reply,
        })
        .await
    }

    async fn save_context_snapshot(
        &self,
        run_id: &str,
        snapshot: ProjectContextSnapshot,
    ) -> Result<(), RunnerError> {
        let run_id = run_id.to_string();
        self.call(|reply| StoreCommand::SaveContextSnapshot {
            run_id,
            snapshot: Box::new(snapshot),
            reply,
        })
        .await
    }

    async fn call<T>(
        &self,
        build: impl FnOnce(oneshot::Sender<Result<T, RunnerError>>) -> StoreCommand,
    ) -> Result<T, RunnerError> {
        let (reply, receive) = oneshot::channel();
        self.sender
            .send(build(reply))
            .await
            .map_err(|_| RunnerError::ActorUnavailable("store"))?;
        receive
            .await
            .map_err(|_| RunnerError::ActorUnavailable("store"))?
    }
}

enum StoreCommand {
    Load {
        root: PathBuf,
        reply: oneshot::Sender<Result<Option<ProjectState>, RunnerError>>,
    },
    Save {
        state: Box<ProjectState>,
        reply: oneshot::Sender<Result<(), RunnerError>>,
    },
    SavePlanningArtifacts {
        run: Box<PlanningRun>,
        harness: Box<HarnessPlanMetadata>,
        reply: oneshot::Sender<Result<(), RunnerError>>,
    },
    SaveContextSnapshot {
        run_id: String,
        snapshot: Box<ProjectContextSnapshot>,
        reply: oneshot::Sender<Result<(), RunnerError>>,
    },
}

#[derive(thespis::Actor)]
struct StoreActor {
    store: Store,
    receiver: mpsc::Receiver<StoreCommand>,
}

impl StoreActor {
    async fn run(mut self) {
        while let Some(command) = self.receiver.recv().await {
            match command {
                StoreCommand::Load { root, reply } => {
                    let _ = reply.send(self.store.load(&root).await);
                }
                StoreCommand::Save { state, reply } => {
                    let _ = reply.send(self.store.save(&state).await);
                }
                StoreCommand::SavePlanningArtifacts {
                    run,
                    harness,
                    reply,
                } => {
                    let _ = reply.send(self.store.save_planning_artifacts(&run, &harness).await);
                }
                StoreCommand::SaveContextSnapshot {
                    run_id,
                    snapshot,
                    reply,
                } => {
                    let _ = reply.send(self.store.save_context_snapshot(&run_id, &snapshot).await);
                }
            }
        }
    }
}

#[derive(Clone)]
struct RepoActorHandle {
    sender: mpsc::Sender<RepoCommand>,
}

impl RepoActorHandle {
    fn spawn(root: PathBuf) -> Self {
        let (sender, receiver) = mpsc::channel(ACTOR_MAILBOX_CAPACITY);
        tokio::spawn(RepoActor { root, receiver }.run());
        Self { sender }
    }

    async fn inspect(&self) -> Result<RepoState, RunnerError> {
        let (reply, receive) = oneshot::channel();
        self.sender
            .send(RepoCommand::Inspect { reply })
            .await
            .map_err(|_| RunnerError::ActorUnavailable("repo"))?;
        receive
            .await
            .map_err(|_| RunnerError::ActorUnavailable("repo"))?
    }
}

enum RepoCommand {
    Inspect {
        reply: oneshot::Sender<Result<RepoState, RunnerError>>,
    },
}

#[derive(thespis::Actor)]
struct RepoActor {
    root: PathBuf,
    receiver: mpsc::Receiver<RepoCommand>,
}

impl RepoActor {
    async fn run(mut self) {
        while let Some(command) = self.receiver.recv().await {
            match command {
                RepoCommand::Inspect { reply } => {
                    let _ = reply.send(inspect_repo(&self.root).await.map_err(RunnerError::Repo));
                }
            }
        }
    }
}

#[derive(Clone)]
struct AcpActorHandle {
    sender: mpsc::Sender<AcpCommand>,
}

impl AcpActorHandle {
    fn spawn(root: PathBuf) -> Self {
        let (sender, receiver) = mpsc::channel(ACTOR_MAILBOX_CAPACITY);
        tokio::spawn(AcpActor { root, receiver }.run());
        Self { sender }
    }

    async fn prompt(&self, prompt: String) -> Result<tzu_acp::AcpRunOutput, RunnerError> {
        let (reply, receive) = oneshot::channel();
        self.sender
            .send(AcpCommand::Prompt { prompt, reply })
            .await
            .map_err(|_| RunnerError::ActorUnavailable("acp"))?;
        receive
            .await
            .map_err(|_| RunnerError::ActorUnavailable("acp"))?
    }
}

enum AcpCommand {
    Prompt {
        prompt: String,
        reply: oneshot::Sender<Result<tzu_acp::AcpRunOutput, RunnerError>>,
    },
}

#[derive(thespis::Actor)]
struct AcpActor {
    root: PathBuf,
    receiver: mpsc::Receiver<AcpCommand>,
}

impl AcpActor {
    async fn run(mut self) {
        while let Some(command) = self.receiver.recv().await {
            match command {
                AcpCommand::Prompt { prompt, reply } => {
                    let result = self.prompt(&prompt).await;
                    let _ = reply.send(result);
                }
            }
        }
    }

    async fn prompt(&self, prompt: &str) -> Result<tzu_acp::AcpRunOutput, RunnerError> {
        let mut process = AcpAgentProcess::spawn(&AcpAgentConfig::from_env(&self.root)).await?;
        let _ = process.initialize().await?;
        let mut permissions = ProjectScopedPermissionHandler::new(self.root.clone());
        process
            .prompt(&self.root, prompt, &mut permissions)
            .await
            .map_err(RunnerError::Acp)
    }
}

#[must_use]
pub fn default_database_url(root: &Path) -> String {
    if let Ok(value) = env::var("TZU_DATABASE_URL") {
        return value;
    }
    if cfg!(target_os = "linux") {
        "postgres:///tzu?host=/run/postgresql".to_string()
    } else {
        root.join(".tzu/state.sqlite")
            .to_str()
            .map(|path| format!("sqlite://{path}"))
            .unwrap_or_else(|| "sqlite://.tzu/state.sqlite".to_string())
    }
}

enum Store {
    Sqlite(SqlitePool),
    Postgres(PgPool),
}

impl Store {
    async fn connect(url: &str) -> Result<Self, RunnerError> {
        if url.starts_with("sqlite:") {
            let path = sqlite_path(url);
            if let Some(parent) = path.parent()
                && !parent.as_os_str().is_empty()
            {
                tokio::fs::create_dir_all(parent).await.map_err(|err| {
                    RunnerError::DatabaseUnavailable {
                        url: url.to_string(),
                        message: err.to_string(),
                    }
                })?;
            }
            let options = SqliteConnectOptions::from_str(url)
                .map_err(|err| RunnerError::DatabaseUnavailable {
                    url: url.to_string(),
                    message: err.to_string(),
                })?
                .create_if_missing(true);
            let pool = SqlitePoolOptions::new()
                .max_connections(1)
                .connect_with(options)
                .await
                .map_err(|err| RunnerError::DatabaseUnavailable {
                    url: url.to_string(),
                    message: err.to_string(),
                })?;
            Ok(Self::Sqlite(pool))
        } else {
            let pool = PgPoolOptions::new()
                .max_connections(5)
                .connect(url)
                .await
                .map_err(|err| RunnerError::DatabaseUnavailable {
                    url: url.to_string(),
                    message: postgres_hint(url, &err),
                })?;
            Ok(Self::Postgres(pool))
        }
    }

    async fn migrate(&self) -> Result<(), RunnerError> {
        match self {
            Self::Sqlite(pool) => {
                sqlx::query(
                    "CREATE TABLE IF NOT EXISTS project_state (
                        id TEXT PRIMARY KEY,
                        project_root TEXT NOT NULL,
                        state_json TEXT NOT NULL,
                        updated_at TEXT NOT NULL
                    )",
                )
                .execute(pool)
                .await?;
                sqlx::query(
                    "CREATE TABLE IF NOT EXISTS planning_runs (
                        id TEXT PRIMARY KEY,
                        run_json TEXT NOT NULL,
                        updated_at TEXT NOT NULL
                    )",
                )
                .execute(pool)
                .await?;
                sqlx::query(
                    "CREATE TABLE IF NOT EXISTS plan_candidates (
                        id TEXT PRIMARY KEY,
                        run_id TEXT NOT NULL,
                        candidate_json TEXT NOT NULL,
                        updated_at TEXT NOT NULL
                    )",
                )
                .execute(pool)
                .await?;
                sqlx::query(
                    "CREATE TABLE IF NOT EXISTS plan_matches (
                        id TEXT PRIMARY KEY,
                        run_id TEXT NOT NULL,
                        match_json TEXT NOT NULL,
                        updated_at TEXT NOT NULL
                    )",
                )
                .execute(pool)
                .await?;
                sqlx::query(
                    "CREATE TABLE IF NOT EXISTS obligations (
                        id TEXT PRIMARY KEY,
                        run_id TEXT NOT NULL,
                        obligation_json TEXT NOT NULL,
                        updated_at TEXT NOT NULL
                    )",
                )
                .execute(pool)
                .await?;
                sqlx::query(
                    "CREATE TABLE IF NOT EXISTS agent_runs (
                        id TEXT PRIMARY KEY,
                        run_id TEXT NOT NULL,
                        agent_json TEXT NOT NULL,
                        updated_at TEXT NOT NULL
                    )",
                )
                .execute(pool)
                .await?;
                sqlx::query(
                    "CREATE TABLE IF NOT EXISTS validator_runs (
                        id TEXT PRIMARY KEY,
                        run_id TEXT NOT NULL,
                        validator_json TEXT NOT NULL,
                        updated_at TEXT NOT NULL
                    )",
                )
                .execute(pool)
                .await?;
                sqlx::query(
                    "CREATE TABLE IF NOT EXISTS context_snapshots (
                        id TEXT PRIMARY KEY,
                        run_id TEXT NOT NULL,
                        snapshot_json TEXT NOT NULL,
                        updated_at TEXT NOT NULL
                    )",
                )
                .execute(pool)
                .await?;
            }
            Self::Postgres(pool) => {
                sqlx::query(
                    "CREATE TABLE IF NOT EXISTS project_state (
                        id TEXT PRIMARY KEY,
                        project_root TEXT NOT NULL,
                        state_json TEXT NOT NULL,
                        updated_at TEXT NOT NULL
                    )",
                )
                .execute(pool)
                .await?;
                sqlx::query(
                    "CREATE TABLE IF NOT EXISTS planning_runs (
                        id TEXT PRIMARY KEY,
                        run_json TEXT NOT NULL,
                        updated_at TEXT NOT NULL
                    )",
                )
                .execute(pool)
                .await?;
                sqlx::query(
                    "CREATE TABLE IF NOT EXISTS plan_candidates (
                        id TEXT PRIMARY KEY,
                        run_id TEXT NOT NULL,
                        candidate_json TEXT NOT NULL,
                        updated_at TEXT NOT NULL
                    )",
                )
                .execute(pool)
                .await?;
                sqlx::query(
                    "CREATE TABLE IF NOT EXISTS plan_matches (
                        id TEXT PRIMARY KEY,
                        run_id TEXT NOT NULL,
                        match_json TEXT NOT NULL,
                        updated_at TEXT NOT NULL
                    )",
                )
                .execute(pool)
                .await?;
                sqlx::query(
                    "CREATE TABLE IF NOT EXISTS obligations (
                        id TEXT PRIMARY KEY,
                        run_id TEXT NOT NULL,
                        obligation_json TEXT NOT NULL,
                        updated_at TEXT NOT NULL
                    )",
                )
                .execute(pool)
                .await?;
                sqlx::query(
                    "CREATE TABLE IF NOT EXISTS agent_runs (
                        id TEXT PRIMARY KEY,
                        run_id TEXT NOT NULL,
                        agent_json TEXT NOT NULL,
                        updated_at TEXT NOT NULL
                    )",
                )
                .execute(pool)
                .await?;
                sqlx::query(
                    "CREATE TABLE IF NOT EXISTS validator_runs (
                        id TEXT PRIMARY KEY,
                        run_id TEXT NOT NULL,
                        validator_json TEXT NOT NULL,
                        updated_at TEXT NOT NULL
                    )",
                )
                .execute(pool)
                .await?;
                sqlx::query(
                    "CREATE TABLE IF NOT EXISTS context_snapshots (
                        id TEXT PRIMARY KEY,
                        run_id TEXT NOT NULL,
                        snapshot_json TEXT NOT NULL,
                        updated_at TEXT NOT NULL
                    )",
                )
                .execute(pool)
                .await?;
            }
        }
        Ok(())
    }

    async fn load(&self, root: &Path) -> Result<Option<ProjectState>, RunnerError> {
        let state = match self {
            Self::Sqlite(pool) => {
                let row = sqlx::query("SELECT state_json FROM project_state WHERE id = ?")
                    .bind(STATE_ID)
                    .fetch_optional(pool)
                    .await?;
                row.map(|row| {
                    let json: String = row.get("state_json");
                    serde_json::from_str(&json).map_err(RunnerError::Json)
                })
                .transpose()?
            }
            Self::Postgres(pool) => {
                let row = sqlx::query("SELECT state_json FROM project_state WHERE id = $1")
                    .bind(STATE_ID)
                    .fetch_optional(pool)
                    .await?;
                row.map(|row| {
                    let json: String = row.get("state_json");
                    serde_json::from_str(&json).map_err(RunnerError::Json)
                })
                .transpose()?
            }
        };

        Ok(state.or_else(|| Some(ProjectState::new(root.display().to_string()))))
    }

    async fn save(&self, state: &ProjectState) -> Result<(), RunnerError> {
        let json = serde_json::to_string_pretty(state)?;
        let updated_at = now_unix_secs().to_string();
        match self {
            Self::Sqlite(pool) => {
                sqlx::query(
                    "INSERT INTO project_state (id, project_root, state_json, updated_at)
                     VALUES (?, ?, ?, ?)
                     ON CONFLICT(id) DO UPDATE SET
                        project_root = excluded.project_root,
                        state_json = excluded.state_json,
                        updated_at = excluded.updated_at",
                )
                .bind(STATE_ID)
                .bind(&state.project_root)
                .bind(json)
                .bind(updated_at)
                .execute(pool)
                .await?;
            }
            Self::Postgres(pool) => {
                sqlx::query(
                    "INSERT INTO project_state (id, project_root, state_json, updated_at)
                     VALUES ($1, $2, $3, $4)
                     ON CONFLICT(id) DO UPDATE SET
                        project_root = excluded.project_root,
                        state_json = excluded.state_json,
                        updated_at = excluded.updated_at",
                )
                .bind(STATE_ID)
                .bind(&state.project_root)
                .bind(json)
                .bind(updated_at)
                .execute(pool)
                .await?;
            }
        }
        Ok(())
    }

    async fn save_planning_artifacts(
        &self,
        run: &PlanningRun,
        harness: &HarnessPlanMetadata,
    ) -> Result<(), RunnerError> {
        let generated_at = now_unix_secs();
        let updated_at = generated_at.to_string();
        let run_json = serde_json::to_string_pretty(run)?;
        let baseline_evidence_ref_count = harness.problem_spec.evidence.len();
        match self {
            Self::Sqlite(pool) => {
                insert_planning_run_sqlite(pool, run, &run_json, &updated_at).await?;
                for candidate in &harness.candidates {
                    let id = scoped_artifact_id(&run.id, &candidate.id);
                    let candidate_json = serde_json::to_string_pretty(candidate)?;
                    insert_plan_candidate_sqlite(pool, &id, &run.id, &candidate_json, &updated_at)
                        .await?;
                    let validator_id = scoped_artifact_id(&id, "validator-static");
                    let outcome = static_validator_outcome(
                        &run.id,
                        candidate,
                        baseline_evidence_ref_count,
                        generated_at,
                    );
                    let validator_json = serde_json::to_string_pretty(&outcome)?;
                    insert_validator_run_sqlite(
                        pool,
                        &validator_id,
                        &run.id,
                        &validator_json,
                        &updated_at,
                    )
                    .await?;
                    for (idx, obligation) in candidate.validation.obligations.iter().enumerate() {
                        let obligation_id = scoped_artifact_id(&id, &format!("obligation-{idx}"));
                        let obligation_json = serde_json::to_string_pretty(obligation)?;
                        insert_obligation_sqlite(
                            pool,
                            &obligation_id,
                            &run.id,
                            &obligation_json,
                            &updated_at,
                        )
                        .await?;
                    }
                }
                for (idx, match_result) in harness.matches.iter().enumerate() {
                    let id = scoped_artifact_id(&run.id, &format!("match-{idx}"));
                    let match_json = serde_json::to_string_pretty(match_result)?;
                    insert_plan_match_sqlite(pool, &id, &run.id, &match_json, &updated_at).await?;
                }
            }
            Self::Postgres(pool) => {
                insert_planning_run_postgres(pool, run, &run_json, &updated_at).await?;
                for candidate in &harness.candidates {
                    let id = scoped_artifact_id(&run.id, &candidate.id);
                    let candidate_json = serde_json::to_string_pretty(candidate)?;
                    insert_plan_candidate_postgres(
                        pool,
                        &id,
                        &run.id,
                        &candidate_json,
                        &updated_at,
                    )
                    .await?;
                    let validator_id = scoped_artifact_id(&id, "validator-static");
                    let outcome = static_validator_outcome(
                        &run.id,
                        candidate,
                        baseline_evidence_ref_count,
                        generated_at,
                    );
                    let validator_json = serde_json::to_string_pretty(&outcome)?;
                    insert_validator_run_postgres(
                        pool,
                        &validator_id,
                        &run.id,
                        &validator_json,
                        &updated_at,
                    )
                    .await?;
                    for (idx, obligation) in candidate.validation.obligations.iter().enumerate() {
                        let obligation_id = scoped_artifact_id(&id, &format!("obligation-{idx}"));
                        let obligation_json = serde_json::to_string_pretty(obligation)?;
                        insert_obligation_postgres(
                            pool,
                            &obligation_id,
                            &run.id,
                            &obligation_json,
                            &updated_at,
                        )
                        .await?;
                    }
                }
                for (idx, match_result) in harness.matches.iter().enumerate() {
                    let id = scoped_artifact_id(&run.id, &format!("match-{idx}"));
                    let match_json = serde_json::to_string_pretty(match_result)?;
                    insert_plan_match_postgres(pool, &id, &run.id, &match_json, &updated_at)
                        .await?;
                }
            }
        }
        Ok(())
    }

    async fn save_context_snapshot(
        &self,
        run_id: &str,
        snapshot: &ProjectContextSnapshot,
    ) -> Result<(), RunnerError> {
        let updated_at = now_unix_secs().to_string();
        let snapshot_json = serde_json::to_string_pretty(snapshot)?;
        match self {
            Self::Sqlite(pool) => {
                sqlx::query(
                    "INSERT INTO context_snapshots (id, run_id, snapshot_json, updated_at)
                     VALUES (?, ?, ?, ?)
                     ON CONFLICT(id) DO UPDATE SET
                        run_id = excluded.run_id,
                        snapshot_json = excluded.snapshot_json,
                        updated_at = excluded.updated_at",
                )
                .bind(&snapshot.id)
                .bind(run_id)
                .bind(snapshot_json)
                .bind(updated_at)
                .execute(pool)
                .await?;
            }
            Self::Postgres(pool) => {
                sqlx::query(
                    "INSERT INTO context_snapshots (id, run_id, snapshot_json, updated_at)
                     VALUES ($1, $2, $3, $4)
                     ON CONFLICT(id) DO UPDATE SET
                        run_id = excluded.run_id,
                        snapshot_json = excluded.snapshot_json,
                        updated_at = excluded.updated_at",
                )
                .bind(&snapshot.id)
                .bind(run_id)
                .bind(snapshot_json)
                .bind(updated_at)
                .execute(pool)
                .await?;
            }
        }
        Ok(())
    }
}

fn sqlite_path(url: &str) -> PathBuf {
    PathBuf::from(url.trim_start_matches("sqlite://"))
}

fn coding_context_summary(snapshot: &ProjectContextSnapshot) -> CodingContextSummary {
    CodingContextSummary {
        snapshot_id: snapshot.id.clone(),
        summary: snapshot.summary.clone(),
        roots: snapshot
            .roots
            .iter()
            .map(|root| CodingContextRootSummary {
                id: root.id.clone(),
                root: root.root.display().to_string(),
                head: root.head.clone(),
                dirty: root.dirty,
                file_count: root.files.len(),
                languages: root
                    .languages
                    .iter()
                    .map(|(language, count)| format!("{language}={count}"))
                    .collect(),
                manifests: root
                    .manifests
                    .iter()
                    .map(|doc| doc.path.display().to_string())
                    .collect(),
                docs: root
                    .docs
                    .iter()
                    .map(|doc| doc.path.display().to_string())
                    .collect(),
                nested_boundaries: root.boundaries.len(),
                traversal: ContextTraversalSummary {
                    traversed_entries: root.traversal.traversed_entries,
                    indexed_files: root.traversal.indexed_files,
                    skipped_ignored_entries: root.traversal.skipped_ignored_entries,
                    walk_errors: root.traversal.walk_errors,
                    skipped_nested_contexts: root.traversal.skipped_nested_contexts,
                    skipped_after_limit: root.traversal.skipped_after_limit,
                },
            })
            .collect(),
    }
}

async fn insert_planning_run_sqlite(
    pool: &SqlitePool,
    run: &PlanningRun,
    run_json: &str,
    updated_at: &str,
) -> Result<(), RunnerError> {
    sqlx::query(
        "INSERT INTO planning_runs (id, run_json, updated_at)
         VALUES (?, ?, ?)
         ON CONFLICT(id) DO UPDATE SET
            run_json = excluded.run_json,
            updated_at = excluded.updated_at",
    )
    .bind(&run.id)
    .bind(run_json)
    .bind(updated_at)
    .execute(pool)
    .await?;
    Ok(())
}

async fn insert_plan_candidate_sqlite(
    pool: &SqlitePool,
    id: &str,
    run_id: &str,
    candidate_json: &str,
    updated_at: &str,
) -> Result<(), RunnerError> {
    sqlx::query(
        "INSERT INTO plan_candidates (id, run_id, candidate_json, updated_at)
         VALUES (?, ?, ?, ?)
         ON CONFLICT(id) DO UPDATE SET
            run_id = excluded.run_id,
            candidate_json = excluded.candidate_json,
            updated_at = excluded.updated_at",
    )
    .bind(id)
    .bind(run_id)
    .bind(candidate_json)
    .bind(updated_at)
    .execute(pool)
    .await?;
    Ok(())
}

async fn insert_plan_match_sqlite(
    pool: &SqlitePool,
    id: &str,
    run_id: &str,
    match_json: &str,
    updated_at: &str,
) -> Result<(), RunnerError> {
    sqlx::query(
        "INSERT INTO plan_matches (id, run_id, match_json, updated_at)
         VALUES (?, ?, ?, ?)
         ON CONFLICT(id) DO UPDATE SET
            run_id = excluded.run_id,
            match_json = excluded.match_json,
            updated_at = excluded.updated_at",
    )
    .bind(id)
    .bind(run_id)
    .bind(match_json)
    .bind(updated_at)
    .execute(pool)
    .await?;
    Ok(())
}

async fn insert_obligation_sqlite(
    pool: &SqlitePool,
    id: &str,
    run_id: &str,
    obligation_json: &str,
    updated_at: &str,
) -> Result<(), RunnerError> {
    sqlx::query(
        "INSERT INTO obligations (id, run_id, obligation_json, updated_at)
         VALUES (?, ?, ?, ?)
         ON CONFLICT(id) DO UPDATE SET
            run_id = excluded.run_id,
            obligation_json = excluded.obligation_json,
            updated_at = excluded.updated_at",
    )
    .bind(id)
    .bind(run_id)
    .bind(obligation_json)
    .bind(updated_at)
    .execute(pool)
    .await?;
    Ok(())
}

async fn insert_validator_run_sqlite(
    pool: &SqlitePool,
    id: &str,
    run_id: &str,
    validator_json: &str,
    updated_at: &str,
) -> Result<(), RunnerError> {
    sqlx::query(
        "INSERT INTO validator_runs (id, run_id, validator_json, updated_at)
         VALUES (?, ?, ?, ?)
         ON CONFLICT(id) DO UPDATE SET
            run_id = excluded.run_id,
            validator_json = excluded.validator_json,
            updated_at = excluded.updated_at",
    )
    .bind(id)
    .bind(run_id)
    .bind(validator_json)
    .bind(updated_at)
    .execute(pool)
    .await?;
    Ok(())
}

async fn insert_planning_run_postgres(
    pool: &PgPool,
    run: &PlanningRun,
    run_json: &str,
    updated_at: &str,
) -> Result<(), RunnerError> {
    sqlx::query(
        "INSERT INTO planning_runs (id, run_json, updated_at)
         VALUES ($1, $2, $3)
         ON CONFLICT(id) DO UPDATE SET
            run_json = excluded.run_json,
            updated_at = excluded.updated_at",
    )
    .bind(&run.id)
    .bind(run_json)
    .bind(updated_at)
    .execute(pool)
    .await?;
    Ok(())
}

async fn insert_plan_candidate_postgres(
    pool: &PgPool,
    id: &str,
    run_id: &str,
    candidate_json: &str,
    updated_at: &str,
) -> Result<(), RunnerError> {
    sqlx::query(
        "INSERT INTO plan_candidates (id, run_id, candidate_json, updated_at)
         VALUES ($1, $2, $3, $4)
         ON CONFLICT(id) DO UPDATE SET
            run_id = excluded.run_id,
            candidate_json = excluded.candidate_json,
            updated_at = excluded.updated_at",
    )
    .bind(id)
    .bind(run_id)
    .bind(candidate_json)
    .bind(updated_at)
    .execute(pool)
    .await?;
    Ok(())
}

async fn insert_plan_match_postgres(
    pool: &PgPool,
    id: &str,
    run_id: &str,
    match_json: &str,
    updated_at: &str,
) -> Result<(), RunnerError> {
    sqlx::query(
        "INSERT INTO plan_matches (id, run_id, match_json, updated_at)
         VALUES ($1, $2, $3, $4)
         ON CONFLICT(id) DO UPDATE SET
            run_id = excluded.run_id,
            match_json = excluded.match_json,
            updated_at = excluded.updated_at",
    )
    .bind(id)
    .bind(run_id)
    .bind(match_json)
    .bind(updated_at)
    .execute(pool)
    .await?;
    Ok(())
}

async fn insert_obligation_postgres(
    pool: &PgPool,
    id: &str,
    run_id: &str,
    obligation_json: &str,
    updated_at: &str,
) -> Result<(), RunnerError> {
    sqlx::query(
        "INSERT INTO obligations (id, run_id, obligation_json, updated_at)
         VALUES ($1, $2, $3, $4)
         ON CONFLICT(id) DO UPDATE SET
            run_id = excluded.run_id,
            obligation_json = excluded.obligation_json,
            updated_at = excluded.updated_at",
    )
    .bind(id)
    .bind(run_id)
    .bind(obligation_json)
    .bind(updated_at)
    .execute(pool)
    .await?;
    Ok(())
}

async fn insert_validator_run_postgres(
    pool: &PgPool,
    id: &str,
    run_id: &str,
    validator_json: &str,
    updated_at: &str,
) -> Result<(), RunnerError> {
    sqlx::query(
        "INSERT INTO validator_runs (id, run_id, validator_json, updated_at)
         VALUES ($1, $2, $3, $4)
         ON CONFLICT(id) DO UPDATE SET
            run_id = excluded.run_id,
            validator_json = excluded.validator_json,
            updated_at = excluded.updated_at",
    )
    .bind(id)
    .bind(run_id)
    .bind(validator_json)
    .bind(updated_at)
    .execute(pool)
    .await?;
    Ok(())
}

fn scoped_artifact_id(scope: &str, local_id: &str) -> String {
    format!("{scope}:{local_id}")
}

fn next_planning_run_id(state: &ProjectState) -> String {
    format!(
        "run-{}-{}",
        now_unix_secs(),
        state.planning_runs.len().saturating_add(1)
    )
}

fn postgres_hint(url: &str, err: &sqlx::Error) -> String {
    let raw = err.to_string();
    if url == "postgres:///tzu?host=/run/postgresql"
        && raw.contains("database \"tzu\" does not exist")
    {
        format!(
            "{raw}; create it with `createdb -h /run/postgresql tzu` and validate with `psql 'postgres:///tzu?host=/run/postgresql' -c 'select 1'`"
        )
    } else {
        raw
    }
}

fn now_unix_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn planner_state_persists_in_sqlite() {
        let temp = tempfile::tempdir().unwrap();
        let db = temp.path().join("state.sqlite");
        let url = format!("sqlite://{}", db.display());
        let runner = TzuRunner::connect(temp.path(), &url).await.unwrap();

        runner.init().await.unwrap();
        runner.plan("add health endpoint").await.unwrap();

        let reopened = TzuRunner::connect(temp.path(), &url).await.unwrap();
        let state = reopened.status().await.unwrap();
        let plan = state.current_plan.unwrap();
        assert_eq!(plan.goal, "add health endpoint");
        assert!(!plan.tasks.is_empty());
    }

    #[tokio::test]
    async fn rejected_goal_prompt_does_not_mutate_state_or_artifacts() {
        let temp = tempfile::tempdir().unwrap();
        let db = temp.path().join("state.sqlite");
        let url = format!("sqlite://{}", db.display());
        let runner = TzuRunner::connect(temp.path(), &url).await.unwrap();

        runner.init().await.unwrap();
        let error = runner
            .plan_with_context(
                "TODO",
                PlanningDomain::Coding,
                vec![temp.path().to_path_buf()],
                false,
            )
            .await
            .unwrap_err();

        assert!(matches!(
            error,
            RunnerError::Planning(tzu_core::PlanError::PromptNeedsImprovement(_))
        ));
        let state = runner.status().await.unwrap();
        assert!(state.current_plan.is_none());
        assert!(state.planning_runs.is_empty());

        let pool = SqlitePoolOptions::new().connect(&url).await.unwrap();
        for table in [
            "planning_runs",
            "plan_candidates",
            "plan_matches",
            "context_snapshots",
        ] {
            let query = format!("SELECT COUNT(*) AS count FROM {table}");
            let count: i64 = sqlx::query(&query)
                .fetch_one(&pool)
                .await
                .unwrap()
                .get("count");
            assert_eq!(count, 0, "{table} should stay empty");
        }
    }

    #[tokio::test]
    async fn harness_candidates_persist_in_sqlite_side_tables() {
        let temp = tempfile::tempdir().unwrap();
        let db = temp.path().join("state.sqlite");
        let url = format!("sqlite://{}", db.display());
        let runner = TzuRunner::connect(temp.path(), &url).await.unwrap();

        let planned = runner.plan("add health endpoint").await.unwrap();
        let harness = planned
            .current_plan
            .as_ref()
            .unwrap()
            .harness
            .as_ref()
            .unwrap();
        let run = planned.planning_runs.last().unwrap().clone();
        let expected_candidate_count = harness.candidates.len() as i64;

        let reopened = TzuRunner::connect(temp.path(), &url).await.unwrap();
        let state = reopened.status().await.unwrap();
        assert_eq!(state.planning_runs.len(), 1);
        assert!(state.current_plan.is_some());

        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect(&url)
            .await
            .unwrap();
        let planning_run_count: i64 =
            sqlx::query("SELECT COUNT(*) AS count FROM planning_runs WHERE id = ?")
                .bind(&run.id)
                .fetch_one(&pool)
                .await
                .unwrap()
                .get("count");
        assert_eq!(planning_run_count, 1);

        let candidate_count: i64 =
            sqlx::query("SELECT COUNT(*) AS count FROM plan_candidates WHERE run_id = ?")
                .bind(&run.id)
                .fetch_one(&pool)
                .await
                .unwrap()
                .get("count");
        assert_eq!(candidate_count, expected_candidate_count);

        let validator_count: i64 =
            sqlx::query("SELECT COUNT(*) AS count FROM validator_runs WHERE run_id = ?")
                .bind(&run.id)
                .fetch_one(&pool)
                .await
                .unwrap()
                .get("count");
        assert_eq!(validator_count, expected_candidate_count);

        let row = sqlx::query(
            "SELECT id, candidate_json FROM plan_candidates WHERE run_id = ? ORDER BY id LIMIT 1",
        )
        .bind(&run.id)
        .fetch_one(&pool)
        .await
        .unwrap();
        let candidate_row_id: String = row.get("id");
        assert!(candidate_row_id.starts_with(&format!("{}:", run.id)));
        let candidate_json: String = row.get("candidate_json");
        let candidate: serde_json::Value = serde_json::from_str(&candidate_json).unwrap();
        assert!(candidate.get("score").is_some());
        assert!(candidate.get("descriptor").is_some());
        assert!(candidate["score"].get("verifier_strength").is_some());
        assert!(candidate["descriptor"].get("verifier_dependency").is_some());

        let row = sqlx::query(
            "SELECT id, validator_json FROM validator_runs WHERE run_id = ? ORDER BY id LIMIT 1",
        )
        .bind(&run.id)
        .fetch_one(&pool)
        .await
        .unwrap();
        let validator_row_id: String = row.get("id");
        assert!(validator_row_id.starts_with(&format!("{}:", run.id)));
        assert!(validator_row_id.ends_with(":validator-static"));
        let validator_json: String = row.get("validator_json");
        let outcome: tzu_core::ValidatorOutcome = serde_json::from_str(&validator_json).unwrap();
        assert_eq!(outcome.run_id, run.id);
        assert_eq!(outcome.candidate_id, "candidate-1");
        assert_eq!(outcome.tier, tzu_core::ValidationTier::Static);
        assert_eq!(outcome.status, tzu_core::ValidationOutcomeStatus::Passed);
        assert_eq!(outcome.reward, tzu_core::ValidationRewardBucket::Partial);
        assert!(!outcome.candidate_hash.is_empty());
    }

    #[tokio::test]
    async fn coding_plan_persists_multi_root_context_snapshot() {
        let state_root = tempfile::tempdir().unwrap();
        let first = tempfile::tempdir().unwrap();
        let second = tempfile::tempdir().unwrap();
        std::fs::write(first.path().join("Cargo.toml"), "[package]\nname='one'\n").unwrap();
        std::fs::write(first.path().join("README.md"), "# One\n").unwrap();
        std::fs::write(second.path().join("flake.nix"), "{ outputs = _: {}; }\n").unwrap();

        let url = format!(
            "sqlite://{}",
            state_root.path().join("state.sqlite").display()
        );
        let runner = TzuRunner::connect(state_root.path(), &url).await.unwrap();
        let planned = runner
            .plan_with_context(
                "add project context",
                PlanningDomain::Coding,
                vec![first.path().to_path_buf(), second.path().to_path_buf()],
                false,
            )
            .await
            .unwrap();

        let plan = planned.current_plan.as_ref().unwrap();
        let harness = plan.harness.as_ref().unwrap();
        assert!(
            harness
                .problem_spec
                .evidence
                .iter()
                .any(|evidence| evidence.source == "project-context:context-root-1")
        );
        assert!(
            harness
                .problem_spec
                .evidence
                .iter()
                .any(|evidence| evidence.source == "project-context:context-root-2")
        );
        assert!(
            harness
                .problem_spec
                .evidence
                .iter()
                .any(|evidence| evidence.summary.contains("Cargo.toml"))
        );
        assert!(
            harness
                .problem_spec
                .evidence
                .iter()
                .any(|evidence| evidence.summary.contains("flake.nix"))
        );

        let run = planned.planning_runs.last().unwrap();
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect(&url)
            .await
            .unwrap();
        let row = sqlx::query(
            "SELECT snapshot_json FROM context_snapshots WHERE run_id = ? ORDER BY id LIMIT 1",
        )
        .bind(&run.id)
        .fetch_one(&pool)
        .await
        .unwrap();
        let snapshot_json: String = row.get("snapshot_json");
        let snapshot: tzu_repo::ProjectContextSnapshot =
            serde_json::from_str(&snapshot_json).unwrap();
        assert_eq!(snapshot.roots.len(), 2);
        assert_eq!(snapshot.roots[0].files.len(), 2);
        assert_eq!(snapshot.roots[1].files.len(), 1);
    }

    #[tokio::test]
    async fn planning_goal_can_differ_from_display_goal() {
        let temp = tempfile::tempdir().unwrap();
        std::fs::write(
            temp.path().join("Cargo.toml"),
            "[package]\nname='fixture'\n",
        )
        .unwrap();
        let url = format!("sqlite://{}", temp.path().join("state.sqlite").display());
        let runner = TzuRunner::connect(temp.path(), &url).await.unwrap();

        let planned = runner
            .actor()
            .create_plan(CreatePlan {
                goal: "update @fixture".to_string(),
                planning_goal: Some(format!("update @{}", temp.path().display())),
                domain: PlanningDomain::Coding,
                context_roots: vec![temp.path().to_path_buf()],
                include_nested_contexts: false,
            })
            .await
            .unwrap();

        let plan = planned.current_plan.as_ref().unwrap();
        assert_eq!(plan.goal, "update @fixture");
        let harness = plan.harness.as_ref().unwrap();
        assert_eq!(
            harness.problem_spec.goal,
            format!("update @{}", temp.path().display())
        );
    }

    #[tokio::test]
    async fn mocked_run_persists_structured_report() {
        let temp = tempfile::tempdir().unwrap();
        std::fs::write(
            temp.path().join("Cargo.toml"),
            "[package]\nname='fixture'\n",
        )
        .unwrap();
        let output = std::process::Command::new("git")
            .args(["init", "-b", "main"])
            .current_dir(temp.path())
            .output()
            .unwrap();
        assert!(output.status.success());

        let url = format!("sqlite://{}", temp.path().join("state.sqlite").display());
        let runner = TzuRunner::connect(temp.path(), &url).await.unwrap();
        runner
            .plan_with_domain("add health endpoint", PlanningDomain::Coding)
            .await
            .unwrap();
        let report = runner
            .run_task("inspect-repo", RunMode::Mock)
            .await
            .unwrap();

        assert_eq!(report.status, TaskStatus::Completed);
        assert!(report.summary.contains("Mocked ACP run completed"));
        let state = runner.status().await.unwrap();
        assert_eq!(state.run_reports.len(), 1);
    }

    #[tokio::test]
    async fn optional_postgres_migration_smoke() {
        let Some(url) = std::env::var("TZU_TEST_DATABASE_URL").ok() else {
            return;
        };
        let temp = tempfile::tempdir().unwrap();
        let runner = TzuRunner::connect(temp.path(), &url).await.unwrap();
        let state = runner.plan("add health endpoint").await.unwrap();
        assert!(state.current_plan.is_some());
        assert!(!state.planning_runs.is_empty());
    }
}
