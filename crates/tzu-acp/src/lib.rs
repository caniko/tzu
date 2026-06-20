use std::env;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use async_trait::async_trait;
use protocol::{
    ACP_PROTOCOL_VERSION, ClientInfo, InitializeParams, InitializeResult, JSONRPC_VERSION,
    JsonRpcError, JsonRpcErrorResponse, JsonRpcMessage, JsonRpcNotification, JsonRpcRequest,
    JsonRpcResponse, PromptContent, RequestId, SessionCloseParams, SessionNewParams,
    SessionNewResult, SessionPromptParams, SessionUpdateParams,
};
use serde::de::DeserializeOwned;
use serde_json::json;
use thiserror::Error;
use tokio::io::{AsyncBufRead, AsyncBufReadExt, AsyncWrite, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin, ChildStdout, Command};

pub mod protocol;

#[derive(Debug, Error)]
pub enum AcpError {
    #[error("spawn ACP agent: {0}")]
    Spawn(String),
    #[error("json-rpc transport: {0}")]
    Transport(String),
    #[error("json-rpc protocol error {code}: {message}")]
    Rpc {
        code: i64,
        message: String,
        data: Option<serde_json::Value>,
    },
    #[error("unexpected json-rpc message: {0:?}")]
    Unexpected(JsonRpcMessage),
    #[error("decode `{target}` result: {source}")]
    Decode {
        target: &'static str,
        source: serde_json::Error,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AcpAgentBackend {
    Codex,
    DeepSeek,
    OpenCode,
    Hermes,
}

impl AcpAgentBackend {
    #[must_use]
    pub fn from_env() -> Self {
        match env::var("TZU_AGENT_BACKEND").as_deref() {
            Ok("deepseek" | "deepseek-v4") => Self::DeepSeek,
            Ok("opencode") => Self::OpenCode,
            Ok("hermes") => Self::Hermes,
            _ => Self::Codex,
        }
    }

    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::Codex => "codex",
            Self::DeepSeek => "deepseek",
            Self::OpenCode => "opencode",
            Self::Hermes => "hermes",
        }
    }
}

#[derive(Debug, Clone)]
pub struct AcpAgentConfig {
    pub backend: AcpAgentBackend,
    pub binary: PathBuf,
    pub args: Vec<String>,
    pub cwd: PathBuf,
    pub timeout: Duration,
}

impl AcpAgentConfig {
    #[must_use]
    pub fn from_env(cwd: impl Into<PathBuf>) -> Self {
        let backend = AcpAgentBackend::from_env();
        match backend {
            AcpAgentBackend::Codex => Self {
                backend,
                binary: env::var_os("TZU_CODEX_ACP_BIN")
                    .map(PathBuf::from)
                    .unwrap_or_else(|| PathBuf::from("codex-acp")),
                args: Vec::new(),
                cwd: cwd.into(),
                timeout: Duration::from_secs(120),
            },
            AcpAgentBackend::DeepSeek => Self {
                backend,
                binary: env::var_os("TZU_DEEPSEEK_ACP_BIN")
                    .map(PathBuf::from)
                    .unwrap_or_else(|| PathBuf::from("deepseek-acp-adapter")),
                args: vec!["serve".to_string()],
                cwd: cwd.into(),
                timeout: Duration::from_secs(120),
            },
            AcpAgentBackend::OpenCode => Self {
                backend,
                binary: env::var_os("TZU_OPENCODE_ACP_BIN")
                    .map(PathBuf::from)
                    .unwrap_or_else(|| PathBuf::from("opencode")),
                args: vec!["acp".to_string()],
                cwd: cwd.into(),
                timeout: Duration::from_secs(120),
            },
            AcpAgentBackend::Hermes => Self {
                backend,
                binary: env::var_os("TZU_HERMES_ACP_BIN")
                    .map(PathBuf::from)
                    .unwrap_or_else(|| PathBuf::from("hermes")),
                args: vec!["acp".to_string()],
                cwd: cwd.into(),
                timeout: Duration::from_secs(120),
            },
        }
    }
}

