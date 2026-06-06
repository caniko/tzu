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

## Usage

```sh
tzu init
tzu plan "organize a research workshop"
tzu plan "add health endpoint" --domain coding
tzu status
tzu run inspect-repo
```

Launch the local GUI:

```sh
tzu-gui --project-root . --port 7070
```

The GUI serves a Leptos/Axum workbench on `http://127.0.0.1:7070` by default.
It uses the same project state and database resolution as the CLI.

`tzu plan` defaults to the generic planning harness. The harness builds an
immutable problem spec, seeds candidate plan sketches, validates them, scores the
valid candidates, selects a champion, and persists the selected plan plus
harness metadata. Use `--domain coding` for repository-aware coding plans.
`--domain legacy-coding` keeps the original deterministic coding DAG available
as a compatibility path.

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
