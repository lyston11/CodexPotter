# codex-hooks (CodexPotter)

## Overview

This `hooks/` crate is intended to stay as close as possible to upstream Codex's hooks crate so that
syncing changes from upstream remains low-friction.

- Upstream reference (relative to this repo root): `../codex/codex-rs/hooks`
- This crate must remain UI-agnostic: it should not depend on `tui/` and must not contain Potter
  workflow/business logic.

## Supported hooks

CodexPotter currently supports one hook event:

- `Potter.ProjectStop` (config key in `hooks.json`)

This event is Potter-specific and is the only intentional divergence from upstream behavior.

## Discovery rules (`hooks.json`)

Hook configuration is discovered the same way as upstream Codex:

- User-level config:
  - `$CODEX_HOME/hooks.json` when `CODEX_HOME` is set and non-empty
  - otherwise `~/.codex/hooks.json`
- Repo-level config:
  - `<repo>/.codex/hooks.json` where `<repo>` is discovered by walking up from `cwd` until a
    directory containing `.git` is found (fallback: `cwd` if no `.git` exists)

Both configs are loaded when present.

## Conventions

- Keep upstream parity by default; when introducing a divergence, document it clearly and add
  regression coverage.
- Keep the public API surface minimal and well-documented (English doc comments).
- Prefer simple, explicit code over clever abstractions.

## Schemas and fixtures

The JSON schema fixtures live under `hooks/schema/generated/` and must be kept in sync with the
Rust schema definitions.

- Update fixtures:
  - `cargo run -p codex-hooks --bin write_hooks_schema_fixtures`
- Verify:
  - `cargo test -p codex-hooks`

## Validation

Before committing changes that touch this crate:

- Run `cargo fmt`
- Run `cargo clippy --workspace --all-targets`
- Run `cargo test -p codex-hooks`

