# tzu

<!-- simit:badges:start -->
![CI](https://img.shields.io/badge/CI-managed-2088ff) [![crates.io](https://img.shields.io/badge/crates.io-ready-f46623)](https://crates.io/crates/tzu-acp)
<!-- simit:badges:end -->

`tzu` is a local-first general planning harness. It owns project state, problem
specs, candidate plan sketches, task decomposition, task DAGs, validation,
persistence, policy, and run reports. Coding is the first specialized domain
adapter. Agent execution happens inside an Agent Client Protocol session.
Codex is the default backend through Zed's `codex-acp` adapter, and DeepSeek V4
is supported through the `deepseek-acp-adapter` crate.

`tzu` does not parse CLI transcript output. It talks to ACP adapters over stdio
JSON-RPC.

## Installation

Build from the repository root:

```sh
cargo build --workspace
```

With Nix:

```sh
nix develop
cargo build --workspace
```

The CLI binary is `tzu`.

## Agent Backends

`tzu` selects the ACP backend with `TZU_AGENT_BACKEND`:

- unset or `codex`: launch `codex-acp`
- `deepseek` or `deepseek-v4`: launch `deepseek-acp-adapter serve`
- `opencode`: launch `opencode acp`
- `hermes`: launch `hermes acp`

`deepseek-acp-adapter` is the right third-party crate for DeepSeek Platform
support in this project because it preserves `tzu`'s existing ACP boundary while
speaking DeepSeek's OpenAI-compatible `/chat/completions` API, including the V4
model defaults and tool loop.

## OpenCode

Install and configure OpenCode outside `tzu`. `tzu` uses the ACP interface
exposed by `opencode acp`:

```sh
export TZU_AGENT_BACKEND=opencode
opencode acp --help
```

Override the adapter binary with:

```sh
export TZU_OPENCODE_ACP_BIN=/path/to/opencode
```

OpenCode handles model selection, tool execution, file operations, LSP
integration, and MCP servers — all behind the ACP boundary.

## Hermes Agent

Install and configure Hermes Agent outside `tzu`. `tzu` uses the ACP interface
exposed by `hermes acp`:

```sh
export TZU_AGENT_BACKEND=hermes
hermes acp --help
```

Override the adapter binary with:

```sh
export TZU_HERMES_ACP_BIN=/path/to/hermes
```

Hermes provides self-improving skills, provider routing across 300+ models,
and a learning loop — all behind the ACP boundary.

## MCP Integration

`tzu` exposes its planning capabilities as MCP tools, allowing agents to call
`tzu` directly for structured planning:

```sh
tzu mcp
```

This starts an MCP server over stdio. Configure it in your agent:

```json
{
  "mcpServers": {
    "tzu": {
      "command": "tzu",
      "args": ["mcp"]
    }
  }
}
```

Available tools:

| Tool | Description |
|---|---|
| `tzu_init` | Initialize project state |
| `tzu_inspect_prompt` | Validate a goal prompt before planning |
| `tzu_plan` | Create a structured plan from a goal |
| `tzu_status` | Show current plan, tasks, and run reports |
| `tzu_inspect` | Show frontier selection details |
| `tzu_run` | Execute a task by ID |
| `tzu_context` | Get repository context summary |

## Codex CLI and `codex-acp`

Install and authenticate Codex CLI outside `tzu`. `tzu` never reads, prints,
stores, or copies Codex authentication tokens.

After authenticating Codex, install `codex-acp` and ensure it is on `PATH`:

```sh
codex-acp --help
```

Override the adapter binary with:

```sh
export TZU_CODEX_ACP_BIN=/path/to/codex-acp
```

## DeepSeek V4 and `deepseek-acp-adapter`

Install the adapter and provide a DeepSeek Platform API key:

```sh
cargo install deepseek-acp-adapter
export TZU_AGENT_BACKEND=deepseek
export DEEPSEEK_API_KEY=sk-...
```

The adapter defaults to DeepSeek Platform at `https://api.deepseek.com` and to
`deepseek-v4-pro`. Override these when needed:

```sh
export TZU_DEEPSEEK_ACP_BIN=/path/to/deepseek-acp-adapter
export DEEPSEEK_BASE_URL=https://api.deepseek.com
export DEEPSEEK_MODEL=deepseek-v4-flash
```

## Database

`tzu` resolves persistence in this order:

1. `TZU_DATABASE_URL`
2. Linux default: `postgres:///tzu?host=/run/postgresql`
3. Other systems: `sqlite://.tzu/state.sqlite`

On this system the local Postgres database can be created with:

```sh
createdb -h /run/postgresql tzu
psql 'postgres:///tzu?host=/run/postgresql' -c 'select 1'
```

SQLite remains supported for local tests and non-Linux defaults.

## Configuration

`tzu` reads optional global configuration from
`$XDG_CONFIG_HOME/tzu/config.toml`, falling back to
`~/.config/tzu/config.toml`.

```toml
projects_directory = "/path/to/projects"
projects_directories = ["/path/to/more-projects", "/path/to/game-dev"]
include_nested_contexts = false

[gui]
host = "127.0.0.1"
port = 7070
enable_service = false
```

`projects_directory` is the legacy single discovery base for colocated local
projects. `projects_directories` adds more discovery bases. Each base is scanned
shallowly: only direct child project roots with known markers such as `.git`,
`Cargo.toml`, `flake.nix`, or `package.json` are discovered. The GUI also
exposes the active project root in its settings dialog.

Environment overrides:

```sh
export TZU_PROJECTS_DIR=/path/to/projects
export TZU_PROJECTS_DIRS="/path/to/projects:/path/to/game-dev"
export TZU_INCLUDE_NESTED_CONTEXTS=true
```

The Nix development shell and the `.envrc` set `TZU_DATABASE_URL` —
scoped to tzu only (no `XDG_CONFIG_HOME` set), so your editor and
developer tool configs are never disturbed.

One-time dev setup:

```sh
tzu-dev-config init
tzu-dev-config validate
```

Run tzu with scoped `XDG_CONFIG_HOME` (config file at `.tzu/xdg/tzu/config.toml`):

```sh
tzu-dev-config exec tzu --project-root . status
tzu-dev-config exec tzu --project-root . plan "add health endpoint" --domain coding
```

Or via cargo:

```sh
tzu-dev-config exec cargo run -p tzu-cli --bin tzu -- status
```

`.tzu/` is intentionally gitignored; it holds the repo-local development config
and local SQLite state.

In the GUI, repository context is selected inside the goal input with `@`
mentions. Type `@` to reference a discovered repository root, for example
`@regicide` when `regicide` is a direct child of one configured discovery base.
Type an absolute path such as `@/home/alice/other-repo` to pass a directory
outside the discovered project list. The visible goal keeps concise readable
labels, while the planner receives a raw version with canonical paths and the
extracted paths as context roots.

`include_nested_contexts` affects context traversal after a root is selected; it
does not make `@` suggestions recurse through project subdirectories.

The flake exposes a Home Manager module as `homeModules.tzu` and
`homeModules.default`:

```nix
programs.tzu = {
  enable = true;
  projectsDirectory = /home/alice/Projects;
  includeNestedContexts = false;
  gui.enable = true;
};
```

## Usage

```sh
tzu init
tzu plan "organize a research workshop"
tzu plan "add health endpoint" --domain coding
tzu status
tzu inspect
tzu run inspect-repo
tzu mcp
```

Launch the GUI with development reload enabled (builds WASM frontend + starts server + watches for changes):

```sh
cargo dev-server
```

Then open `http://127.0.0.1:7070` in a browser.

For a plain server start (no auto-reload, no WASM rebuild — uses previously built frontend assets):

```sh
tzu-dev-config exec cargo run -p tzu-gui -- --project-root . --port 7070
```

Before the first dev run, create the required repo-local config with
`tzu-dev-config init`. The `cargo dev-server` shortcut uses SQLite at
`.tzu/state.sqlite` and enables the GUI's `dev-hot-reload` feature. The GUI
reloads config-derived settings on config-facing API requests, so edits to
`.tzu/xdg/tzu/config.toml` are visible after the next browser refresh.

The GUI serves a Leptos/Axum workbench on `http://127.0.0.1:7070` by default.
It uses the same project state and database resolution as the CLI. The server
address is printed to stderr at startup.

`tzu plan` defaults to the generic planning harness. The harness builds an
immutable problem spec, seeds deterministic candidate plan sketches, validates
them, scores the valid candidates with structured selection metadata, retains a
capped frontier, selects one execution champion, and persists the selected plan
plus harness metadata. Use `--domain coding` for repository-aware coding plans.

`tzu status` keeps the normal view concise: it shows the selected candidate,
candidate count, frontier size, and ordered tasks. Use `tzu inspect` for
retained candidates, discard reasons, score buckets, and descriptor cells.

`tzu run` currently uses a mocked ACP run path by default and writes a structured
run report into project state.

To use the real adapter path later, set:

```sh
TZU_RUN_MODE=real tzu run implement-goal
```

## Security Notes

- Codex authentication is external to `tzu`.
- `tzu` never stores auth tokens, DeepSeek API keys, or OpenCode/Hermes credentials.
- `codex-acp` is found on `PATH` or via `TZU_CODEX_ACP_BIN`.
- `deepseek-acp-adapter` is found on `PATH` or via `TZU_DEEPSEEK_ACP_BIN` when
  `TZU_AGENT_BACKEND=deepseek`.
- `opencode` is found on `PATH` or via `TZU_OPENCODE_ACP_BIN` when
  `TZU_AGENT_BACKEND=opencode`.
- `hermes` is found on `PATH` or via `TZU_HERMES_ACP_BIN` when
  `TZU_AGENT_BACKEND=hermes`.
- ACP permission requests are handled explicitly by `tzu-acp`; unknown protocol
  messages are preserved as opaque JSON instead of being guessed outside
  `tzu-acp/src/protocol.rs`.
- Agent execution goes through the selected ACP adapter; direct provider API
  calls are not part of `tzu`.
- Project planning, validation, persistence, task DAGs, and policy remain local.
