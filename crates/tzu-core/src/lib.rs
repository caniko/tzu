use std::cmp::Reverse;
use std::collections::{BTreeMap, BTreeSet};

use async_trait::async_trait;
use petgraph::Direction;
use petgraph::algo::toposort;
use petgraph::graph::{DiGraph, NodeIndex};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct AcceptanceCriterion {
    pub description: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum DomainKind {
    Generic,
    Coding,
}

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "kebab-case")]
pub enum Risk {
    Low,
    Medium,
    High,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum TaskStatus {
    Pending,
    Running,
    Completed,
    Failed,
    Blocked,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct Task {
    pub id: String,
    pub title: String,
    pub description: String,
    pub status: TaskStatus,
    pub risk: Risk,
    pub acceptance_criteria: Vec<AcceptanceCriterion>,
    pub depends_on: Vec<String>,
}

impl Task {
    #[must_use]
    pub fn new(
        id: impl Into<String>,
        title: impl Into<String>,
        description: impl Into<String>,
        risk: Risk,
        acceptance_criteria: Vec<AcceptanceCriterion>,
        depends_on: Vec<String>,
    ) -> Self {
        Self {
            id: id.into(),
            title: title.into(),
            description: description.into(),
            status: TaskStatus::Pending,
            risk,
            acceptance_criteria,
            depends_on,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Plan {
    pub id: String,
    pub goal: String,
    pub tasks: Vec<Task>,
    #[serde(default = "default_domain_kind")]
    pub domain: DomainKind,
    #[serde(default)]
    pub harness: Option<HarnessPlanMetadata>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct RunReport {
    pub task_id: String,
    pub status: TaskStatus,
    pub summary: String,
    pub changed_files: Vec<String>,
    pub events: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ProjectState {
    pub project_root: String,
    pub current_plan: Option<Plan>,
    #[serde(default)]
    pub planning_runs: Vec<PlanningRun>,
    pub run_reports: Vec<RunReport>,
}

impl ProjectState {
    #[must_use]
    pub fn new(project_root: impl Into<String>) -> Self {
        Self {
            project_root: project_root.into(),
            current_plan: None,
            planning_runs: Vec::new(),
            run_reports: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct EvidenceRef {
    pub source: String,
    pub summary: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ProblemSpec {
    pub id: String,
    pub goal: String,
    pub domain: DomainKind,
    pub project_root: Option<String>,
    pub constraints: Vec<String>,
    pub acceptance_criteria: Vec<AcceptanceCriterion>,
    pub evidence: Vec<EvidenceRef>,
    pub immutable_hash: String,
}

impl ProblemSpec {
    #[must_use]
    pub fn new(
        goal: impl Into<String>,
        domain: DomainKind,
        project_root: Option<String>,
        constraints: Vec<String>,
        acceptance_criteria: Vec<AcceptanceCriterion>,
        evidence: Vec<EvidenceRef>,
    ) -> Self {
        let goal = goal.into();
        let id = stable_plan_id(&goal).replacen("plan-", "spec-", 1);
        let mut spec = Self {
            id,
            goal,
            domain,
            project_root,
            constraints,
            acceptance_criteria,
            evidence,
            immutable_hash: String::new(),
        };
        spec.immutable_hash = spec.compute_hash();
        spec
    }

    #[must_use]
    pub fn compute_hash(&self) -> String {
        let mut clone = self.clone();
        clone.immutable_hash.clear();
        stable_hash_json(&clone)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct Obligation {
    pub id: String,
    pub description: String,
    pub producer: String,
    pub regenerate_command: String,
    pub validation_command: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ValidationFinding {
    pub code: String,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ValidationResult {
    pub spec_hash_ok: bool,
    pub hard_failures: Vec<ValidationFinding>,
    pub soft_findings: Vec<ValidationFinding>,
    pub obligations: Vec<Obligation>,
    pub evidence_refs: Vec<EvidenceRef>,
}

impl ValidationResult {
    #[must_use]
    pub fn valid(evidence_refs: Vec<EvidenceRef>) -> Self {
        Self {
            spec_hash_ok: true,
            hard_failures: Vec::new(),
            soft_findings: Vec::new(),
            obligations: Vec::new(),
            evidence_refs,
        }
    }

    #[must_use]
    pub fn is_valid(&self) -> bool {
        self.spec_hash_ok && self.hard_failures.is_empty()
    }

    fn hard_failure(mut self, code: impl Into<String>, message: impl Into<String>) -> Self {
        self.hard_failures.push(ValidationFinding {
            code: code.into(),
            message: message.into(),
        });
        self
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct PlanCandidate {
    pub summary: String,
    pub tasks: Vec<Task>,
    pub assumptions: Vec<String>,
    pub risks: Vec<String>,
    pub verification: Vec<String>,
    pub rollout: Vec<String>,
    pub blockers: Vec<Obligation>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum SketchStatus {
    Seeded,
    Valid,
    Invalid,
    Selected,
}

#[derive(
    Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "kebab-case")]
pub enum VerifierStrength {
    #[default]
    Weak,
    Moderate,
    Strong,
}

#[derive(
    Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "kebab-case")]
pub enum ObligationBurden {
    None,
    One,
    #[default]
    Many,
}

#[derive(
    Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "kebab-case")]
pub enum CostTier {
    Low,
    Medium,
    #[default]
    High,
}

#[derive(
    Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "kebab-case")]
pub enum VerifierDependency {
    #[default]
    Static,
    Repository,
    Agent,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct CandidateScore {
    pub verifier_strength: VerifierStrength,
    pub obligation_burden: ObligationBurden,
    pub risk_profile: Risk,
    pub cost_tier: CostTier,
    pub task_graph_quality: u8,
    pub execution_readiness: u8,
}

impl Default for CandidateScore {
    fn default() -> Self {
        Self {
            verifier_strength: VerifierStrength::Weak,
            obligation_burden: ObligationBurden::Many,
            risk_profile: Risk::High,
            cost_tier: CostTier::High,
            task_graph_quality: 0,
            execution_readiness: 0,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct CandidateDescriptor {
    pub cost_tier: CostTier,
    pub risk_profile: Risk,
    pub verifier_dependency: VerifierDependency,
}

impl Default for CandidateDescriptor {
    fn default() -> Self {
        Self {
            cost_tier: CostTier::High,
            risk_profile: Risk::High,
            verifier_dependency: VerifierDependency::Static,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct PlanSketch {
    pub id: String,
    pub problem_id: String,
    pub parent_ids: Vec<String>,
    pub candidate: PlanCandidate,
    pub status: SketchStatus,
    pub validation: ValidationResult,
    #[serde(default)]
    pub score: CandidateScore,
    #[serde(default)]
    pub descriptor: CandidateDescriptor,
    pub created_by: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct MatchResult {
    pub candidate_ids: Vec<String>,
    pub ranking: Vec<String>,
    pub rationale: String,
    pub rater_backend: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct FrontierPolicy {
    pub min_elite: usize,
    pub max_elite: usize,
    pub retain_descriptor_cells: bool,
}

impl Default for FrontierPolicy {
    fn default() -> Self {
        Self {
            min_elite: 3,
            max_elite: 8,
            retain_descriptor_cells: true,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum FrontierDiscardReason {
    Invalid,
    Dominated,
    Capacity,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct FrontierDiscard {
    pub candidate_id: String,
    pub reason: FrontierDiscardReason,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct FrontierMetadata {
    pub policy: FrontierPolicy,
    pub retained_candidate_ids: Vec<String>,
    pub discarded_candidates: Vec<FrontierDiscard>,
    pub selected_candidate_id: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct HarnessPlanMetadata {
    pub problem_spec: ProblemSpec,
    pub selected_candidate_id: String,
    pub candidates: Vec<PlanSketch>,
    pub matches: Vec<MatchResult>,
    #[serde(default)]
    pub frontier: FrontierMetadata,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct PlanningRun {
    pub id: String,
    pub domain: DomainKind,
    pub problem_spec: ProblemSpec,
    pub selected_candidate_id: String,
    pub candidate_count: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum ValidationTier {
    Static,
    Repository,
    AcpPlanning,
    ExpensiveVerifier,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum ValidationOutcomeStatus {
    Passed,
    Failed,
    Blocked,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum ValidationRewardBucket {
    Zero,
    Partial,
    Full,
}

impl ValidationRewardBucket {
    #[must_use]
    pub const fn as_f64(self) -> f64 {
        match self {
            Self::Zero => 0.0,
            Self::Partial => 0.5,
            Self::Full => 1.0,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ValidatorOutcome {
    pub candidate_id: String,
    pub candidate_hash: String,
    pub run_id: String,
    pub tier: ValidationTier,
    pub status: ValidationOutcomeStatus,
    pub reward: ValidationRewardBucket,
    pub obligations_discharged: usize,
    pub evidence_refs_added: usize,
    pub generated_at_unix_secs: u64,
}

#[derive(Debug, Error)]
pub enum PlanError {
    #[error("task `{0}` has no acceptance criteria")]
    MissingAcceptanceCriteria(String),
    #[error("duplicate task id `{0}`")]
    DuplicateTaskId(String),
    #[error("task `{task_id}` depends on missing task `{dependency}`")]
    MissingDependency { task_id: String, dependency: String },
    #[error("task DAG contains a cycle")]
    Cycle,
    #[error("candidate `{0}` is invalid")]
    InvalidCandidate(String),
}

#[async_trait]
pub trait Planner: Send + Sync {
    async fn create_plan(&self, goal: &str) -> Result<Plan, PlanError>;
}

#[derive(Debug, Clone, Default)]
pub struct DeterministicPlanner;

#[async_trait]
impl Planner for DeterministicPlanner {
    async fn create_plan(&self, goal: &str) -> Result<Plan, PlanError> {
        let normalized = goal.trim();
        let plan = Plan {
            id: stable_plan_id(normalized),
            goal: normalized.to_string(),
            tasks: vec![
                Task::new(
                    "inspect-repo",
                    "Inspect repository state",
                    "Load repository metadata, file tree, and language summary before making changes.",
                    Risk::Low,
                    vec![AcceptanceCriterion {
                        description: "Repository state is indexed and attached to the run context."
                            .to_string(),
                    }],
                    Vec::new(),
                ),
                Task::new(
                    "implement-goal",
                    "Implement requested goal",
                    format!(
                        "Use ACP-backed Codex execution for semantic coding work: {normalized}"
                    ),
                    Risk::Medium,
                    vec![AcceptanceCriterion {
                        description: format!(
                            "The codebase implements the requested goal: {normalized}"
                        ),
                    }],
                    vec!["inspect-repo".to_string()],
                ),
                Task::new(
                    "verify-goal",
                    "Verify implementation",
                    "Run focused checks, inspect changed files, and produce a structured run report.",
                    Risk::Low,
                    vec![AcceptanceCriterion {
                        description:
                            "Verification results and changed files are captured in a run report."
                                .to_string(),
                    }],
                    vec!["implement-goal".to_string()],
                ),
            ],
            domain: DomainKind::Coding,
            harness: None,
        };
        validate_plan(&plan)?;
        Ok(plan)
    }
}

#[derive(Debug, Clone)]
pub struct HarnessPlanner<A> {
    adapter: A,
}

impl<A> HarnessPlanner<A> {
    #[must_use]
    pub fn new(adapter: A) -> Self {
        Self { adapter }
    }
}

#[async_trait]
impl<A> Planner for HarnessPlanner<A>
where
    A: DomainAdapter + Send + Sync,
{
    async fn create_plan(&self, goal: &str) -> Result<Plan, PlanError> {
        self.create_harness_plan(goal).await
    }
}

impl<A> HarnessPlanner<A>
where
    A: DomainAdapter,
{
    pub async fn create_harness_plan(&self, goal: &str) -> Result<Plan, PlanError> {
        let spec = self.adapter.build_spec(goal);
        let mut candidates = self
            .adapter
            .seed_candidates(&spec)
            .into_iter()
            .enumerate()
            .map(|(idx, candidate)| {
                let validation = self.adapter.validate_candidate(&spec, &candidate);
                let status = if validation.is_valid() {
                    SketchStatus::Valid
                } else {
                    SketchStatus::Invalid
                };
                PlanSketch {
                    id: format!("candidate-{}", idx + 1),
                    problem_id: spec.id.clone(),
                    parent_ids: Vec::new(),
                    candidate,
                    status,
                    validation,
                    score: CandidateScore::default(),
                    descriptor: CandidateDescriptor::default(),
                    created_by: "seed".to_string(),
                }
            })
            .collect::<Vec<_>>();

        score_candidates(&mut candidates);
        let frontier = select_candidate_frontier(&mut candidates, FrontierPolicy::default())?;
        let selected = candidates
            .iter()
            .find(|candidate| candidate.id == frontier.selected_candidate_id)
            .cloned()
            .ok_or_else(|| PlanError::InvalidCandidate(frontier.selected_candidate_id.clone()))?;
        let selected_candidate = selected.candidate.clone();
        let plan = Plan {
            id: stable_plan_id(&spec.goal),
            goal: spec.goal.clone(),
            tasks: selected_candidate.tasks,
            domain: spec.domain,
            harness: Some(HarnessPlanMetadata {
                problem_spec: spec,
                selected_candidate_id: selected.id,
                candidates,
                matches: Vec::new(),
                frontier,
            }),
        };
        validate_plan(&plan)?;
        Ok(plan)
    }
}

pub trait DomainAdapter {
    fn domain(&self) -> DomainKind;
    fn build_spec(&self, goal: &str) -> ProblemSpec;
    fn seed_candidates(&self, spec: &ProblemSpec) -> Vec<PlanCandidate>;
    fn validate_candidate(&self, spec: &ProblemSpec, candidate: &PlanCandidate)
    -> ValidationResult;
}

#[derive(Debug, Clone, Default)]
pub struct GenericDomainAdapter;

impl DomainAdapter for GenericDomainAdapter {
    fn domain(&self) -> DomainKind {
        DomainKind::Generic
    }

    fn build_spec(&self, goal: &str) -> ProblemSpec {
        let normalized = goal.trim();
        ProblemSpec::new(
            normalized,
            self.domain(),
            None,
            vec![
                "Do not fabricate or silently substitute missing required data.".to_string(),
                "Represent foundational unknowns as explicit blockers.".to_string(),
            ],
            vec![AcceptanceCriterion {
                description: format!("A validated plan exists for: {normalized}"),
            }],
            vec![EvidenceRef {
                source: "user-goal".to_string(),
                summary: normalized.to_string(),
            }],
        )
    }

    fn seed_candidates(&self, spec: &ProblemSpec) -> Vec<PlanCandidate> {
        vec![
            generic_conservative_serial_candidate(spec),
            generic_evidence_first_candidate(spec),
            generic_parallel_exploration_candidate(spec),
            generic_blocker_reduction_candidate(spec),
        ]
    }

    fn validate_candidate(
        &self,
        spec: &ProblemSpec,
        candidate: &PlanCandidate,
    ) -> ValidationResult {
        validate_candidate_common(spec, candidate)
    }
}

fn generic_conservative_serial_candidate(spec: &ProblemSpec) -> PlanCandidate {
    PlanCandidate {
        summary: format!("Conservative verifier-grounded plan for {}", spec.goal),
        tasks: vec![
            Task::new(
                "ground-inputs",
                "Ground inputs and constraints",
                "Collect required artifacts, evidence, constraints, and missing-input blockers before choosing an approach.",
                Risk::Low,
                vec![AcceptanceCriterion {
                    description:
                        "All required inputs are either cited or represented as explicit blockers."
                            .to_string(),
                }],
                Vec::new(),
            ),
            Task::new(
                "select-minimal-plan",
                "Select minimal plan",
                "Choose the smallest valid plan that satisfies the stated constraints and records rejected alternatives.",
                Risk::Low,
                vec![AcceptanceCriterion {
                    description:
                        "The selected plan is valid, minimal, and includes rationale for rejected alternatives."
                            .to_string(),
                }],
                vec!["ground-inputs".to_string()],
            ),
        ],
        assumptions: vec![
            "No domain-specific adapter was selected; use generic planning validators.".to_string(),
            "Prefer the smallest valid plan until evidence requires broader exploration.".to_string(),
        ],
        risks: vec!["The minimal plan may miss a higher-upside alternative.".to_string()],
        verification: vec![
            "Validate task DAG, acceptance criteria, explicit blockers, and evidence references."
                .to_string(),
        ],
        rollout: vec!["Persist selected plan and candidate rationale.".to_string()],
        blockers: Vec::new(),
    }
}

fn generic_evidence_first_candidate(spec: &ProblemSpec) -> PlanCandidate {
    PlanCandidate {
        summary: format!("Evidence-first plan for {}", spec.goal),
        tasks: vec![
            Task::new(
                "inventory-evidence",
                "Inventory evidence",
                "Collect user-provided sources, constraints, prior plans, and known missing inputs before comparing approaches.",
                Risk::Low,
                vec![AcceptanceCriterion {
                    description:
                        "Evidence sources and missing inputs are listed with provenance."
                            .to_string(),
                }],
                Vec::new(),
            ),
            Task::new(
                "cross-check-constraints",
                "Cross-check constraints",
                "Validate evidence against constraints and mark contradictions as explicit blockers.",
                Risk::Medium,
                vec![AcceptanceCriterion {
                    description:
                        "Contradictions and unresolved assumptions are represented as findings or blockers."
                            .to_string(),
                }],
                vec!["inventory-evidence".to_string()],
            ),
            Task::new(
                "choose-evidence-backed-plan",
                "Choose evidence-backed plan",
                "Select the strongest plan supported by the checked evidence and preserve tradeoff rationale.",
                Risk::Low,
                vec![AcceptanceCriterion {
                    description:
                        "The plan cites its evidence and records the tradeoffs that determined selection."
                            .to_string(),
                }],
                vec!["cross-check-constraints".to_string()],
            ),
        ],
        assumptions: vec![
            "The highest-quality plan depends on careful evidence grounding before selection."
                .to_string(),
        ],
        risks: vec![
            "Evidence gathering may cost more time than a minimal serial plan.".to_string(),
            "Contradictory inputs may block selection until the upstream source is fixed.".to_string(),
        ],
        verification: vec![
            "Validate task DAG, acceptance criteria, explicit blockers, and evidence references."
                .to_string(),
            "Review evidence provenance before accepting the selected plan.".to_string(),
        ],
        rollout: vec![
            "Persist selected plan, evidence inventory, and rejected alternative rationale."
                .to_string(),
        ],
        blockers: Vec::new(),
    }
}

fn generic_parallel_exploration_candidate(spec: &ProblemSpec) -> PlanCandidate {
    PlanCandidate {
        summary: format!("Parallel exploration plan for {}", spec.goal),
        tasks: vec![
            Task::new(
                "ground-shared-context",
                "Ground shared context",
                "Collect baseline constraints and evidence before opening parallel search branches.",
                Risk::Low,
                vec![AcceptanceCriterion {
                    description:
                        "Shared context is stable enough for independent branch comparison."
                            .to_string(),
                }],
                Vec::new(),
            ),
            Task::new(
                "explore-safe-option",
                "Explore safe option",
                "Generate a low-risk branch and define repository checks needed to validate it.",
                Risk::Medium,
                vec![AcceptanceCriterion {
                    description: "The safe branch has explicit repository check criteria."
                        .to_string(),
                }],
                vec!["ground-shared-context".to_string()],
            ),
            Task::new(
                "explore-ambitious-option",
                "Explore ambitious option",
                "Generate a higher-upside branch and identify repository checks that would falsify it.",
                Risk::Medium,
                vec![AcceptanceCriterion {
                    description: "The ambitious branch has falsification criteria.".to_string(),
                }],
                vec!["ground-shared-context".to_string()],
            ),
            Task::new(
                "compare-branches",
                "Compare branches",
                "Compare the validated branches, preserve rejected rationale, and choose the strongest candidate.",
                Risk::Medium,
                vec![AcceptanceCriterion {
                    description:
                        "Branch comparison records evidence, tradeoffs, and selected direction."
                            .to_string(),
                }],
                vec![
                    "explore-safe-option".to_string(),
                    "explore-ambitious-option".to_string(),
                ],
            ),
        ],
        assumptions: vec![
            "Parallel exploration is useful when the local optimum risk is higher than the cost of extra branches."
                .to_string(),
        ],
        risks: vec![
            "Parallel branches may duplicate work without clear comparison criteria.".to_string(),
            "Repository checks may be needed before branch comparison is meaningful.".to_string(),
        ],
        verification: vec![
            "Validate task DAG, acceptance criteria, explicit blockers, and evidence references."
                .to_string(),
            "Run repository checks or build checks named by the selected branch before accepting it."
                .to_string(),
        ],
        rollout: vec![
            "Persist selected branch and rejected branch rationale after repository validation."
                .to_string(),
        ],
        blockers: Vec::new(),
    }
}

fn generic_blocker_reduction_candidate(spec: &ProblemSpec) -> PlanCandidate {
    PlanCandidate {
        summary: format!("Blocker-reduction plan for {}", spec.goal),
        tasks: vec![
            Task::new(
                "classify-foundational-inputs",
                "Classify foundational inputs",
                "Identify artifacts, sources, and agent-produced evidence required before execution can proceed.",
                Risk::Medium,
                vec![AcceptanceCriterion {
                    description:
                        "Each foundational input is classified as present, missing, invalid, or blocked."
                            .to_string(),
                }],
                Vec::new(),
            ),
            Task::new(
                "regenerate-missing-inputs",
                "Regenerate missing inputs",
                "For every missing artifact, name the upstream producer, regeneration workflow, and validation command.",
                Risk::High,
                vec![AcceptanceCriterion {
                    description:
                        "Missing artifacts have concrete regeneration and validation workflows."
                            .to_string(),
                }],
                vec!["classify-foundational-inputs".to_string()],
            ),
            Task::new(
                "agent-review-blockers",
                "Agent-review blockers",
                "Use an agent-backed review only after blocker workflows are explicit and bounded.",
                Risk::Medium,
                vec![AcceptanceCriterion {
                    description:
                        "Agent review confirms blocker workflows are actionable without inventing data."
                            .to_string(),
                }],
                vec!["regenerate-missing-inputs".to_string()],
            ),
            Task::new(
                "select-unblocked-plan",
                "Select unblocked plan",
                "Choose the strongest plan whose blockers are discharged or explicitly carried forward.",
                Risk::Medium,
                vec![AcceptanceCriterion {
                    description:
                        "The selected plan has no hidden foundational blockers.".to_string(),
                }],
                vec!["agent-review-blockers".to_string()],
            ),
        ],
        assumptions: vec![
            "Unknown or invalid foundational inputs are the main risk to successful planning."
                .to_string(),
        ],
        risks: vec![
            "Blocker reduction may delay implementation when inputs are already sufficient."
                .to_string(),
            "Agent review must not fabricate missing artifacts.".to_string(),
        ],
        verification: vec![
            "Validate task DAG, acceptance criteria, explicit blockers, and evidence references."
                .to_string(),
            "Confirm every blocker has an upstream producer, regeneration command, and validation command."
                .to_string(),
        ],
        rollout: vec![
            "Persist discharged blockers and carry unresolved blockers as explicit obligations."
                .to_string(),
        ],
        blockers: Vec::new(),
    }
}

#[derive(Debug, Clone)]
pub struct CodingDomainAdapter {
    pub project_root: String,
    pub repo_dirty: bool,
    pub repo_head: Option<String>,
    pub file_count: usize,
}

impl DomainAdapter for CodingDomainAdapter {
    fn domain(&self) -> DomainKind {
        DomainKind::Coding
    }

    fn build_spec(&self, goal: &str) -> ProblemSpec {
        let normalized = goal.trim();
        let mut evidence = vec![EvidenceRef {
            source: "user-goal".to_string(),
            summary: normalized.to_string(),
        }];
        evidence.push(EvidenceRef {
            source: "repo-inspection".to_string(),
            summary: format!(
                "head={}, dirty={}, files={}",
                self.repo_head.as_deref().unwrap_or("unknown"),
                self.repo_dirty,
                self.file_count
            ),
        });
        ProblemSpec::new(
            normalized,
            self.domain(),
            Some(self.project_root.clone()),
            vec![
                "Use codex-acp for semantic coding agent work.".to_string(),
                "Do not overwrite unrelated user work.".to_string(),
                "Project planning, persistence, policy, and validation stay local to tzu."
                    .to_string(),
            ],
            vec![AcceptanceCriterion {
                description: format!("The codebase implements the requested goal: {normalized}"),
            }],
            evidence,
        )
    }

    fn seed_candidates(&self, spec: &ProblemSpec) -> Vec<PlanCandidate> {
        vec![PlanCandidate {
            summary: format!("Verifier-grounded coding plan for {}", spec.goal),
            tasks: vec![
                Task::new(
                    "inspect-repo",
                    "Inspect repository state",
                    "Load repository metadata, file tree, language summary, current plan state, and dirty-worktree context before changing source.",
                    Risk::Low,
                    vec![AcceptanceCriterion {
                        description:
                            "Repository state and relevant evidence are attached to the run context."
                                .to_string(),
                    }],
                    Vec::new(),
                ),
                Task::new(
                    "search-implementation-plan",
                    "Search implementation plans",
                    format!(
                        "Use ACP-backed Codex planning episodes to generate and validate candidate implementation strategies for: {}",
                        spec.goal
                    ),
                    Risk::Medium,
                    vec![AcceptanceCriterion {
                        description:
                            "Multiple candidate approaches are validated against constraints and ranked."
                                .to_string(),
                    }],
                    vec!["inspect-repo".to_string()],
                ),
                Task::new(
                    "implement-goal",
                    "Implement selected plan",
                    "Execute the selected validated coding plan through bounded ACP-backed work sessions.",
                    Risk::Medium,
                    vec![AcceptanceCriterion {
                        description: format!("Implementation satisfies the requested goal: {}", spec.goal),
                    }],
                    vec!["search-implementation-plan".to_string()],
                ),
                Task::new(
                    "verify-goal",
                    "Verify implementation",
                    "Run focused checks, inspect changed files, and persist a structured run report with failures or residual risk.",
                    Risk::Low,
                    vec![AcceptanceCriterion {
                        description:
                            "Verification results and changed files are captured in a run report."
                                .to_string(),
                    }],
                    vec!["implement-goal".to_string()],
                ),
            ],
            assumptions: vec![
                "codex-acp remains the only v1 agent backend.".to_string(),
                "Repo-local validators decide whether an agent-produced plan is admissible."
                    .to_string(),
            ],
            risks: vec![
                "Dirty worktree requires careful isolation of unrelated user edits.".to_string(),
                "Agent output must be schema-checked before it becomes project state.".to_string(),
            ],
            verification: vec![
                "Run the repo's focused tests or build checks after implementation.".to_string(),
                "Validate changed files against the selected plan and acceptance criteria."
                    .to_string(),
            ],
            rollout: vec![
                "Persist selected plan before implementation.".to_string(),
                "Persist run reports after each execution step.".to_string(),
            ],
            blockers: Vec::new(),
        }]
    }

    fn validate_candidate(
        &self,
        spec: &ProblemSpec,
        candidate: &PlanCandidate,
    ) -> ValidationResult {
        let mut result = validate_candidate_common(spec, candidate);
        if spec.project_root.is_none() {
            result = result.hard_failure(
                "missing-project-root",
                "coding plans require a project root in the immutable problem spec",
            );
        }
        if self.file_count == 0 {
            result.obligations.push(Obligation {
                id: "repo-files".to_string(),
                description: "Repository inspection found no files.".to_string(),
                producer: "tzu-repo inspect_repo".to_string(),
                regenerate_command: "tzu status".to_string(),
                validation_command: "find . -type f | head".to_string(),
            });
        }
        result
    }
}

#[must_use]
pub fn stable_plan_id(goal: &str) -> String {
    let slug = goal
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() {
                ch.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect::<String>()
        .split('-')
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join("-");
    format!("plan-{}", if slug.is_empty() { "default" } else { &slug })
}

pub fn validate_plan(plan: &Plan) -> Result<(), PlanError> {
    let mut ids = BTreeSet::new();
    for task in &plan.tasks {
        if task.acceptance_criteria.is_empty() {
            return Err(PlanError::MissingAcceptanceCriteria(task.id.clone()));
        }
        if !ids.insert(task.id.clone()) {
            return Err(PlanError::DuplicateTaskId(task.id.clone()));
        }
    }

    for task in &plan.tasks {
        for dependency in &task.depends_on {
            if !ids.contains(dependency) {
                return Err(PlanError::MissingDependency {
                    task_id: task.id.clone(),
                    dependency: dependency.clone(),
                });
            }
        }
    }

    task_graph(plan).and_then(|graph| {
        toposort(&graph, None)
            .map(|_| ())
            .map_err(|_| PlanError::Cycle)
    })
}

#[must_use]
pub fn validate_candidate_common(
    spec: &ProblemSpec,
    candidate: &PlanCandidate,
) -> ValidationResult {
    let mut result = ValidationResult::valid(spec.evidence.clone());
    result.spec_hash_ok = spec.immutable_hash == spec.compute_hash();
    if !result.spec_hash_ok {
        result = result.hard_failure(
            "spec-hash-mismatch",
            "immutable problem spec hash does not match its contents",
        );
    }

    if candidate.summary.trim().is_empty() {
        result = result.hard_failure("missing-summary", "candidate summary is empty");
    }
    if candidate.tasks.is_empty() {
        result = result.hard_failure("missing-tasks", "candidate has no tasks");
    } else {
        let plan = Plan {
            id: "candidate-validation".to_string(),
            goal: spec.goal.clone(),
            tasks: candidate.tasks.clone(),
            domain: spec.domain,
            harness: None,
        };
        if let Err(err) = validate_plan(&plan) {
            result = result.hard_failure("invalid-task-dag", err.to_string());
        }
    }
    if candidate.verification.is_empty() {
        result = result.hard_failure(
            "missing-verification",
            "candidate must include at least one validation step",
        );
    }
    if candidate
        .tasks
        .iter()
        .any(|task| task.description.to_ascii_lowercase().contains("tbd"))
    {
        result = result.hard_failure("tbd-task", "task descriptions may not hide TBD work");
    }
    result.obligations.extend(candidate.blockers.clone());
    result
}

fn score_candidates(candidates: &mut [PlanSketch]) {
    for candidate in candidates {
        let risk_profile = derive_risk_profile(&candidate.candidate.tasks);
        let cost_tier = derive_cost_tier(&candidate.candidate.tasks);
        let obligation_burden = derive_obligation_burden(&candidate.validation);
        let verifier_strength = derive_verifier_strength(&candidate.validation);
        candidate.score = CandidateScore {
            verifier_strength,
            obligation_burden,
            risk_profile,
            cost_tier,
            task_graph_quality: derive_task_graph_quality(&candidate.validation),
            execution_readiness: derive_execution_readiness(&candidate.validation),
        };
        candidate.descriptor = CandidateDescriptor {
            cost_tier,
            risk_profile,
            verifier_dependency: derive_verifier_dependency(&candidate.candidate),
        };
    }
}

fn derive_obligation_burden(validation: &ValidationResult) -> ObligationBurden {
    match validation.obligations.len() {
        0 => ObligationBurden::None,
        1 => ObligationBurden::One,
        _ => ObligationBurden::Many,
    }
}

fn derive_risk_profile(tasks: &[Task]) -> Risk {
    if tasks.iter().any(|task| task.risk == Risk::High) {
        Risk::High
    } else if tasks.iter().any(|task| task.risk == Risk::Medium) {
        Risk::Medium
    } else {
        Risk::Low
    }
}

fn derive_cost_tier(tasks: &[Task]) -> CostTier {
    let high_risk_tasks = tasks.iter().filter(|task| task.risk == Risk::High).count();
    let medium_risk_tasks = tasks
        .iter()
        .filter(|task| task.risk == Risk::Medium)
        .count();
    if tasks.len() >= 6 || high_risk_tasks > 0 {
        CostTier::High
    } else if tasks.len() >= 3 || medium_risk_tasks > 0 {
        CostTier::Medium
    } else {
        CostTier::Low
    }
}

fn derive_verifier_strength(validation: &ValidationResult) -> VerifierStrength {
    if !validation.is_valid() {
        VerifierStrength::Weak
    } else if validation.obligations.is_empty() {
        VerifierStrength::Strong
    } else {
        VerifierStrength::Moderate
    }
}

fn derive_verifier_dependency(candidate: &PlanCandidate) -> VerifierDependency {
    let text = candidate
        .verification
        .iter()
        .chain(candidate.rollout.iter())
        .chain(candidate.tasks.iter().map(|task| &task.description))
        .map(|value| value.to_ascii_lowercase())
        .collect::<Vec<_>>()
        .join(" ");
    if text.contains("acp") || text.contains("agent") || text.contains("codex") {
        VerifierDependency::Agent
    } else if text.contains("repo")
        || text.contains("repository")
        || text.contains("test")
        || text.contains("build")
    {
        VerifierDependency::Repository
    } else {
        VerifierDependency::Static
    }
}

fn derive_task_graph_quality(validation: &ValidationResult) -> u8 {
    if validation.is_valid() { 100 } else { 0 }
}

fn derive_execution_readiness(validation: &ValidationResult) -> u8 {
    if !validation.is_valid() {
        0
    } else if validation.obligations.is_empty() {
        100
    } else {
        50
    }
}

fn select_candidate_frontier(
    candidates: &mut [PlanSketch],
    policy: FrontierPolicy,
) -> Result<FrontierMetadata, PlanError> {
    let valid_indexes = candidates
        .iter()
        .enumerate()
        .filter_map(|(idx, candidate)| candidate.validation.is_valid().then_some(idx))
        .collect::<Vec<_>>();
    if valid_indexes.is_empty() {
        return Err(PlanError::InvalidCandidate(
            "no valid candidate in planning population".to_string(),
        ));
    }

    let max_elite = policy.max_elite.max(1);
    let min_elite = policy.min_elite.min(valid_indexes.len()).min(max_elite);
    let mut retained = BTreeSet::new();

    for &idx in &valid_indexes {
        let dominated = valid_indexes
            .iter()
            .copied()
            .filter(|other_idx| *other_idx != idx)
            .any(|other_idx| dominates_candidate(&candidates[other_idx], &candidates[idx]));
        if !dominated {
            retained.insert(candidates[idx].id.clone());
        }
    }

    if policy.retain_descriptor_cells {
        let mut best_by_cell: BTreeMap<(CostTier, Risk, VerifierDependency), usize> =
            BTreeMap::new();
        for &idx in &valid_indexes {
            let cell = descriptor_cell(&candidates[idx]);
            let replace = best_by_cell
                .get(&cell)
                .map(|&existing_idx| {
                    candidate_selection_key(&candidates[idx])
                        > candidate_selection_key(&candidates[existing_idx])
                })
                .unwrap_or(true);
            if replace {
                best_by_cell.insert(cell, idx);
            }
        }
        retained.extend(
            best_by_cell
                .values()
                .map(|&idx| candidates[idx].id.clone())
                .collect::<Vec<_>>(),
        );
    }

    let sorted_valid = sorted_candidate_ids_by_boundary_key(candidates, &valid_indexes);
    for candidate_id in &sorted_valid {
        if retained.len() >= min_elite {
            break;
        }
        retained.insert(candidate_id.clone());
    }

    let retained_indexes = retained
        .iter()
        .filter_map(|candidate_id| candidate_index(candidates, candidate_id))
        .collect::<Vec<_>>();
    let mut sorted_retained = sorted_candidate_ids_by_boundary_key(candidates, &retained_indexes);
    let capacity_discards = if sorted_retained.len() > max_elite {
        let discarded = sorted_retained.split_off(max_elite);
        retained = sorted_retained.iter().cloned().collect();
        discarded
    } else {
        Vec::new()
    };

    let selected_candidate_id = sorted_candidate_ids_by_boundary_key(
        candidates,
        &retained
            .iter()
            .filter_map(|candidate_id| candidate_index(candidates, candidate_id))
            .collect::<Vec<_>>(),
    )
    .into_iter()
    .next()
    .ok_or_else(|| PlanError::InvalidCandidate("frontier retained no candidates".to_string()))?;

    for candidate in candidates.iter_mut() {
        candidate.status = if candidate.id == selected_candidate_id {
            SketchStatus::Selected
        } else if candidate.validation.is_valid() {
            SketchStatus::Valid
        } else {
            SketchStatus::Invalid
        };
    }

    let mut discard_reasons = BTreeMap::new();
    for candidate in candidates
        .iter()
        .filter(|candidate| !candidate.validation.is_valid())
    {
        discard_reasons.insert(candidate.id.clone(), FrontierDiscardReason::Invalid);
    }
    for candidate in candidates
        .iter()
        .filter(|candidate| candidate.validation.is_valid() && !retained.contains(&candidate.id))
    {
        discard_reasons.insert(candidate.id.clone(), FrontierDiscardReason::Dominated);
    }
    for candidate_id in capacity_discards {
        discard_reasons.insert(candidate_id, FrontierDiscardReason::Capacity);
    }

    Ok(FrontierMetadata {
        policy,
        retained_candidate_ids: retained.into_iter().collect(),
        discarded_candidates: discard_reasons
            .into_iter()
            .map(|(candidate_id, reason)| FrontierDiscard {
                candidate_id,
                reason,
            })
            .collect(),
        selected_candidate_id,
    })
}

fn dominates_candidate(left: &PlanSketch, right: &PlanSketch) -> bool {
    let no_worse = left.score.verifier_strength >= right.score.verifier_strength
        && left.score.obligation_burden <= right.score.obligation_burden
        && left.score.risk_profile <= right.score.risk_profile
        && left.score.cost_tier <= right.score.cost_tier;
    let strictly_better = left.score.verifier_strength > right.score.verifier_strength
        || left.score.obligation_burden < right.score.obligation_burden
        || left.score.risk_profile < right.score.risk_profile
        || left.score.cost_tier < right.score.cost_tier;
    no_worse && strictly_better
}

fn descriptor_cell(candidate: &PlanSketch) -> (CostTier, Risk, VerifierDependency) {
    (
        candidate.descriptor.cost_tier,
        candidate.descriptor.risk_profile,
        candidate.descriptor.verifier_dependency,
    )
}

fn candidate_index(candidates: &[PlanSketch], candidate_id: &str) -> Option<usize> {
    candidates
        .iter()
        .position(|candidate| candidate.id == candidate_id)
}

fn sorted_candidate_ids_by_boundary_key(
    candidates: &[PlanSketch],
    indexes: &[usize],
) -> Vec<String> {
    let mut sorted = indexes.to_vec();
    sorted.sort_by(|left_idx, right_idx| {
        candidate_selection_key(&candidates[*right_idx])
            .cmp(&candidate_selection_key(&candidates[*left_idx]))
    });
    sorted
        .into_iter()
        .map(|idx| candidates[idx].id.clone())
        .collect()
}

fn candidate_selection_key(
    candidate: &PlanSketch,
) -> (
    VerifierStrength,
    Reverse<ObligationBurden>,
    Reverse<Risk>,
    Reverse<CostTier>,
    u8,
    u8,
    String,
) {
    (
        candidate.score.verifier_strength,
        Reverse(candidate.score.obligation_burden),
        Reverse(candidate.score.risk_profile),
        Reverse(candidate.score.cost_tier),
        candidate.score.task_graph_quality,
        candidate.score.execution_readiness,
        candidate.id.clone(),
    )
}

#[must_use]
pub fn stable_candidate_hash(candidate: &PlanSketch) -> String {
    stable_hash_json(&candidate.candidate)
}

#[must_use]
pub fn validation_outcome_status(validation: &ValidationResult) -> ValidationOutcomeStatus {
    if !validation.hard_failures.is_empty() || !validation.spec_hash_ok {
        ValidationOutcomeStatus::Failed
    } else if !validation.obligations.is_empty() {
        ValidationOutcomeStatus::Blocked
    } else {
        ValidationOutcomeStatus::Passed
    }
}

/// Derives a validation-budget reward bucket for future resource allocation.
///
/// This is stage telemetry, not final plan quality: it measures whether one
/// validator pull made concrete validation progress. Champion selection and
/// frontier retention must continue to use admissibility, score, and descriptor
/// metadata instead of this reward.
#[must_use]
pub fn derive_validation_reward(
    status: ValidationOutcomeStatus,
    obligations_discharged: usize,
    evidence_refs_added: usize,
) -> ValidationRewardBucket {
    match status {
        ValidationOutcomeStatus::Failed | ValidationOutcomeStatus::Blocked => {
            ValidationRewardBucket::Zero
        }
        ValidationOutcomeStatus::Passed if obligations_discharged > 0 || evidence_refs_added > 0 => {
            ValidationRewardBucket::Full
        }
        ValidationOutcomeStatus::Passed => ValidationRewardBucket::Partial,
    }
}

#[must_use]
pub fn static_validator_outcome(
    run_id: impl Into<String>,
    candidate: &PlanSketch,
    baseline_evidence_ref_count: usize,
    generated_at_unix_secs: u64,
) -> ValidatorOutcome {
    let status = validation_outcome_status(&candidate.validation);
    let evidence_refs_added = candidate
        .validation
        .evidence_refs
        .len()
        .saturating_sub(baseline_evidence_ref_count);
    let obligations_discharged = 0;
    ValidatorOutcome {
        candidate_id: candidate.id.clone(),
        candidate_hash: stable_candidate_hash(candidate),
        run_id: run_id.into(),
        tier: ValidationTier::Static,
        status,
        reward: derive_validation_reward(status, obligations_discharged, evidence_refs_added),
        obligations_discharged,
        evidence_refs_added,
        generated_at_unix_secs,
    }
}

pub fn ordered_tasks(plan: &Plan) -> Result<Vec<Task>, PlanError> {
    let graph = task_graph(plan)?;
    let indexes = toposort(&graph, None).map_err(|_| PlanError::Cycle)?;
    Ok(indexes
        .into_iter()
        .map(|idx| graph[idx].clone())
        .collect::<Vec<_>>())
}

fn task_graph(plan: &Plan) -> Result<DiGraph<Task, ()>, PlanError> {
    let mut graph = DiGraph::new();
    let mut indexes: BTreeMap<String, NodeIndex> = BTreeMap::new();

    for task in &plan.tasks {
        let idx = graph.add_node(task.clone());
        indexes.insert(task.id.clone(), idx);
    }

    for task in &plan.tasks {
        let task_idx = indexes[&task.id];
        for dependency in &task.depends_on {
            let Some(dependency_idx) = indexes.get(dependency).copied() else {
                return Err(PlanError::MissingDependency {
                    task_id: task.id.clone(),
                    dependency: dependency.clone(),
                });
            };
            graph.add_edge(dependency_idx, task_idx, ());
        }
    }

    for idx in graph.node_indices() {
        let _ = graph.neighbors_directed(idx, Direction::Incoming).count();
    }

    Ok(graph)
}

const fn default_domain_kind() -> DomainKind {
    DomainKind::Coding
}

fn stable_hash_json<T>(value: &T) -> String
where
    T: Serialize,
{
    let payload = serde_json::to_vec(value).unwrap_or_default();
    let mut hash = 0xcbf29ce484222325_u64;
    for byte in payload {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    format!("{hash:016x}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    #[tokio::test]
    async fn deterministic_planner_creates_ordered_dag() {
        let planner = DeterministicPlanner;
        let plan = planner.create_plan("add health endpoint").await.unwrap();
        let ordered = ordered_tasks(&plan).unwrap();

        assert_eq!(
            ordered
                .iter()
                .map(|task| task.id.as_str())
                .collect::<Vec<_>>(),
            vec!["inspect-repo", "implement-goal", "verify-goal"]
        );
        assert!(
            ordered
                .iter()
                .all(|task| !task.acceptance_criteria.is_empty())
        );
    }

    #[test]
    fn validation_rejects_cycles() {
        let plan = Plan {
            id: "cycle".to_string(),
            goal: "cycle".to_string(),
            tasks: vec![
                Task::new(
                    "a",
                    "A",
                    "A",
                    Risk::Low,
                    vec![AcceptanceCriterion {
                        description: "A done".to_string(),
                    }],
                    vec!["b".to_string()],
                ),
                Task::new(
                    "b",
                    "B",
                    "B",
                    Risk::Low,
                    vec![AcceptanceCriterion {
                        description: "B done".to_string(),
                    }],
                    vec!["a".to_string()],
                ),
            ],
            domain: DomainKind::Coding,
            harness: None,
        };

        assert!(matches!(validate_plan(&plan), Err(PlanError::Cycle)));
    }

    #[tokio::test]
    async fn generic_harness_planner_selects_valid_candidate() {
        let planner = HarnessPlanner::new(GenericDomainAdapter);
        let plan = planner
            .create_plan("open a new community workshop")
            .await
            .unwrap();

        assert_eq!(plan.domain, DomainKind::Generic);
        assert!(plan.harness.is_some());
        assert_eq!(
            plan.tasks
                .iter()
                .map(|task| task.id.as_str())
                .collect::<Vec<_>>(),
            vec!["ground-inputs", "select-minimal-plan"]
        );
        let harness = plan.harness.as_ref().unwrap();
        assert_eq!(harness.candidates.len(), 4);
        assert_eq!(
            harness.frontier.selected_candidate_id,
            harness.selected_candidate_id
        );
        assert!(
            harness
                .frontier
                .retained_candidate_ids
                .contains(&harness.selected_candidate_id)
        );
        assert_eq!(harness.frontier.retained_candidate_ids.len(), 4);
        assert!(harness.frontier.discarded_candidates.is_empty());
        let selected = harness
            .candidates
            .iter()
            .find(|candidate| candidate.id == harness.selected_candidate_id)
            .unwrap();
        assert_eq!(selected.score.verifier_strength, VerifierStrength::Strong);
        assert_eq!(selected.score.obligation_burden, ObligationBurden::None);
        assert_eq!(selected.descriptor.cost_tier, CostTier::Low);
        assert_eq!(
            selected.descriptor.verifier_dependency,
            VerifierDependency::Static
        );
    }

    #[test]
    fn candidate_validation_rejects_hidden_tbd_work() {
        let adapter = GenericDomainAdapter;
        let spec = adapter.build_spec("plan a launch");
        let mut candidate = adapter.seed_candidates(&spec).remove(0);
        candidate.tasks[0].description = "TBD".to_string();

        let result = adapter.validate_candidate(&spec, &candidate);

        assert!(!result.is_valid());
        assert!(
            result
                .hard_failures
                .iter()
                .any(|finding| finding.code == "tbd-task")
        );
    }

    #[test]
    fn problem_spec_hash_detects_mutation() {
        let adapter = GenericDomainAdapter;
        let mut spec = adapter.build_spec("plan a migration");
        spec.goal = "different".to_string();
        let candidate = adapter.seed_candidates(&spec).remove(0);

        let result = adapter.validate_candidate(&spec, &candidate);

        assert!(!result.spec_hash_ok);
    }

    #[test]
    fn candidate_score_defaults_are_conservative() {
        let score = CandidateScore::default();
        let descriptor = CandidateDescriptor::default();

        assert_eq!(score.verifier_strength, VerifierStrength::Weak);
        assert_eq!(score.obligation_burden, ObligationBurden::Many);
        assert_eq!(score.risk_profile, Risk::High);
        assert_eq!(score.cost_tier, CostTier::High);
        assert_eq!(score.task_graph_quality, 0);
        assert_eq!(score.execution_readiness, 0);
        assert_eq!(descriptor.cost_tier, CostTier::High);
        assert_eq!(descriptor.risk_profile, Risk::High);
        assert_eq!(descriptor.verifier_dependency, VerifierDependency::Static);
    }

    #[test]
    fn obligation_burden_buckets_by_validation_obligations() {
        let mut validation = ValidationResult::valid(Vec::new());
        assert_eq!(
            derive_obligation_burden(&validation),
            ObligationBurden::None
        );

        validation.obligations.push(Obligation {
            id: "first".to_string(),
            description: "First missing input.".to_string(),
            producer: "fixture".to_string(),
            regenerate_command: "true".to_string(),
            validation_command: "true".to_string(),
        });
        assert_eq!(derive_obligation_burden(&validation), ObligationBurden::One);

        validation.obligations.push(Obligation {
            id: "second".to_string(),
            description: "Second missing input.".to_string(),
            producer: "fixture".to_string(),
            regenerate_command: "true".to_string(),
            validation_command: "true".to_string(),
        });
        assert_eq!(
            derive_obligation_burden(&validation),
            ObligationBurden::Many
        );
    }

    #[test]
    fn score_candidates_derives_structured_metadata() {
        let adapter = GenericDomainAdapter;
        let spec = adapter.build_spec("plan a launch");
        let candidate = adapter.seed_candidates(&spec).remove(0);
        let validation = adapter.validate_candidate(&spec, &candidate);
        let mut candidates = vec![PlanSketch {
            id: "candidate-1".to_string(),
            problem_id: spec.id.clone(),
            parent_ids: Vec::new(),
            candidate,
            status: SketchStatus::Valid,
            validation,
            score: CandidateScore::default(),
            descriptor: CandidateDescriptor::default(),
            created_by: "test".to_string(),
        }];

        score_candidates(&mut candidates);

        let scored = &candidates[0];
        assert_eq!(scored.score.verifier_strength, VerifierStrength::Strong);
        assert_eq!(scored.score.obligation_burden, ObligationBurden::None);
        assert_eq!(scored.score.risk_profile, Risk::Low);
        assert_eq!(scored.score.cost_tier, CostTier::Low);
        assert_eq!(scored.score.task_graph_quality, 100);
        assert_eq!(scored.score.execution_readiness, 100);
        assert_eq!(scored.descriptor.risk_profile, Risk::Low);
        assert_eq!(scored.descriptor.cost_tier, CostTier::Low);
        assert_eq!(
            scored.descriptor.verifier_dependency,
            VerifierDependency::Static
        );
    }

    #[test]
    fn validation_reward_buckets_measure_stage_progress() {
        assert_eq!(
            derive_validation_reward(ValidationOutcomeStatus::Passed, 1, 0),
            ValidationRewardBucket::Full
        );
        assert_eq!(
            derive_validation_reward(ValidationOutcomeStatus::Passed, 0, 1),
            ValidationRewardBucket::Full
        );
        assert_eq!(
            derive_validation_reward(ValidationOutcomeStatus::Passed, 0, 0),
            ValidationRewardBucket::Partial
        );
        assert_eq!(
            derive_validation_reward(ValidationOutcomeStatus::Failed, 1, 1),
            ValidationRewardBucket::Zero
        );
        assert_eq!(
            derive_validation_reward(ValidationOutcomeStatus::Blocked, 1, 1),
            ValidationRewardBucket::Zero
        );
        assert_eq!(ValidationRewardBucket::Full.as_f64(), 1.0);
        assert_eq!(ValidationRewardBucket::Partial.as_f64(), 0.5);
        assert_eq!(ValidationRewardBucket::Zero.as_f64(), 0.0);
    }

    #[test]
    fn static_validator_outcome_uses_candidate_hash_and_baseline_evidence() {
        let mut candidate = frontier_test_sketch(
            "candidate-1",
            VerifierStrength::Strong,
            ObligationBurden::None,
            Risk::Low,
            CostTier::Low,
            VerifierDependency::Static,
            true,
        );
        candidate.validation.evidence_refs.push(EvidenceRef {
            source: "validator".to_string(),
            summary: "Concrete validation evidence.".to_string(),
        });

        let outcome = static_validator_outcome("run-1", &candidate, 0, 42);

        assert_eq!(outcome.run_id, "run-1");
        assert_eq!(outcome.candidate_id, "candidate-1");
        assert_eq!(outcome.candidate_hash, stable_candidate_hash(&candidate));
        assert_eq!(outcome.tier, ValidationTier::Static);
        assert_eq!(outcome.status, ValidationOutcomeStatus::Passed);
        assert_eq!(outcome.reward, ValidationRewardBucket::Full);
        assert_eq!(outcome.evidence_refs_added, 1);
        assert_eq!(outcome.obligations_discharged, 0);
        assert_eq!(outcome.generated_at_unix_secs, 42);
    }

    #[test]
    fn generic_adapter_seeds_deterministic_valid_candidate_population() {
        let adapter = GenericDomainAdapter;
        let spec = adapter.build_spec("plan a launch");
        let first = adapter.seed_candidates(&spec);
        let second = adapter.seed_candidates(&spec);

        assert_eq!(first, second);
        assert!(first.len() >= 3);

        for candidate in &first {
            let validation = adapter.validate_candidate(&spec, candidate);
            assert!(validation.is_valid(), "{:?}", validation.hard_failures);
            assert!(
                candidate
                    .tasks
                    .iter()
                    .all(|task| !task.description.to_ascii_lowercase().contains("tbd"))
            );
            validate_plan(&Plan {
                id: "candidate-validation".to_string(),
                goal: spec.goal.clone(),
                tasks: candidate.tasks.clone(),
                domain: spec.domain,
                harness: None,
            })
            .unwrap();
        }
    }

    #[test]
    fn generic_adapter_seeds_descriptor_diverse_candidates() {
        let adapter = GenericDomainAdapter;
        let spec = adapter.build_spec("plan a launch");
        let mut sketches = adapter
            .seed_candidates(&spec)
            .into_iter()
            .enumerate()
            .map(|(idx, candidate)| {
                let validation = adapter.validate_candidate(&spec, &candidate);
                PlanSketch {
                    id: format!("candidate-{}", idx + 1),
                    problem_id: spec.id.clone(),
                    parent_ids: Vec::new(),
                    candidate,
                    status: SketchStatus::Valid,
                    validation,
                    score: CandidateScore::default(),
                    descriptor: CandidateDescriptor::default(),
                    created_by: "test".to_string(),
                }
            })
            .collect::<Vec<_>>();

        score_candidates(&mut sketches);

        let descriptor_cells = sketches
            .iter()
            .map(|candidate| {
                (
                    candidate.descriptor.cost_tier,
                    candidate.descriptor.risk_profile,
                    candidate.descriptor.verifier_dependency,
                )
            })
            .collect::<BTreeSet<_>>();
        let task_shapes = sketches
            .iter()
            .map(|candidate| {
                (
                    candidate.candidate.summary.clone(),
                    candidate.candidate.tasks.len(),
                    candidate.candidate.tasks.last().unwrap().id.clone(),
                )
            })
            .collect::<BTreeSet<_>>();

        assert!(descriptor_cells.len() >= 3, "{descriptor_cells:?}");
        assert_eq!(task_shapes.len(), sketches.len());
    }

    #[test]
    fn frontier_excludes_invalid_candidates() {
        let mut candidates = vec![
            frontier_test_sketch(
                "valid",
                VerifierStrength::Moderate,
                ObligationBurden::None,
                Risk::Medium,
                CostTier::Medium,
                VerifierDependency::Static,
                true,
            ),
            frontier_test_sketch(
                "invalid",
                VerifierStrength::Strong,
                ObligationBurden::None,
                Risk::Low,
                CostTier::Low,
                VerifierDependency::Agent,
                false,
            ),
        ];

        let frontier =
            select_candidate_frontier(&mut candidates, FrontierPolicy::default()).unwrap();

        assert_eq!(frontier.selected_candidate_id, "valid");
        assert_eq!(frontier.retained_candidate_ids, vec!["valid".to_string()]);
        assert_eq!(
            frontier.discarded_candidates,
            vec![FrontierDiscard {
                candidate_id: "invalid".to_string(),
                reason: FrontierDiscardReason::Invalid,
            }]
        );
        assert_eq!(candidates[0].status, SketchStatus::Selected);
        assert_eq!(candidates[1].status, SketchStatus::Invalid);
    }

    #[test]
    fn coarse_bucket_dominance_discards_weaker_candidate() {
        let mut candidates = vec![
            frontier_test_sketch(
                "dominant",
                VerifierStrength::Strong,
                ObligationBurden::None,
                Risk::Low,
                CostTier::Low,
                VerifierDependency::Static,
                true,
            ),
            frontier_test_sketch(
                "dominated",
                VerifierStrength::Strong,
                ObligationBurden::One,
                Risk::Medium,
                CostTier::Medium,
                VerifierDependency::Static,
                true,
            ),
        ];

        let frontier = select_candidate_frontier(
            &mut candidates,
            FrontierPolicy {
                min_elite: 1,
                max_elite: 8,
                retain_descriptor_cells: false,
            },
        )
        .unwrap();

        assert_eq!(frontier.selected_candidate_id, "dominant");
        assert_eq!(
            frontier.retained_candidate_ids,
            vec!["dominant".to_string()]
        );
        assert_eq!(
            frontier.discarded_candidates,
            vec![FrontierDiscard {
                candidate_id: "dominated".to_string(),
                reason: FrontierDiscardReason::Dominated,
            }]
        );
    }

    #[test]
    fn descriptor_cell_retention_keeps_dominated_cell_winner() {
        let mut candidates = vec![
            frontier_test_sketch(
                "static-best",
                VerifierStrength::Strong,
                ObligationBurden::None,
                Risk::Low,
                CostTier::Low,
                VerifierDependency::Static,
                true,
            ),
            frontier_test_sketch(
                "agent-cell",
                VerifierStrength::Strong,
                ObligationBurden::One,
                Risk::High,
                CostTier::High,
                VerifierDependency::Agent,
                true,
            ),
        ];

        let frontier = select_candidate_frontier(
            &mut candidates,
            FrontierPolicy {
                min_elite: 1,
                max_elite: 8,
                retain_descriptor_cells: true,
            },
        )
        .unwrap();

        assert_eq!(frontier.selected_candidate_id, "static-best");
        assert_eq!(
            frontier.retained_candidate_ids,
            vec!["agent-cell".to_string(), "static-best".to_string()]
        );
        assert!(frontier.discarded_candidates.is_empty());
    }

    #[test]
    fn frontier_enforces_min_elite_and_max_elite() {
        let mut candidates = vec![
            frontier_test_sketch(
                "candidate-1",
                VerifierStrength::Strong,
                ObligationBurden::None,
                Risk::Low,
                CostTier::Low,
                VerifierDependency::Static,
                true,
            ),
            frontier_test_sketch(
                "candidate-2",
                VerifierStrength::Strong,
                ObligationBurden::One,
                Risk::Low,
                CostTier::Low,
                VerifierDependency::Repository,
                true,
            ),
            frontier_test_sketch(
                "candidate-3",
                VerifierStrength::Moderate,
                ObligationBurden::None,
                Risk::Low,
                CostTier::Low,
                VerifierDependency::Agent,
                true,
            ),
            frontier_test_sketch(
                "candidate-4",
                VerifierStrength::Weak,
                ObligationBurden::None,
                Risk::Low,
                CostTier::Low,
                VerifierDependency::Agent,
                true,
            ),
        ];

        let frontier = select_candidate_frontier(
            &mut candidates,
            FrontierPolicy {
                min_elite: 1,
                max_elite: 2,
                retain_descriptor_cells: true,
            },
        )
        .unwrap();

        assert_eq!(frontier.selected_candidate_id, "candidate-1");
        assert_eq!(
            frontier.retained_candidate_ids,
            vec!["candidate-1".to_string(), "candidate-2".to_string()]
        );
        assert_eq!(
            frontier.discarded_candidates,
            vec![
                FrontierDiscard {
                    candidate_id: "candidate-3".to_string(),
                    reason: FrontierDiscardReason::Capacity,
                },
                FrontierDiscard {
                    candidate_id: "candidate-4".to_string(),
                    reason: FrontierDiscardReason::Dominated,
                },
            ]
        );
    }

    #[test]
    fn frontier_tie_breaks_by_stable_candidate_id() {
        let mut candidates = vec![
            frontier_test_sketch(
                "candidate-1",
                VerifierStrength::Strong,
                ObligationBurden::None,
                Risk::Low,
                CostTier::Low,
                VerifierDependency::Static,
                true,
            ),
            frontier_test_sketch(
                "candidate-2",
                VerifierStrength::Strong,
                ObligationBurden::None,
                Risk::Low,
                CostTier::Low,
                VerifierDependency::Static,
                true,
            ),
        ];

        let frontier =
            select_candidate_frontier(&mut candidates, FrontierPolicy::default()).unwrap();

        assert_eq!(frontier.selected_candidate_id, "candidate-2");
        assert!(
            frontier
                .retained_candidate_ids
                .contains(&frontier.selected_candidate_id)
        );
    }

    fn frontier_test_sketch(
        id: &str,
        verifier_strength: VerifierStrength,
        obligation_burden: ObligationBurden,
        risk_profile: Risk,
        cost_tier: CostTier,
        verifier_dependency: VerifierDependency,
        valid: bool,
    ) -> PlanSketch {
        let validation = if valid {
            ValidationResult::valid(Vec::new())
        } else {
            ValidationResult {
                spec_hash_ok: true,
                hard_failures: vec![ValidationFinding {
                    code: "invalid-fixture".to_string(),
                    message: "invalid fixture".to_string(),
                }],
                soft_findings: Vec::new(),
                obligations: Vec::new(),
                evidence_refs: Vec::new(),
            }
        };
        PlanSketch {
            id: id.to_string(),
            problem_id: "spec-frontier-test".to_string(),
            parent_ids: Vec::new(),
            candidate: PlanCandidate {
                summary: format!("{id} summary"),
                tasks: vec![Task::new(
                    format!("{id}-task"),
                    "Fixture task",
                    "Fixture task description",
                    risk_profile,
                    vec![AcceptanceCriterion {
                        description: "Fixture task is accepted.".to_string(),
                    }],
                    Vec::new(),
                )],
                assumptions: Vec::new(),
                risks: Vec::new(),
                verification: vec!["Validate fixture.".to_string()],
                rollout: Vec::new(),
                blockers: Vec::new(),
            },
            status: if valid {
                SketchStatus::Valid
            } else {
                SketchStatus::Invalid
            },
            validation,
            score: CandidateScore {
                verifier_strength,
                obligation_burden,
                risk_profile,
                cost_tier,
                task_graph_quality: 100,
                execution_readiness: 100,
            },
            descriptor: CandidateDescriptor {
                cost_tier,
                risk_profile,
                verifier_dependency,
            },
            created_by: "test".to_string(),
        }
    }
}
