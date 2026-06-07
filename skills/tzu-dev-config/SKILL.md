---
name: tzu-dev-config
description: Create, validate, or repair tzu's repo-local development config at .tzu/xdg/tzu/config.toml. Use when working in the tzu repository and development commands fail because the required dev config is missing or stale.
---

# tzu Dev Config

Use this skill only inside the `tzu` repository.

## Workflow

1. Inspect the active path:

   ```sh
   tzu-dev-config path
   ```

2. Create the config if it is missing:

   ```sh
   tzu-dev-config init
   ```

   If the file exists and should be replaced, use:

   ```sh
   tzu-dev-config init --force
   ```

3. Validate through the real loader and CLI path:

   ```sh
   tzu-dev-config validate
   ```

## Failure Rule

Do not invent project roots or silently substitute another config path. If the config cannot be created or validated, report:

- the missing artifact or source
- why it is required
- the upstream producer to fix
- the exact regeneration command
- the validation command

The expected regeneration command is `tzu-dev-config init`; the validation command is `tzu-dev-config validate`.
