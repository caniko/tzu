use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::net::{IpAddr, SocketAddr};
use std::path::{Path as FsPath, PathBuf};

use anyhow::Context;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use leptos::config::LeptosOptions;
use serde::{Deserialize, Serialize};
use tzu_config::{
    ConfigError, DiscoveredProject, TzuConfig, config_path_with_env, default_config_path,
    discover_projects, load_config, load_config_with_env,
};
use tzu_core::{PlanError, ProjectState, PromptInspection};
use tzu_repo::RepoState;
use tzu_runner::{
    PlanningDomain, RunMode, RunnerActorHandle, RunnerError, TzuRunReport, TzuRunner,
    default_database_url,
};

use crate::app::shell;

#[derive(Clone)]
pub struct GuiState {
    runner: RunnerActorHandle,
    config_source: ConfigSource,
}

#[derive(Clone)]
enum ConfigSource {
    Environment,
    EnvironmentMap(BTreeMap<String, String>),
    Static {
        config: TzuConfig,
        discovered_projects: Vec<DiscoveredProject>,
    },
}

impl GuiState {
    #[must_use]
    pub fn new(runner: RunnerActorHandle) -> Self {
        Self {
            runner,
            config_source: ConfigSource::Environment,
        }
    }

    #[must_use]
    pub fn with_config(
        runner: RunnerActorHandle,
        config: TzuConfig,
        discovered_projects: Vec<DiscoveredProject>,
    ) -> Self {
        Self {
            runner,
            config_source: ConfigSource::Static {
                config,
                discovered_projects,
            },
        }
    }

    #[must_use]
    pub fn with_config_env(
        runner: RunnerActorHandle,
        config_env: BTreeMap<String, String>,
    ) -> Self {
        Self {
            runner,
            config_source: ConfigSource::EnvironmentMap(config_env),
        }
    }

    fn load_config_snapshot(&self) -> Result<LoadedConfigSnapshot, ConfigError> {
        match &self.config_source {
            ConfigSource::Environment => {
                let config = load_config()?;
                let discovered_projects = discover_projects(&config)?;
                Ok(LoadedConfigSnapshot {
                    config_path: default_config_path(),
                    config,
                    discovered_projects,
                })
            }
            ConfigSource::EnvironmentMap(env) => {
                let config = load_config_with_env(env)?;
                let discovered_projects = discover_projects(&config)?;
                Ok(LoadedConfigSnapshot {
                    config_path: config_path_with_env(env),
                    config,
                    discovered_projects,
                })
            }
            ConfigSource::Static {
                config,
                discovered_projects,
            } => Ok(LoadedConfigSnapshot {
                config_path: default_config_path(),
                config: config.clone(),
                discovered_projects: discovered_projects.clone(),
            }),
        }
    }
}

