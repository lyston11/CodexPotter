# Configuration

CodexPotter reads a per-user TOML config file:

- Path: `~/.codexpotter/config.toml`

## Keys

- `rounds` (integer, >= 1): default round budget when `--rounds` is not provided (default: `10`)
- `check_for_update_on_startup` (bool): enable update checks on startup (default: `true`)
- `yolo` (bool): enable YOLO by default (default: `false`)
  - Warning: YOLO disables approvals and sandboxing.
  - Legacy: `[potter].yolo` is still accepted and is migrated to the top-level key on read
    (best-effort).
- `[notice].hide_gitignore_prompt` (bool): hide the global gitignore startup prompt (default:
  `false`)
- `[tui].verbosity` (string): default transcript verbosity (default: `"minimal"`, one of
  `"minimal"`, `"simple"`)

These settings can also be changed interactively in the TUI (for example via `/yolo` and
`/verbosity`).

## Precedence

- `--rounds` overrides `rounds`.
- `--yolo` always enables YOLO for the current run (overrides `yolo`).
- `codex-potter exec --verbosity` overrides `[tui].verbosity` for that command.

## Example

```toml
# Default number of rounds to run when `--rounds` is not provided.
# (default: 10)
# rounds = 10

# Check for updates on startup. (default: true)
# check_for_update_on_startup = true

# Enable YOLO by default. (default: false)
# Warning: YOLO disables approvals and sandboxing.
# yolo = false

[notice]
# Hide the global gitignore prompt on startup. (default: false)
# hide_gitignore_prompt = false

[tui]
# Default transcript verbosity. (default: "minimal")
# One of: "minimal", "simple"
# verbosity = "minimal"
```
