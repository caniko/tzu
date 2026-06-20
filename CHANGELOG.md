# Changelog

## [Unreleased]

### Added

- ACP session/close protocol params; hermes and opencode agent backends; project-scoped permission handler (ACP crate)
- Agent generation types, prompt generation for generic and coding domains, and JSON candidate parser (core crate)
- ACP agent candidate generation with async background spawn and task-dependency blocking check (runner crate)
- MCP server command (`tzu mcp`) exposing planning tools via stdio MCP transport (CLI crate, new tzu-mcp crate)
- Plan arena visualization with WASM fighter simulation and candidate scoring display (GUI crate, new tzu-arena crate)
- `exec` subcommand to `tzu-dev-config` for running commands with scoped XDG/tzu env vars

### Changed

- Runner uses `ProjectScopedPermissionHandler` instead of `RejectingPermissionHandler` for ACP sessions
- Runner replaces codex-agent-env detection with opencode/hermes agent config
- GUI tracing defaults to INFO level when no env filter is set
- `.envrc` sets `TZU_DATABASE_URL` for direnv users
- Nix dev shell no longer overrides `XDG_CONFIG_HOME` or requires config (moved to `tzu-dev-config exec`)

### Documentation

- README documents opencode, hermes, MCP integration, and updated dev workflow
