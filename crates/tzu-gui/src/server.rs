use std::net::{IpAddr, SocketAddr};
use std::path::PathBuf;

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use leptos::config::LeptosOptions;
use serde::{Deserialize, Serialize};
use tzu_core::ProjectState;
use tzu_repo::RepoState;
use tzu_runner::{
    PlanningDomain, RunMode, RunnerActorHandle, RunnerError, TzuRunReport, TzuRunner,
    default_database_url,
};

use crate::app::shell;

#[derive(Clone)]
pub struct GuiState {
    runner: RunnerActorHandle,
}

impl GuiState {
    #[must_use]
    pub fn new(runner: RunnerActorHandle) -> Self {
        Self { runner }
    }
}

#[derive(Debug, Clone)]
pub struct GuiConfig {
    pub host: IpAddr,
    pub port: u16,
    pub project_root: PathBuf,
    pub database_url: Option<String>,
}

impl GuiConfig {
    #[must_use]
    pub fn addr(&self) -> SocketAddr {
        SocketAddr::new(self.host, self.port)
    }
}

pub async fn build_state(config: &GuiConfig) -> Result<GuiState, RunnerError> {
    let root = config
        .project_root
        .canonicalize()
        .unwrap_or_else(|_| config.project_root.clone());
    let database_url = config
        .database_url
        .clone()
        .unwrap_or_else(|| default_database_url(&root));
    let runner = TzuRunner::connect(root, &database_url).await?;
    Ok(GuiState::new(runner.actor()))
}

pub fn router(state: GuiState, options: LeptosOptions) -> Router {
    let _ = any_spawner::Executor::init_tokio();
    let app_options = options.clone();
    Router::new()
        .route("/api/health", get(health))
        .route("/api/state", get(state_snapshot))
        .route("/api/init", post(init))
        .route("/api/plans", post(create_plan))
        .route("/api/tasks/{task_id}/run", post(run_task))
        .route("/api/repo", get(repo_state))
        .route("/pkg/tzu-gui.css", get(style_css))
        .route("/static/app.js", get(app_js))
        .fallback(leptos_axum::render_app_to_stream(move || {
            shell(app_options.clone())
        }))
        .with_state(state)
}

async fn health(State(state): State<GuiState>) -> Result<Json<HealthResponse>, ApiError> {
    let project = state.runner.status().await?.project_root;
    Ok(Json(HealthResponse {
        status: "ok",
        project_root: project,
    }))
}

async fn state_snapshot(State(state): State<GuiState>) -> Result<Json<ProjectState>, ApiError> {
    Ok(Json(state.runner.status().await?))
}

async fn init(State(state): State<GuiState>) -> Result<Json<ProjectState>, ApiError> {
    Ok(Json(state.runner.init().await?))
}

async fn create_plan(
    State(state): State<GuiState>,
    Json(request): Json<CreatePlanRequest>,
) -> Result<Json<ProjectState>, ApiError> {
    let plan = state
        .runner
        .create_plan(tzu_runner::CreatePlan {
            goal: request.goal,
            domain: request.domain,
        })
        .await?;
    Ok(Json(plan))
}

async fn run_task(
    State(state): State<GuiState>,
    Path(task_id): Path<String>,
    Json(request): Json<RunTaskRequest>,
) -> Result<Json<TzuRunReport>, ApiError> {
    let report = state
        .runner
        .run_task(tzu_runner::RunTask {
            task_id,
            mode: request.mode,
        })
        .await?;
    Ok(Json(report))
}

async fn repo_state(State(state): State<GuiState>) -> Result<Json<RepoState>, ApiError> {
    Ok(Json(state.runner.inspect_repo().await?))
}

async fn style_css() -> impl IntoResponse {
    (
        StatusCode::OK,
        [("content-type", "text/css")],
        include_str!("../style/main.css"),
    )
}

async fn app_js() -> impl IntoResponse {
    (
        StatusCode::OK,
        [("content-type", "application/javascript")],
        include_str!("../public/static/app.js"),
    )
}

#[derive(Debug, Serialize)]
struct HealthResponse {
    status: &'static str,
    project_root: String,
}

#[derive(Debug, Deserialize)]
struct CreatePlanRequest {
    goal: String,
    domain: PlanningDomain,
}

#[derive(Debug, Deserialize)]
struct RunTaskRequest {
    mode: RunMode,
}

#[derive(Debug, Serialize)]
struct ErrorBody {
    error: String,
    kind: &'static str,
}

struct ApiError(RunnerError);

impl From<RunnerError> for ApiError {
    fn from(value: RunnerError) -> Self {
        Self(value)
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let status = match self.0 {
            RunnerError::MissingPlan | RunnerError::MissingTask(_) => StatusCode::NOT_FOUND,
            RunnerError::DatabaseUnavailable { .. } => StatusCode::SERVICE_UNAVAILABLE,
            _ => StatusCode::INTERNAL_SERVER_ERROR,
        };
        let body = ErrorBody {
            error: self.0.to_string(),
            kind: "runner-error",
        };
        (status, Json(body)).into_response()
    }
}

#[cfg(test)]
mod tests {
    use axum::body::Body;
    use axum::http::{Method, Request, StatusCode};
    use http_body_util::BodyExt;
    use tower::ServiceExt;

    use super::*;

    async fn test_router() -> Router {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.keep();
        let db = root.join("state.sqlite");
        let url = format!("sqlite://{}", db.display());
        let runner = TzuRunner::connect(&root, &url).await.unwrap();
        let options = LeptosOptions::builder()
            .output_name("tzu-gui".to_string())
            .build();
        router(GuiState::new(runner.actor()), options)
    }

    #[tokio::test]
    async fn health_endpoint_reports_project() {
        let app = test_router().await;
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/api/health")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn plan_and_run_endpoints_mutate_state() {
        let app = test_router().await;
        let init = app
            .clone()
            .oneshot(
                Request::builder()
                    .method(Method::POST)
                    .uri("/api/init")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(init.status(), StatusCode::OK);

        let plan = app
            .clone()
            .oneshot(
                Request::builder()
                    .method(Method::POST)
                    .uri("/api/plans")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        r#"{"goal":"add health endpoint","domain":"generic"}"#,
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(plan.status(), StatusCode::OK);
        let bytes = plan.into_body().collect().await.unwrap().to_bytes();
        let body = String::from_utf8_lossy(&bytes);
        assert!(body.contains(r#""frontier""#));
        assert!(body.contains(r#""retained_candidate_ids""#));
        assert!(body.contains(r#""selected_candidate_id":"candidate-1""#));

        let run = app
            .oneshot(
                Request::builder()
                    .method(Method::POST)
                    .uri("/api/tasks/ground-inputs/run")
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"mode":"mock"}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(run.status(), StatusCode::OK);

        let bytes = run.into_body().collect().await.unwrap().to_bytes();
        assert!(String::from_utf8_lossy(&bytes).contains("mock-acp:complete"));
    }
}
