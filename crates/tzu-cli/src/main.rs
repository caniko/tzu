use anyhow::{Context, Result};
use clap::{Parser, Subcommand, ValueEnum};
use tzu_config::load_config;
use tzu_core::{
    DomainKind, FrontierDiscardReason, PlanError, PlanSketch, ProjectState, PromptInspection,
    TaskStatus, ordered_tasks,
};
use tzu_runner::{PlanningDomain, RunMode, TzuRunner, default_database_url};

#[derive(Debug, Parser)]
#[command(name = "tzu")]
#[command(about = "Local-first general planning harness backed by ACP agents")]
struct Cli {
    #[arg(long, env = "TZU_DATABASE_URL")]
    database_url: Option<String>,
    #[arg(long, default_value = ".")]
    project_root: std::path::PathBuf,
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    Init,
    Plan {
        goal: String,
        #[arg(long, value_enum, default_value_t = CliPlanningDomain::Generic)]
        domain: CliPlanningDomain,
        #[arg(long = "context-root")]
        context_roots: Vec<std::path::PathBuf>,
        #[arg(long)]
        include_nested_contexts: bool,
    },
    Run {
        task_id: String,
    },
    Status,
    Inspect {
        #[arg(long)]
        frontier: bool,
    },
}

#[derive(Debug, Clone, Copy, ValueEnum)]
enum CliPlanningDomain {
    Generic,
    Coding,
}