pub type CodexAcpConfig = AcpAgentConfig;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PermissionDecision {
    Accept,
    Reject,
}

#[async_trait]
pub trait PermissionHandler: Send {
    async fn handle_permission_request(
        &mut self,
        request: JsonRpcRequest,
    ) -> Result<PermissionDecision, AcpError>;
}

#[derive(Debug, Default)]
pub struct RejectingPermissionHandler;

#[async_trait]
impl PermissionHandler for RejectingPermissionHandler {
    async fn handle_permission_request(
        &mut self,
        _request: JsonRpcRequest,
    ) -> Result<PermissionDecision, AcpError> {
        Ok(PermissionDecision::Reject)
    }
}

const ALLOWED_COMMANDS: &[&str] = &[
    "cargo",
    "git",
    "ls",
    "cat",
    "find",
    "grep",
    "mkdir",
    "touch",
    "rm",
    "cp",
    "mv",
    "rustc",
    "node",
    "python",
    "pip",
    "npm",
    "npx",
    "deno",
    "which",
    "head",
    "tail",
    "sort",
    "wc",
    "echo",
    "printf",
    "dirname",
    "basename",
    "realpath",
    "readlink",
    "stat",
    "du",
    "df",
    "file",
    "diff",
    "comm",
    "cmp",
    "tee",
    "xargs",
    "env",
    "printenv",
    "pwd",
    "date",
    "sleep",
    "uname",
    "id",
    "whoami",
    "tr",
    "cut",
    "paste",
    "join",
    "uniq",
    "expand",
    "unexpand",
    "fold",
    "fmt",
    "pr",
    "nl",
    "od",
    "xxd",
    "hexdump",
    "tarls",
    "nix",
    "sqlx",
    "psql",
    "createdb",
];

pub struct ProjectScopedPermissionHandler {
    project_root: PathBuf,
}

impl ProjectScopedPermissionHandler {
    #[must_use]
    pub fn new(project_root: PathBuf) -> Self {
        Self { project_root }
    }

    fn path_within_root(&self, path: &Path) -> bool {
        path.canonicalize()
            .is_ok_and(|canonical| canonical.starts_with(&self.project_root))
    }

    fn command_allowed(command: &str) -> bool {
        let trimmed = command.trim();
        ALLOWED_COMMANDS
            .iter()
            .any(|prefix| trimmed.starts_with(prefix))
    }
}

