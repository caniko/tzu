# tzu

`tzu` is a local-first general planning harness. It owns project state, problem
specs, candidate plan sketches, task decomposition, task DAGs, validation,
persistence, policy, and run reports. Coding is the first specialized domain
adapter. Codex owns code-generation and execution inside an Agent Client
Protocol session through Zed's `codex-acp` adapter.

`tzu` does not call OpenAI APIs directly and does not parse `codex exec` output.
It talks to `codex-acp` over stdio JSON-RPC.

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
include_nested_contexts = false

[gui]
host = "127.0.0.1"
port = 7070
enable_service = false
```

`projects_directory` is a discovery base for colocated local projects. The GUI
discovers direct child directories that contain `.git` or a known project
manifest such as `Cargo.toml`, `package.json`, `flake.nix`, `pyproject.toml`,
`go.mod`, or `lakefile.toml`. Discovered projects are suggestions only; they
are added to plan context explicitly.

Environment overrides:

```sh
export TZU_PROJECTS_DIR=/path/to/projects
export TZU_INCLUDE_NESTED_CONTEXTS=true
```

When the GUI sees a discovered project name while you type a goal, it shows a
small suggestion. Press `Ctrl+Space` to add that project path to the context
roots.

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
```

Launch the local GUI:

```sh
tzu-gui --project-root . --port 7070
```

Launch the GUI with development reload enabled:

```sh
cargo dev-server
```

This shortcut uses SQLite at `.tzu/state.sqlite` and enables the GUI's
`dev-hot-reload` feature. Plain `cargo run --bin tzu-gui` remains the normal
non-reloading server path.

The GUI serves a Leptos/Axum workbench on `http://127.0.0.1:7070` by default.
It uses the same project state and database resolution as the CLI.

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
- `tzu` never stores auth tokens.
- `codex-acp` is found on `PATH` or via `TZU_CODEX_ACP_BIN`.
- ACP permission requests are handled explicitly by `tzu-acp`; unknown protocol
  messages are preserved as opaque JSON instead of being guessed outside
  `tzu-acp/src/protocol.rs`.
- Agent execution goes through `codex-acp`; direct OpenAI API calls are not part
  of `tzu`.
- Project planning, validation, persistence, task DAGs, and policy remain local.