impl From<CliPlanningDomain> for PlanningDomain {
    fn from(value: CliPlanningDomain) -> Self {
        match value {
            CliPlanningDomain::Generic => Self::Generic,
            CliPlanningDomain::Coding => Self::Coding,
        }
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .with_writer(std::io::stderr)
        .init();

    let cli = Cli::parse();
    run(cli).await
}

async fn run(cli: Cli) -> Result<()> {
    let config = load_config().context("load tzu config")?;
    let root = cli
        .project_root
        .canonicalize()
        .with_context(|| format!("resolve project root {}", cli.project_root.display()))?;
    let database_url = cli
        .database_url
        .unwrap_or_else(|| default_database_url(&root));
    let runner = TzuRunner::connect(&root, &database_url).await?;

    match cli.command {
        Command::Init => {
            let state = runner.init().await?;
            println!("initialized tzu state for {}", state.project_root);
        }
        Command::Plan {
            goal,
            domain,
            context_roots,
            include_nested_contexts,
        } => {
            let include_nested_contexts = include_nested_contexts || config.include_nested_contexts;
            let state = match runner
                .plan_with_context(&goal, domain.into(), context_roots, include_nested_contexts)
                .await
            {
                Ok(state) => state,
                Err(tzu_runner::RunnerError::Planning(PlanError::PromptNeedsImprovement(
                    inspection,
                ))) => {
                    print_prompt_inspection(&inspection);
                    return Err(tzu_runner::RunnerError::Planning(
                        PlanError::PromptNeedsImprovement(inspection),
                    )
                    .into());
                }
                Err(error) => return Err(error.into()),
            };
            print_status(&state)?;
        }
        Command::Run { task_id } => {
            let report = runner.run_task(&task_id, RunMode::from_env()).await?;
            println!("{}", serde_json::to_string_pretty(&report)?);
        }
        Command::Status => {
            let state = runner.status().await?;
            print_status(&state)?;
        }
        Command::Inspect { frontier: _ } => {
            let state = runner.status().await?;
            print_frontier(&state)?;
        }
    }

    Ok(())
}

fn print_prompt_inspection(inspection: &PromptInspection) {
    eprintln!("Goal prompt needs improvement before planning.");
    for finding in &inspection.findings {
        eprintln!("- {}: {}", finding.code, finding.message);
    }
    if let Some(suggestion) = &inspection.suggestion {
        eprintln!("Recommended model: {}", suggestion.model);
        eprintln!(
            "Recommended reasoning_effort: {}",
            suggestion.reasoning_effort
        );
        eprintln!("Rationale: {}", suggestion.rationale);
        eprintln!("Prompt guidance: {}", suggestion.improved_prompt_guidance);
    }
}

fn print_status(state: &ProjectState) -> Result<()> {
    let Some(plan) = state.current_plan.as_ref() else {
        println!("No current plan for {}", state.project_root);
        return Ok(());
    };

    println!("Plan: {}", plan.id);
    println!("Goal: {}", plan.goal);
    println!("Domain: {}", domain_label(plan.domain));
    if let Some(harness) = plan.harness.as_ref() {
        println!("Selected candidate: {}", harness.selected_candidate_id);
        println!("Candidates evaluated: {}", harness.candidates.len());
        println!(
            "Frontier size: {}",
            harness.frontier.retained_candidate_ids.len()
        );
    }
    for task in ordered_tasks(plan)? {
        println!(
            "- {} [{}] {}",
            task.id,
            status_label(task.status),
            task.title
        );
        for criterion in task.acceptance_criteria {
            println!("  acceptance: {}", criterion.description);
        }
    }
    if !state.run_reports.is_empty() {
        println!("Run reports: {}", state.run_reports.len());
    }
    Ok(())
}

fn print_frontier(state: &ProjectState) -> Result<()> {
    let Some(plan) = state.current_plan.as_ref() else {
        println!("No current plan for {}", state.project_root);
        return Ok(());
    };
    let Some(harness) = plan.harness.as_ref() else {
        println!("Plan {} has no harness metadata", plan.id);
        return Ok(());
    };

    println!("Plan: {}", plan.id);
    println!(
        "Selected champion: {}",
        harness.frontier.selected_candidate_id
    );
    println!(
        "Policy: min_elite={} max_elite={} descriptor_cells={}",
        harness.frontier.policy.min_elite,
        harness.frontier.policy.max_elite,
        harness.frontier.policy.retain_descriptor_cells
    );
    println!(
        "Retained candidates: {}",
        harness.frontier.retained_candidate_ids.len()
    );
    for candidate_id in &harness.frontier.retained_candidate_ids {
        if let Some(candidate) = harness
            .candidates
            .iter()
            .find(|candidate| candidate.id == *candidate_id)
        {
            print_frontier_candidate(
                candidate,
                candidate.id == harness.frontier.selected_candidate_id,
            );
        } else {
            println!("- {} [missing metadata]", candidate_id);
        }
    }

    println!(
        "Discarded candidates: {}",
        harness.frontier.discarded_candidates.len()
    );
    for discarded in &harness.frontier.discarded_candidates {
        println!(
            "- {} reason={}",
            discarded.candidate_id,
            discard_reason_label(discarded.reason)
        );
    }
    Ok(())
}

fn print_frontier_candidate(candidate: &PlanSketch, selected: bool) {
    let marker = if selected { " [selected]" } else { "" };
    println!(
        "- {}{}: {}",
        candidate.id, marker, candidate.candidate.summary
    );
    println!(
        "  score: verifier={:?} obligations={:?} risk={:?} cost={:?} task_graph_quality={} execution_readiness={}",
        candidate.score.verifier_strength,
        candidate.score.obligation_burden,
        candidate.score.risk_profile,
        candidate.score.cost_tier,
        candidate.score.task_graph_quality,
        candidate.score.execution_readiness,
    );
    println!(
        "  descriptor: cost={:?} risk={:?} verifier_dependency={:?}",
        candidate.descriptor.cost_tier,
        candidate.descriptor.risk_profile,
        candidate.descriptor.verifier_dependency,
    );
}

fn discard_reason_label(reason: FrontierDiscardReason) -> &'static str {
    match reason {
        FrontierDiscardReason::Invalid => "invalid",
        FrontierDiscardReason::Dominated => "dominated",
        FrontierDiscardReason::Capacity => "capacity",
    }
}

fn domain_label(domain: DomainKind) -> &'static str {
    match domain {
        DomainKind::Generic => "generic",
        DomainKind::Coding => "coding",
    }
}

fn status_label(status: TaskStatus) -> &'static str {
    match status {
        TaskStatus::Pending => "pending",
        TaskStatus::Running => "running",
        TaskStatus::Completed => "completed",
        TaskStatus::Failed => "failed",
        TaskStatus::Blocked => "blocked",
    }
}

#[cfg(test)]
mod tests {
    use std::process::Command as ProcessCommand;