#[async_trait]
impl PermissionHandler for ProjectScopedPermissionHandler {
    async fn handle_permission_request(
        &mut self,
        request: JsonRpcRequest,
    ) -> Result<PermissionDecision, AcpError> {
        let Some(params) = request.params.as_ref() else {
            return Ok(PermissionDecision::Reject);
        };
        let method = params
            .get("method")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("");
        match method {
            "files/read" | "files/write" | "files/edit" | "files/delete" | "files/create" => {
                let path = params
                    .get("path")
                    .and_then(serde_json::Value::as_str)
                    .map(Path::new);
                match path {
                    Some(path) if self.path_within_root(path) => {
                        Ok(PermissionDecision::Accept)
                    }
                    _ => Ok(PermissionDecision::Reject),
                }
            }
            "bash/run" => {
                let command = params
                    .get("command")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or("");
                if Self::command_allowed(command) {
                    Ok(PermissionDecision::Accept)
                } else {
                    Ok(PermissionDecision::Reject)
                }
            }
            _ => Ok(PermissionDecision::Reject),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AcpEvent {
    pub method: String,
    pub text: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AcpRunOutput {
    pub session_id: String,
    pub text: String,
    pub events: Vec<AcpEvent>,
}

pub struct AcpAgentProcess {
    child: Child,
    client: AcpClient<BufReader<ChildStdout>, ChildStdin>,
}

impl AcpAgentProcess {
    pub async fn spawn(config: &AcpAgentConfig) -> Result<Self, AcpError> {
        let mut child = Command::new(&config.binary)
            .args(&config.args)
            .current_dir(&config.cwd)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|err| AcpError::Spawn(format!("{}: {err}", config.binary.display())))?;

        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| AcpError::Spawn("ACP agent stdin unavailable".to_string()))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| AcpError::Spawn("ACP agent stdout unavailable".to_string()))?;

        Ok(Self {
            child,
            client: AcpClient::new(BufReader::new(stdout), stdin),
        })
    }

    pub async fn initialize(&mut self) -> Result<InitializeResult, AcpError> {
        self.client.initialize().await
    }

    pub async fn prompt(
        &mut self,
        cwd: &Path,
        prompt: &str,
        permissions: &mut dyn PermissionHandler,
    ) -> Result<AcpRunOutput, AcpError> {
        self.client.prompt(cwd, prompt, permissions).await
    }
}

impl Drop for AcpAgentProcess {
    fn drop(&mut self) {
        let _ = self.child.start_kill();
    }
}

pub type CodexAcpProcess = AcpAgentProcess;

pub struct AcpClient<R, W> {
    reader: R,
    writer: W,
    next_id: i64,
}

impl<R, W> AcpClient<R, W>
where
    R: AsyncBufRead + Unpin + Send,
    W: AsyncWrite + Unpin + Send,
{
    pub fn new(reader: R, writer: W) -> Self {
        Self {
            reader,
            writer,
            next_id: 1,
        }
    }

    pub async fn initialize(&mut self) -> Result<InitializeResult, AcpError> {
        let params = InitializeParams {
            protocol_version: ACP_PROTOCOL_VERSION,
            client_info: ClientInfo {
                name: "tzu".to_string(),
                version: env!("CARGO_PKG_VERSION").to_string(),
            },
        };
        self.request("initialize", serde_json::to_value(params).unwrap())
            .await
    }

    pub async fn prompt(
        &mut self,
        cwd: &Path,
        prompt: &str,
        permissions: &mut dyn PermissionHandler,
    ) -> Result<AcpRunOutput, AcpError> {
        let session: SessionNewResult = self
            .request(
                "session/new",
                serde_json::to_value(SessionNewParams {
                    cwd: cwd.display().to_string(),
                })
                .unwrap(),
            )
            .await?;
        let request_id = self.claim_id();
        self.send_request(
            request_id.clone(),
            "session/prompt",
            serde_json::to_value(SessionPromptParams {
                session_id: session.session_id.clone(),
                prompt: vec![PromptContent {
                    kind: "text".to_string(),
                    text: prompt.to_string(),
                }],
            })
            .unwrap(),
        )
        .await?;

        let mut events = Vec::new();
        let mut text = String::new();
        loop {
            match self.read_message().await? {
                JsonRpcMessage::Notification(notification) => {
                    collect_notification(&notification, &mut text, &mut events);
                }
                JsonRpcMessage::Request(request) => {
                    self.answer_permission_request(request, permissions).await?;
                }
                JsonRpcMessage::Response(response) if response.id == request_id => {
                    let output = AcpRunOutput {
                        session_id: session.session_id.clone(),
                        text,
                        events,
                    };
                    let _ = self.close_session(&session.session_id).await;
                    return Ok(output);
                }
                JsonRpcMessage::Error(error) if error.id == request_id => {
                    let _ = self.close_session(&session.session_id).await;
                    return Err(rpc_error(error.error));
                }
                other => events.push(AcpEvent {
                    method: "unexpected".to_string(),
                    text: Some(format!("{other:?}")),
                }),
            }
        }
    }

    async fn close_session(&mut self, session_id: &str) -> Result<(), AcpError> {
        let result: Result<serde_json::Value, AcpError> = self
            .request(
                "session/close",
                serde_json::to_value(SessionCloseParams {
                    session_id: session_id.to_string(),
                })
                .unwrap(),
            )
            .await;
        match result {
            Ok(_) | Err(AcpError::Rpc { .. }) => Ok(()),
            Err(e) => Err(e),
        }
    }

    pub async fn request<T>(
        &mut self,
        method: &str,
        params: serde_json::Value,
    ) -> Result<T, AcpError>
    where
        T: DeserializeOwned,
    {
        let id = self.claim_id();
        self.send_request(id.clone(), method, params).await?;
        loop {
            match self.read_message().await? {
                JsonRpcMessage::Response(response) if response.id == id => {
                    return serde_json::from_value(response.result).map_err(|source| {
                        AcpError::Decode {
                            target: "request",
                            source,
                        }
                    });
                }
                JsonRpcMessage::Error(error) if error.id == id => {
                    return Err(rpc_error(error.error));
                }
                JsonRpcMessage::Notification(_) => continue,
                JsonRpcMessage::Request(request) => {
                    self.send_error(
                        request.id,
                        JsonRpcError {
                            code: -32601,
                            message: "tzu has no permission handler during setup".to_string(),
                            data: None,
                        },
                    )
                    .await?;
                }
                other => return Err(AcpError::Unexpected(other)),
            }
        }
    }

    pub async fn send_request(
        &mut self,
        id: RequestId,
        method: &str,
        params: serde_json::Value,
    ) -> Result<(), AcpError> {
        write_jsonrpc_message(
            &mut self.writer,
            &JsonRpcMessage::Request(JsonRpcRequest {
                jsonrpc: JSONRPC_VERSION.to_string(),
                id,
                method: method.to_string(),
                params: Some(params),
            }),
        )
        .await
    }

    pub async fn read_message(&mut self) -> Result<JsonRpcMessage, AcpError> {
        read_jsonrpc_message(&mut self.reader).await
    }

    fn claim_id(&mut self) -> RequestId {
        let id = self.next_id;
        self.next_id += 1;
        RequestId::Number(id)
    }

    async fn answer_permission_request(
        &mut self,
        request: JsonRpcRequest,
        permissions: &mut dyn PermissionHandler,
    ) -> Result<(), AcpError> {
        let id = request.id.clone();
        let decision = permissions.handle_permission_request(request).await?;
        let result = match decision {
            PermissionDecision::Accept => json!({ "decision": "accept" }),
            PermissionDecision::Reject => json!({ "decision": "reject" }),
        };
        write_jsonrpc_message(
            &mut self.writer,
            &JsonRpcMessage::Response(JsonRpcResponse {
                jsonrpc: JSONRPC_VERSION.to_string(),
                id,
                result,
            }),
        )
        .await
    }

    async fn send_error(&mut self, id: RequestId, error: JsonRpcError) -> Result<(), AcpError> {
        write_jsonrpc_message(
            &mut self.writer,
            &JsonRpcMessage::Error(JsonRpcErrorResponse {
                jsonrpc: JSONRPC_VERSION.to_string(),
                id,
                error,
            }),
        )
        .await
    }
}

pub async fn write_jsonrpc_message<W>(
    writer: &mut W,
    message: &JsonRpcMessage,
) -> Result<(), AcpError>
where
    W: AsyncWrite + Unpin,
{
    let payload =
        serde_json::to_vec(message).map_err(|err| AcpError::Transport(err.to_string()))?;
    writer
        .write_all(&payload)
        .await
        .map_err(|err| AcpError::Transport(err.to_string()))?;
    writer
        .write_all(b"\n")
        .await
        .map_err(|err| AcpError::Transport(err.to_string()))?;
    writer
        .flush()
        .await
        .map_err(|err| AcpError::Transport(err.to_string()))
}

pub async fn read_jsonrpc_message<R>(reader: &mut R) -> Result<JsonRpcMessage, AcpError>
where
    R: AsyncBufRead + Unpin,
{
    let mut line = String::new();
    let len = reader
        .read_line(&mut line)
        .await
        .map_err(|err| AcpError::Transport(err.to_string()))?;
    if len == 0 {
        return Err(AcpError::Transport("stdio closed".to_string()));
    }
    serde_json::from_str(line.trim()).map_err(|err| AcpError::Transport(err.to_string()))
}

fn rpc_error(error: JsonRpcError) -> AcpError {
    AcpError::Rpc {
        code: error.code,
        message: error.message,
        data: error.data,
    }
}

fn collect_notification(
    notification: &JsonRpcNotification,
    text: &mut String,
    events: &mut Vec<AcpEvent>,
) {
    let mut event_text = None;
    if notification.method == "session/update"
        && let Some(params) = notification.params.clone()
        && let Ok(update) = serde_json::from_value::<SessionUpdateParams>(params)
        && let Some(chunk) = update
            .update
            .pointer("/content/text")
            .and_then(serde_json::Value::as_str)
    {
        text.push_str(chunk);
        event_text = Some(chunk.to_string());
    }
    events.push(AcpEvent {
        method: notification.method.clone(),
        text: event_text,
    });
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::sync::{Mutex as StdMutex, OnceLock};

    use tokio::sync::Mutex;

    use super::*;

    fn env_lock() -> std::sync::MutexGuard<'static, ()> {
        static LOCK: OnceLock<StdMutex<()>> = OnceLock::new();
        LOCK.get_or_init(|| StdMutex::new(())).lock().unwrap()
    }

