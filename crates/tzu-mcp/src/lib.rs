use std::path::PathBuf;

use rmcp::{
    tool, tool_router,
    handler::server::wrapper::Parameters,
    ServiceExt, transport::stdio,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use tzu_runner::{RunnerActorHandle, TzuRunner};
use tzu_core::{DomainKind, inspect_goal_prompt};

#[derive(Clone)]
pub struct TzuMcpServer {
    handle: RunnerActorHandle,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct TzuPlanParams {
    pub goal: String,
    #[serde(default)]
    pub domain: Option<String>,
    #[serde(default)]
    pub context_roots: Option<Vec<String>>,
    #[serde(default)]
    pub include_nested_contexts: Option<bool>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct TzuRunParams {
    pub task_id: String,
    #[serde(default)]
    pub mode: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct TzuInspectPromptParams {
    pub goal: String,
    #[serde(default)]
    pub domain: Option<String>,
}

fn json_ok<T: Serialize>(value: &T) -> String {
    serde_json::to_string_pretty(value).unwrap_or_else(|e| {
        format!(r#"{{"error":"serialization failed: {e}"}}"#)
    })
}

fn json_error(error: &dyn std::fmt::Display) -> String {
    format!(r#"{{"error":"{}"}}"#, error.to_string().replace('"', r#"\""#))
}

#[tool_router(server_handler)]
impl TzuMcpServer {
    #[tool(description = "Initialize tzu project state in the project root directory")]
    async fn tzu_init(&self) -> String {
        match self.handle.init().await {
            Ok(state) => json_ok(&state),
            Err(e) => json_error(&e),
        }
    }

    #[tool(description = "Validate a goal prompt before planning; returns findings and improvement suggestions")]
    async fn tzu_inspect_prompt(
        &self,
        Parameters(params): Parameters<TzuInspectPromptParams>,
    ) -> String {
        let domain = match params.domain.as_deref() {
            Some("coding") => DomainKind::Coding,
            _ => DomainKind::Generic,
        };
        let inspection = inspect_goal_prompt(&params.goal, domain);
        json_ok(&inspection)
    }

    #[tool(description = "Create a structured plan from a goal")]
    async fn tzu_plan(
        &self,
        Parameters(params): Parameters<TzuPlanParams>,
    ) -> String {
        let domain = match params.domain.as_deref() {
            Some("coding") => tzu_runner::PlanningDomain::Coding,
            _ => tzu_runner::PlanningDomain::Generic,
        };
        let context_roots: Vec<PathBuf> = params
            .context_roots
            .unwrap_or_default()
            .into_iter()
            .map(PathBuf::from)
            .collect();
        let include_nested = params.include_nested_contexts.unwrap_or(false);
        match self
            .handle
            .create_plan(tzu_runner::CreatePlan {
                goal: params.goal,
                planning_goal: None,
                domain,
                context_roots,
                include_nested_contexts: include_nested,
            })
            .await
        {
            Ok(state) => json_ok(&state),
            Err(e) => json_error(&e),
        }
    }

    #[tool(description = "Show current plan, tasks, and run reports")]
    async fn tzu_status(&self) -> String {
        match self.handle.status().await {
            Ok(state) => json_ok(&state),
            Err(e) => json_error(&e),
        }
    }

    #[tool(description = "Show frontier selection details: retained candidates, scores, descriptors, and discard reasons")]
    async fn tzu_inspect(&self) -> String {
        match self.handle.status().await {
            Ok(state) => {
                let Some(plan) = &state.current_plan else {
                    return r#"{"message":"no current plan"}"#.to_string();
                };
                let Some(harness) = &plan.harness else {
                    return format!(r#"{{"plan_id":"{}","message":"no harness metadata"}}"#, plan.id);
                };
                json_ok(&harness.frontier)
            }
            Err(e) => json_error(&e),
        }
    }

    #[tool(description = "Execute a task by ID using the configured ACP backend")]
    async fn tzu_run(
        &self,
        Parameters(params): Parameters<TzuRunParams>,
    ) -> String {
        let mode = match params.mode.as_deref() {
            Some("real") => tzu_runner::RunMode::Real,
            _ => tzu_runner::RunMode::Mock,
        };
        match self.handle.run_task(tzu_runner::RunTask {
            task_id: params.task_id,
            mode,
        }).await {
            Ok(report) => json_ok(&report),
            Err(e) => json_error(&e),
        }
    }

    #[tool(description = "Get the repository context summary (file tree, languages, git status)")]
    async fn tzu_context(&self) -> String {
        match self.handle.inspect_repo().await {
            Ok(state) => json_ok(&state),
            Err(e) => json_error(&e),
        }
    }
}

impl TzuMcpServer {
    pub async fn connect(
        project_root: PathBuf,
        database_url: &str,
    ) -> Result<Self, tzu_runner::RunnerError> {
        let runner = TzuRunner::connect(&project_root, database_url).await?;
        let handle = runner.actor();
        Ok(Self { handle })
    }

    pub async fn serve_stdio(self) -> anyhow::Result<()> {
        let service = self
            .serve(stdio())
            .await
            .map_err(|e| anyhow::anyhow!("mcp server: {e}"))?;
        service
            .waiting()
            .await
            .map_err(|e| anyhow::anyhow!("mcp server: {e}"))?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rmcp::ClientHandler;
    use rmcp::model::{CallToolRequestParams, ClientInfo};

    #[derive(Debug, Clone, Default)]
    struct DummyClientHandler;

    impl ClientHandler for DummyClientHandler {
        fn get_info(&self) -> ClientInfo {
            ClientInfo::default()
        }
    }

    fn setup_project() -> tempfile::TempDir {
        let temp = tempfile::tempdir().unwrap();
        std::process::Command::new("git")
            .args(["init", "-b", "main"])
            .current_dir(temp.path())
            .output()
            .unwrap();
        temp
    }

    async fn connect_server(temp: &tempfile::TempDir) -> TzuMcpServer {
        let db = temp.path().join("state.sqlite");
        let db_url = format!("sqlite://{}", db.display());
        TzuMcpServer::connect(temp.path().to_path_buf(), &db_url)
            .await
            .unwrap()
    }

    #[tokio::test]
    async fn mcp_tzu_inspect_prompt_empty_goal() {
        let temp = setup_project();
        let server = connect_server(&temp).await;
        let (server_transport, client_transport) = tokio::io::duplex(4096);
        let server_handle = tokio::spawn(async move {
            server.serve(server_transport).await?.waiting().await?;
            anyhow::Ok(())
        });
        let client = DummyClientHandler::default()
            .serve(client_transport)
            .await
            .unwrap();

        let result = client
            .call_tool(
                CallToolRequestParams::new("tzu_inspect_prompt").with_arguments(
                    serde_json::json!({"goal": "", "domain": "generic"})
                        .as_object()
                        .unwrap()
                        .clone(),
                ),
            )
            .await
            .unwrap();
        let text = result
            .content
            .first()
            .and_then(|c| c.raw.as_text())
            .map(|t| t.text.as_str())
            .expect("Expected text content");
        assert!(text.contains("empty-goal"), "expected empty-goal finding in: {text}");

        client.cancel().await.unwrap();
        server_handle.await.unwrap().unwrap();
    }

    #[tokio::test]
    async fn mcp_tzu_inspect_prompt_valid_goal() {
        let temp = setup_project();
        let server = connect_server(&temp).await;
        let (server_transport, client_transport) = tokio::io::duplex(4096);
        let server_handle = tokio::spawn(async move {
            server.serve(server_transport).await?.waiting().await?;
            anyhow::Ok(())
        });
        let client = DummyClientHandler::default()
            .serve(client_transport)
            .await
            .unwrap();

        let result = client
            .call_tool(
                CallToolRequestParams::new("tzu_inspect_prompt").with_arguments(
                    serde_json::json!({"goal": "add health endpoint", "domain": "coding"})
                        .as_object()
                        .unwrap()
                        .clone(),
                ),
            )
            .await
            .unwrap();
        let text = result
            .content
            .first()
            .and_then(|c| c.raw.as_text())
            .map(|t| t.text.as_str())
            .expect("Expected text content");
        assert!(text.contains("acceptable"), "expected acceptable status in: {text}");

        client.cancel().await.unwrap();
        server_handle.await.unwrap().unwrap();
    }

    #[tokio::test]
    async fn mcp_tzu_init_and_status() {
        let temp = setup_project();
        let server = connect_server(&temp).await;
        let (server_transport, client_transport) = tokio::io::duplex(4096);
        let server_handle = tokio::spawn(async move {
            server.serve(server_transport).await?.waiting().await?;
            anyhow::Ok(())
        });
        let client = DummyClientHandler::default()
            .serve(client_transport)
            .await
            .unwrap();

        let init_result = client
            .call_tool(CallToolRequestParams::new("tzu_init"))
            .await
            .unwrap();
        let init_text = init_result
            .content
            .first()
            .and_then(|c| c.raw.as_text())
            .map(|t| t.text.as_str())
            .expect("Expected text content");
        assert!(init_text.contains("project_root"), "expected project_root in: {init_text}");

        let status_result = client
            .call_tool(CallToolRequestParams::new("tzu_status"))
            .await
            .unwrap();
        let status_text = status_result
            .content
            .first()
            .and_then(|c| c.raw.as_text())
            .map(|t| t.text.as_str())
            .expect("Expected text content");
        assert!(status_text.contains("project_root"), "expected project_root in: {status_text}");

        client.cancel().await.unwrap();
        server_handle.await.unwrap().unwrap();
    }
}