    #[test]
    fn cli_smoke_plan_status_and_mock_run() {
        let temp = tempfile::tempdir().unwrap();
        let output = ProcessCommand::new("git")
            .args(["init", "-b", "main"])
            .current_dir(temp.path())
            .output()
            .unwrap();
        assert!(output.status.success());
        let db = temp.path().join("state.sqlite");
        let db_url = format!("sqlite://{}", db.display());
        let Ok(bin) = std::env::var("CARGO_BIN_EXE_tzu") else {
            return;
        };

        let init = ProcessCommand::new(&bin)
            .arg("--project-root")
            .arg(temp.path())
            .arg("--database-url")
            .arg(&db_url)
            .env("XDG_CONFIG_HOME", temp.path().join("config"))
            .arg("init")
            .output()
            .unwrap();
        assert!(
            init.status.success(),
            "{}",
            String::from_utf8_lossy(&init.stderr)
        );

        let bad_plan = ProcessCommand::new(&bin)
            .arg("--project-root")
            .arg(temp.path())
            .arg("--database-url")
            .arg(&db_url)
            .env("XDG_CONFIG_HOME", temp.path().join("config"))
            .args(["plan", "TODO"])
            .output()
            .unwrap();
        assert!(!bad_plan.status.success());
        let bad_plan_stderr = String::from_utf8_lossy(&bad_plan.stderr);
        assert!(bad_plan_stderr.contains("Goal prompt needs improvement before planning."));
        assert!(bad_plan_stderr.contains("Recommended model: gpt-5.5"));
        assert!(bad_plan_stderr.contains("Recommended reasoning_effort: medium"));

        let plan = ProcessCommand::new(&bin)
            .arg("--project-root")
            .arg(temp.path())
            .arg("--database-url")
            .arg(&db_url)
            .env("XDG_CONFIG_HOME", temp.path().join("config"))
            .args([
                "plan",
                "add health endpoint",
                "--domain",
                "coding",
                "--context-root",
            ])
            .arg(temp.path())
            .output()
            .unwrap();
        assert!(
            plan.status.success(),
            "{}",
            String::from_utf8_lossy(&plan.stderr)
        );
        assert!(String::from_utf8_lossy(&plan.stdout).contains("inspect-repo"));

        let status = ProcessCommand::new(&bin)
            .arg("--project-root")
            .arg(temp.path())
            .arg("--database-url")
            .arg(&db_url)
            .env("XDG_CONFIG_HOME", temp.path().join("config"))
            .arg("status")
            .output()
            .unwrap();
        assert!(
            status.status.success(),
            "{}",
            String::from_utf8_lossy(&status.stderr)
        );
        let status_stdout = String::from_utf8_lossy(&status.stdout);
        assert!(status_stdout.contains("add health endpoint"));
        assert!(status_stdout.contains("Selected candidate: candidate-1"));
        assert!(status_stdout.contains("Candidates evaluated:"));
        assert!(status_stdout.contains("Frontier size:"));

        let inspect = ProcessCommand::new(&bin)
            .arg("--project-root")
            .arg(temp.path())
            .arg("--database-url")
            .arg(&db_url)
            .env("XDG_CONFIG_HOME", temp.path().join("config"))
            .arg("inspect")
            .output()
            .unwrap();
        assert!(
            inspect.status.success(),
            "{}",
            String::from_utf8_lossy(&inspect.stderr)
        );
        let inspect_stdout = String::from_utf8_lossy(&inspect.stdout);
        assert!(inspect_stdout.contains("Selected champion: candidate-1"));
        assert!(inspect_stdout.contains("Retained candidates:"));
        assert!(inspect_stdout.contains("- candidate-1 [selected]:"));
        assert!(inspect_stdout.contains("score: verifier="));
        assert!(inspect_stdout.contains("descriptor: cost="));

        let run = ProcessCommand::new(&bin)
            .arg("--project-root")
            .arg(temp.path())
            .arg("--database-url")
            .arg(&db_url)
            .env("XDG_CONFIG_HOME", temp.path().join("config"))
            .args(["run", "inspect-repo"])
            .output()
            .unwrap();
        assert!(
            run.status.success(),
            "{}",
            String::from_utf8_lossy(&run.stderr)
        );
        assert!(String::from_utf8_lossy(&run.stdout).contains("mock-acp:complete"));
    }
}