    #[tokio::test]
    async fn jsonrpc_message_framing_round_trips_one_line() {
        let message = JsonRpcMessage::Request(JsonRpcRequest {
            jsonrpc: JSONRPC_VERSION.to_string(),
            id: RequestId::Number(7),
            method: "initialize".to_string(),
            params: Some(json!({"protocolVersion": 1})),
        });
        let mut output = Vec::new();
        write_jsonrpc_message(&mut output, &message).await.unwrap();

        assert!(output.ends_with(b"\n"));
        let mut reader = BufReader::new(output.as_slice());
        let parsed = read_jsonrpc_message(&mut reader).await.unwrap();
        assert_eq!(parsed, message);
    }

    #[tokio::test]
    async fn malformed_jsonrpc_message_is_rejected() {
        let mut reader = BufReader::new("not json\n".as_bytes());
        assert!(matches!(
            read_jsonrpc_message(&mut reader).await,
            Err(AcpError::Transport(_))
        ));
    }

    #[test]
    fn default_agent_config_launches_codex_acp() {
        let _guard = env_lock();
        unsafe {
            env::remove_var("TZU_AGENT_BACKEND");
            env::remove_var("TZU_CODEX_ACP_BIN");
            env::remove_var("TZU_DEEPSEEK_ACP_BIN");
        }

        let config = AcpAgentConfig::from_env("/work");

        assert_eq!(config.backend, AcpAgentBackend::Codex);
        assert_eq!(config.backend.label(), "codex");
        assert_eq!(config.binary, PathBuf::from("codex-acp"));
        assert!(config.args.is_empty());
        assert_eq!(config.cwd, PathBuf::from("/work"));
    }