struct LoadedConfigSnapshot {
    config_path: Option<PathBuf>,
    config: TzuConfig,
    discovered_projects: Vec<DiscoveredProject>,
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

pub async fn build_state(config: &GuiConfig) -> anyhow::Result<GuiState> {
    let app_config = load_config().context("load tzu config")?;
    discover_projects(&app_config).context("discover configured projects")?;
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
        .route("/api/config", get(config_snapshot))
        .route("/static/tzu-arena/tzu_arena.js", get(arena_js))
        .route("/static/tzu-arena/tzu_arena_bg.wasm", get(arena_wasm))
        .route("/api/context-references", get(context_references))
        .route("/api/context-roots/resolve", post(resolve_context_roots))
        .route("/api/projects", get(projects_snapshot))
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

async fn config_snapshot(
    State(state): State<GuiState>,
) -> Result<Json<GuiConfigSnapshot>, ApiError> {
    let snapshot = state.load_config_snapshot()?;
    Ok(Json(GuiConfigSnapshot {
        config_path: snapshot.config_path,
        config: snapshot.config,
        discovered_projects: snapshot.discovered_projects,
    }))
}

async fn projects_snapshot(
    State(state): State<GuiState>,
) -> Result<Json<Vec<DiscoveredProject>>, ApiError> {
    let snapshot = state.load_config_snapshot()?;
    Ok(Json(snapshot.discovered_projects))
}

async fn init(State(state): State<GuiState>) -> Result<Json<ProjectState>, ApiError> {
    Ok(Json(state.runner.init().await?))
}

async fn create_plan(
    State(state): State<GuiState>,
    Json(request): Json<CreatePlanRequest>,
) -> Result<Json<ProjectState>, ApiError> {
    let display_goal = request
        .goal_display
        .as_deref()
        .or(request.goal.as_deref())
        .unwrap_or_default()
        .trim()
        .to_string();
    let planning_goal = request
        .goal_raw
        .as_deref()
        .or(request.goal.as_deref())
        .unwrap_or(display_goal.as_str())
        .trim()
        .to_string();
    let snapshot = state.load_config_snapshot()?;
    let include_nested_contexts =
        request.include_nested_contexts || snapshot.config.include_nested_contexts;
    let context_roots = validate_context_roots(request.context_roots)?;
    let plan = state
        .runner
        .create_plan(tzu_runner::CreatePlan {
            goal: display_goal.clone(),
            planning_goal: (planning_goal != display_goal).then_some(planning_goal),
            domain: request.domain,
            context_roots,
            include_nested_contexts,
        })
        .await?;
    Ok(Json(plan))
}

async fn context_references(
    State(state): State<GuiState>,
) -> Result<Json<Vec<ContextReference>>, ApiError> {
    let snapshot = state.load_config_snapshot()?;
    Ok(Json(discover_context_references(
        &snapshot.discovered_projects,
    )))
}

async fn resolve_context_roots(
    Json(request): Json<ResolveContextRootsRequest>,
) -> Json<ResolveContextRootsResponse> {
    Json(ResolveContextRootsResponse {
        results: request
            .paths
            .into_iter()
            .map(|path| resolve_context_root(&path))
            .collect(),
    })
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

async fn arena_js() -> impl IntoResponse {
    (
        StatusCode::OK,
        [("content-type", "application/javascript")],
        include_str!("../public/static/tzu-arena/tzu_arena.js"),
    )
}

async fn arena_wasm() -> impl IntoResponse {
    (
        StatusCode::OK,
        [("content-type", "application/wasm")],
        include_bytes!("../public/static/tzu-arena/tzu_arena_bg.wasm").as_slice(),
    )
}

#[derive(Debug, Serialize)]
struct HealthResponse {
    status: &'static str,
    project_root: String,
}

#[derive(Debug, Serialize)]
struct GuiConfigSnapshot {
    config_path: Option<PathBuf>,
    config: TzuConfig,
    discovered_projects: Vec<DiscoveredProject>,
}

#[derive(Debug, Serialize)]
struct ContextReference {
    label: String,
    display: String,
    raw: String,
    path: PathBuf,
    relative_path: PathBuf,
}

#[derive(Debug, Deserialize)]
struct ResolveContextRootsRequest {
    #[serde(default)]
    paths: Vec<String>,
}

#[derive(Debug, Serialize)]
struct ResolveContextRootsResponse {
    results: Vec<ContextRootResolution>,
}

#[derive(Debug, Serialize)]
struct ContextRootResolution {
    input: String,
    ok: bool,
    path: Option<PathBuf>,
    error: Option<String>,
}

#[derive(Debug, Deserialize)]
struct CreatePlanRequest {
    #[serde(default)]
    goal: Option<String>,
    #[serde(default)]
    goal_display: Option<String>,
    #[serde(default)]
    goal_raw: Option<String>,
    domain: PlanningDomain,
    #[serde(default)]
    context_roots: Vec<String>,
    #[serde(default)]
    include_nested_contexts: bool,
}

#[derive(Debug, Deserialize)]
struct RunTaskRequest {
    mode: RunMode,
}

#[derive(Debug, Serialize)]
struct ErrorBody {
    error: String,
    kind: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    prompt_inspection: Option<PromptInspection>,
}

#[derive(Debug)]
enum ApiError {
    Runner(RunnerError),
    Config(ConfigError),
    BadRequest(String),
}

impl From<RunnerError> for ApiError {
    fn from(value: RunnerError) -> Self {
        Self::Runner(value)
    }
}

impl From<std::io::Error> for ApiError {
    fn from(value: std::io::Error) -> Self {
        Self::BadRequest(value.to_string())
    }
}

impl From<ConfigError> for ApiError {
    fn from(value: ConfigError) -> Self {
        Self::Config(value)
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let (status, error, kind, prompt_inspection) = match self {
            Self::Runner(error) => {
                if let RunnerError::Planning(PlanError::PromptNeedsImprovement(inspection)) = error
                {
                    (
                        StatusCode::UNPROCESSABLE_ENTITY,
                        format!("goal prompt needs improvement: {inspection}"),
                        "prompt-inspection",
                        Some(*inspection),
                    )
                } else {
                    let status = match error {
                        RunnerError::MissingPlan | RunnerError::MissingTask(_) => {
                            StatusCode::NOT_FOUND
                        }
                        RunnerError::DatabaseUnavailable { .. } => StatusCode::SERVICE_UNAVAILABLE,
                        _ => StatusCode::INTERNAL_SERVER_ERROR,
                    };
                    (status, error.to_string(), "runner-error", None)
                }
            }
            Self::Config(error) => (
                StatusCode::INTERNAL_SERVER_ERROR,
                error.to_string(),
                "config-error",
                None,
            ),
            Self::BadRequest(error) => (StatusCode::BAD_REQUEST, error, "bad-request", None),
        };
        let body = ErrorBody {
            error,
            kind,
            prompt_inspection,
        };
        (status, Json(body)).into_response()
    }
}

fn validate_context_roots(roots: Vec<String>) -> Result<Vec<PathBuf>, ApiError> {
    let mut seen = BTreeSet::new();
    let mut validated = Vec::new();
    for root in roots.into_iter().filter(|root| !root.trim().is_empty()) {
        let resolution = resolve_context_root(&root);
        let canonical = resolution.path.ok_or_else(|| {
            ApiError::BadRequest(
                resolution
                    .error
                    .unwrap_or_else(|| format!("context root `{}` is unavailable", root.trim())),
            )
        })?;
        if seen.insert(canonical.clone()) {
            validated.push(canonical);
        }
    }
    Ok(validated)
}

fn resolve_context_root(root: &str) -> ContextRootResolution {
    let input = root.trim().to_string();
    let path = PathBuf::from(&input);
    if input.is_empty() {
        return ContextRootResolution {
            input,
            ok: false,
            path: None,
            error: Some("context path must not be empty".to_string()),
        };
    }
    if !path.is_absolute() {
        return ContextRootResolution {
            input,
            ok: false,
            path: None,
            error: Some(format!(
                "context path `{}` must be an absolute path",
                path.display()
            )),
        };
    }
    let canonical = match path.canonicalize() {
        Ok(canonical) => canonical,
        Err(err) => {
            return ContextRootResolution {
                input,
                ok: false,
                path: None,
                error: Some(format!(
                    "context path `{}` is unavailable: {err}",
                    path.display()
                )),
            };
        }
    };
    match fs::metadata(&canonical) {
        Ok(metadata) if metadata.is_dir() => ContextRootResolution {
            input,
            ok: true,
            path: Some(canonical),
            error: None,
        },
        Ok(_) => ContextRootResolution {
            input,
            ok: false,
            path: None,
            error: Some(format!(
                "context path `{}` is not a directory",
                canonical.display()
            )),
        },
        Err(err) => ContextRootResolution {
            input,
            ok: false,
            path: None,
            error: Some(format!(
                "context path `{}` is unavailable: {err}",
                canonical.display()
            )),
        },
    }
}

fn discover_context_references(projects: &[DiscoveredProject]) -> Vec<ContextReference> {
    let mut name_counts = BTreeMap::<String, usize>::new();
    for project in projects {
        *name_counts.entry(project.name.clone()).or_default() += 1;
    }

    projects
        .iter()
        .map(|project| {
            let display_label = if name_counts.get(&project.name).copied().unwrap_or_default() > 1 {
                disambiguated_project_label(&project.path, &project.name)
            } else {
                project.name.clone()
            };
            let path = project.path.clone();
            ContextReference {
                label: project.name.clone(),
                display: format!("@{display_label}"),
                raw: format!("@{}", path.display()),
                path,
                relative_path: PathBuf::from(&project.name),
            }
        })
        .collect()
}

fn disambiguated_project_label(path: &FsPath, name: &str) -> String {
    path.parent()
        .and_then(FsPath::file_name)
        .and_then(|value| value.to_str())
        .map(|parent| format!("{parent}/{name}"))
        .unwrap_or_else(|| path.display().to_string())
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
        router(
            GuiState::with_config(runner.actor(), TzuConfig::default(), Vec::new()),
            options,
        )
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
    async fn shell_contains_error_dialog_controls() {
        let app = test_router().await;
        let response = app
            .oneshot(Request::builder().uri("/").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let bytes = response.into_body().collect().await.unwrap().to_bytes();
        let body = String::from_utf8_lossy(&bytes);
        assert!(body.contains(r#"id="error-dialog""#));
        assert!(body.contains(r#"role="dialog""#));
        assert!(body.contains(r#"id="error-dialog-explainer""#));
        assert!(body.contains(r#"id="error-dialog-logs""#));
        assert!(body.contains(r#"id="error-dialog-close""#));
        assert!(body.contains(r#"id="settings-btn""#));
        assert!(body.contains(r#"id="settings-dialog""#));
        assert!(body.contains(r#"id="goal-input""#));
        assert!(body.contains(r#"contenteditable="true""#));
        assert!(body.contains(r#"id="goal-value""#));
        assert!(body.contains(r#"id="mention-suggestion""#));
        assert!(body.contains(r#"id="toast-region""#));
        assert!(body.contains(r#"name="domain""#));
        assert!(!body.contains(r#"id="context-roots-input""#));
        assert!(!body.contains(r#"id="include-nested-contexts""#));
        assert!(!body.contains(r#"<span>"Project"</span>"#));
    }

    #[test]
    fn context_reference_discovery_uses_discovered_project_roots_only() {
        let temp = tempfile::tempdir().unwrap();
        let projects_root = temp.path().join("projects");
        let active_root = temp.path().join("active");
        let regicide = projects_root.join("regicide");
        std::fs::create_dir_all(&regicide).unwrap();
        std::fs::create_dir_all(active_root.join("crates")).unwrap();
        std::fs::write(regicide.join("Cargo.toml"), "[package]\nname='regicide'\n").unwrap();
        let config = TzuConfig {
            projects_directory: Some(projects_root),
            projects_directories: Vec::new(),
            include_nested_contexts: false,
            gui: Default::default(),
        };
        let projects = discover_projects(&config).unwrap();

        let references = discover_context_references(&projects);
        let displays = references
            .iter()
            .map(|reference| reference.display.as_str())
            .collect::<Vec<_>>();

        assert!(displays.contains(&"@regicide"));
        assert!(!displays.contains(&"@crates"));
        assert_eq!(references[0].path, regicide.canonicalize().unwrap());
        assert_eq!(references[0].raw, format!("@{}", regicide.display()));
    }

    #[test]
    fn duplicate_context_reference_names_are_disambiguated_but_match_by_basename() {
        let first = DiscoveredProject {
            name: "regicide".to_string(),
            path: PathBuf::from("/projects/games/regicide"),
            markers: vec![".git".to_string()],
        };
        let second = DiscoveredProject {
            name: "regicide".to_string(),
            path: PathBuf::from("/projects/archive/regicide"),
            markers: vec![".git".to_string()],
        };

        let references = discover_context_references(&[first, second]);

        assert_eq!(references[0].label, "regicide");
        assert_eq!(references[0].display, "@games/regicide");
        assert_eq!(references[1].label, "regicide");
        assert_eq!(references[1].display, "@archive/regicide");
    }

    #[test]
    fn context_roots_are_canonicalized_deduplicated_and_validated() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("repo");
        std::fs::create_dir_all(&root).unwrap();
        let file = temp.path().join("file.txt");
        std::fs::write(&file, "not a directory").unwrap();

        let roots =
            validate_context_roots(vec![root.display().to_string(), root.display().to_string()])
                .unwrap();
        assert_eq!(roots, vec![root.canonicalize().unwrap()]);

        let file_error = validate_context_roots(vec![file.display().to_string()]).unwrap_err();
        let response = file_error.into_response();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);

        let missing_error =
            validate_context_roots(vec![temp.path().join("missing").display().to_string()])
                .unwrap_err();
        let response = missing_error.into_response();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);

        let relative_error = validate_context_roots(vec!["relative/path".to_string()]).unwrap_err();
        let response = relative_error.into_response();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn context_root_resolver_reports_per_path_results() {
        let app = test_router().await;
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("repo");
        let missing = temp.path().join("missing");
        let file = temp.path().join("file.txt");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(&file, "not a directory").unwrap();

        let body = serde_json::json!({
            "paths": [
                root.display().to_string(),
                missing.display().to_string(),
                "relative/path",
                file.display().to_string(),
            ],
        });
        let response = app
            .oneshot(
                Request::builder()
                    .method(Method::POST)
                    .uri("/api/context-roots/resolve")
                    .header("content-type", "application/json")
                    .body(Body::from(body.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let bytes = response.into_body().collect().await.unwrap().to_bytes();
        let body = String::from_utf8_lossy(&bytes);
        assert!(body.contains(r#""ok":true"#));
        assert!(body.contains(&root.canonicalize().unwrap().display().to_string()));
        assert!(body.contains(r#""ok":false"#));
        assert!(body.contains("is unavailable"));
        assert!(body.contains("must be an absolute path"));
        assert!(body.contains("is not a directory"));
    }

    #[tokio::test]
    async fn config_and_projects_endpoints_report_discovery() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("state");
        std::fs::create_dir_all(&root).unwrap();
        let projects_root = temp.path().join("projects");
        let project = projects_root.join("alpha");
        std::fs::create_dir_all(&project).unwrap();
        std::fs::write(project.join("Cargo.toml"), "[package]\nname='alpha'\n").unwrap();
        let db = root.join("state.sqlite");
        let url = format!("sqlite://{}", db.display());
        let runner = TzuRunner::connect(&root, &url).await.unwrap();
        let options = LeptosOptions::builder()
            .output_name("tzu-gui".to_string())
            .build();
        let config = TzuConfig {
            projects_directory: Some(projects_root.clone()),
            projects_directories: Vec::new(),
            include_nested_contexts: true,
            gui: Default::default(),
        };
        let projects = discover_projects(&config).unwrap();
        let app = router(
            GuiState::with_config(runner.actor(), config, projects),
            options,
        );

        let config_response = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/api/config")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(config_response.status(), StatusCode::OK);
        let bytes = config_response
            .into_body()
            .collect()
            .await
            .unwrap()
            .to_bytes();
        let body = String::from_utf8_lossy(&bytes);
        assert!(body.contains(r#""include_nested_contexts":true"#));
        assert!(body.contains(r#""discovered_projects""#));

        let projects_response = app
            .oneshot(
                Request::builder()
                    .uri("/api/projects")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(projects_response.status(), StatusCode::OK);
        let bytes = projects_response
            .into_body()
            .collect()
            .await
            .unwrap()
            .to_bytes();
        let body = String::from_utf8_lossy(&bytes);
        assert!(body.contains(r#""name":"alpha""#));
        assert!(body.contains("Cargo.toml"));
    }

    #[tokio::test]
    async fn config_endpoints_reload_config_between_requests() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("state");
        std::fs::create_dir_all(&root).unwrap();
        let config_home = temp.path().join("config");
        let config_dir = config_home.join("tzu");
        std::fs::create_dir_all(&config_dir).unwrap();
        let projects_a = temp.path().join("projects-a");
        let projects_b = temp.path().join("projects-b");
        let alpha = projects_a.join("alpha");
        let beta = projects_b.join("beta");
        std::fs::create_dir_all(&alpha).unwrap();
        std::fs::create_dir_all(&beta).unwrap();
        std::fs::write(alpha.join("Cargo.toml"), "[package]\nname='alpha'\n").unwrap();
        std::fs::write(beta.join("Cargo.toml"), "[package]\nname='beta'\n").unwrap();
        std::fs::write(
            config_dir.join("config.toml"),
            format!(
                r#"
projects_directory = "{}"
include_nested_contexts = false
"#,
                projects_a.display()
            ),
        )
        .unwrap();
        let db = root.join("state.sqlite");
        let url = format!("sqlite://{}", db.display());
        let runner = TzuRunner::connect(&root, &url).await.unwrap();
        let options = LeptosOptions::builder()
            .output_name("tzu-gui".to_string())
            .build();
        let env = BTreeMap::from([(
            "XDG_CONFIG_HOME".to_string(),
            config_home.display().to_string(),
        )]);
        let app = router(GuiState::with_config_env(runner.actor(), env), options);

        let first_response = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/api/config")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(first_response.status(), StatusCode::OK);
        let first_body = String::from_utf8_lossy(
            &first_response
                .into_body()
                .collect()
                .await
                .unwrap()
                .to_bytes(),
        )
        .to_string();
        assert!(first_body.contains(r#""include_nested_contexts":false"#));
        assert!(first_body.contains(r#""name":"alpha""#));
        assert!(!first_body.contains(r#""name":"beta""#));

        std::fs::write(
            config_dir.join("config.toml"),
            format!(
                r#"
projects_directory = "{}"
include_nested_contexts = true
"#,
                projects_b.display()
            ),
        )
        .unwrap();

        let second_response = app
            .oneshot(
                Request::builder()
                    .uri("/api/config")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(second_response.status(), StatusCode::OK);
        let second_body = String::from_utf8_lossy(
            &second_response
                .into_body()
                .collect()
                .await
                .unwrap()
                .to_bytes(),
        )
        .to_string();
        assert!(second_body.contains(r#""include_nested_contexts":true"#));
        assert!(second_body.contains(r#""name":"beta""#));
        assert!(!second_body.contains(r#""name":"alpha""#));
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
                        r#"{"goal_display":"add health endpoint @fixture","goal_raw":"add health endpoint @/tmp","domain":"generic","context_roots":[]}"#,
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(plan.status(), StatusCode::OK);
        let bytes = plan.into_body().collect().await.unwrap().to_bytes();
        let body = String::from_utf8_lossy(&bytes);
        assert!(body.contains(r#""goal":"add health endpoint @fixture""#));
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

    #[tokio::test]
    async fn plan_endpoint_rejects_bad_goal_prompt_with_structured_suggestion() {
        let app = test_router().await;
        let response = app
            .oneshot(
                Request::builder()
                    .method(Method::POST)
                    .uri("/api/plans")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        r#"{"goal_display":"TODO","goal_raw":"TODO","domain":"generic","context_roots":[]}"#,
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
        let bytes = response.into_body().collect().await.unwrap().to_bytes();
        let body = String::from_utf8_lossy(&bytes);
        assert!(body.contains(r#""kind":"prompt-inspection""#));
        assert!(body.contains(r#""model":"gpt-5.5""#));
        assert!(body.contains(r#""reasoning_effort":"medium""#));
        assert!(body.contains(r#""code":"placeholder-goal""#));
    }
}