    #[test]
    fn deepseek_agent_config_launches_deepseek_adapter_serve() {
        let _guard = env_lock();
        unsafe {
            env::set_var("TZU_AGENT_BACKEND", "deepseek");
            env::set_var("TZU_DEEPSEEK_ACP_BIN", "/bin/deepseek-acp-adapter");
            env::remove_var("TZU_CODEX_ACP_BIN");
        }

        let config = AcpAgentConfig::from_env("/work");

        assert_eq!(config.backend, AcpAgentBackend::DeepSeek);
        assert_eq!(config.backend.label(), "deepseek");
        assert_eq!(config.binary, PathBuf::from("/bin/deepseek-acp-adapter"));
        assert_eq!(config.args, vec!["serve".to_string()]);
        assert_eq!(config.cwd, PathBuf::from("/work"));

        unsafe {
            env::remove_var("TZU_AGENT_BACKEND");
            env::remove_var("TZU_DEEPSEEK_ACP_BIN");
        }
    }

    #[test]
    fn opencode_agent_config_launches_opencode_acp() {
        let _guard = env_lock();
        unsafe {
            env::set_var("TZU_AGENT_BACKEND", "opencode");
            env::remove_var("TZU_OPENCODE_ACP_BIN");
            env::remove_var("TZU_CODEX_ACP_BIN");
        }

        let config = AcpAgentConfig::from_env("/work");

        assert_eq!(config.backend, AcpAgentBackend::OpenCode);
        assert_eq!(config.backend.label(), "opencode");
        assert_eq!(config.binary, PathBuf::from("opencode"));
        assert_eq!(config.args, vec!["acp".to_string()]);
        assert_eq!(config.cwd, PathBuf::from("/work"));

        unsafe {
            env::remove_var("TZU_AGENT_BACKEND");
        }
    }

    #[test]
    fn hermes_agent_config_launches_hermes_acp() {
        let _guard = env_lock();
        unsafe {
            env::set_var("TZU_AGENT_BACKEND", "hermes");
            env::set_var("TZU_HERMES_ACP_BIN", "/usr/local/bin/hermes");
            env::remove_var("TZU_CODEX_ACP_BIN");
        }

        let config = AcpAgentConfig::from_env("/work");

        assert_eq!(config.backend, AcpAgentBackend::Hermes);
        assert_eq!(config.backend.label(), "hermes");
        assert_eq!(config.binary, PathBuf::from("/usr/local/bin/hermes"));
        assert_eq!(config.args, vec!["acp".to_string()]);
        assert_eq!(config.cwd, PathBuf::from("/work"));

        unsafe {
            env::remove_var("TZU_AGENT_BACKEND");
            env::remove_var("TZU_HERMES_ACP_BIN");
        }
    }

    #[derive(Default)]
    struct RecordingPermissions {
        seen: Arc<Mutex<usize>>,
    }

    #[async_trait]
    impl PermissionHandler for RecordingPermissions {
        async fn handle_permission_request(
            &mut self,
            _request: JsonRpcRequest,
        ) -> Result<PermissionDecision, AcpError> {
            *self.seen.lock().await += 1;
            Ok(PermissionDecision::Accept)
        }
    }

    #[tokio::test]
    async fn mocked_acp_prompt_collects_updates_and_answers_permission_request() {
        let inbound = concat!(
            "{\"jsonrpc\":\"2.0\",\"id\":1,\"result\":{}}\n",
            "{\"jsonrpc\":\"2.0\",\"id\":2,\"result\":{\"sessionId\":\"s1\"}}\n",
            "{\"jsonrpc\":\"2.0\",\"id\":99,\"method\":\"permission/request\",\"params\":{\"reason\":\"test\"}}\n",
            "{\"jsonrpc\":\"2.0\",\"method\":\"session/update\",\"params\":{\"sessionId\":\"s1\",\"update\":{\"content\":{\"text\":\"done\"}}}}\n",
            "{\"jsonrpc\":\"2.0\",\"id\":3,\"result\":{}}\n"
        );
        let reader = BufReader::new(inbound.as_bytes());
        let writer = Vec::new();
        let mut client = AcpClient::new(reader, writer);
        client.initialize().await.unwrap();

        let seen = Arc::new(Mutex::new(0));
        let mut permissions = RecordingPermissions {
            seen: Arc::clone(&seen),
        };
        let output = client
            .prompt(Path::new("."), "do thing", &mut permissions)
            .await
            .unwrap();

        assert_eq!(output.text, "done");
        assert_eq!(*seen.lock().await, 1);
    }
}
