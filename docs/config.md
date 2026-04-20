# Configuration

CodexPotter reads a per-user TOML config file:

- Path: `~/.codexpotter/config.toml`

## Common settings

```toml
# Default number of rounds to run when `--rounds` is not provided.
# (default: 10)
rounds = 15

# Check for updates on startup. (default: true)
check_for_update_on_startup = true

[notice]
# Hide the global gitignore prompt on startup. (default: false)
hide_gitignore_prompt = false

[tui]
# Default transcript verbosity. (default: "minimal")
# One of: "minimal", "simple"
verbosity = "minimal"

[potter]
# Enable YOLO by default. (default: false)
# Warning: YOLO disables approvals and sandboxing.
yolo = false
```

## Precedence rules

- `rounds`: `--rounds` > `config.toml` `rounds` > `10`
- YOLO: `--yolo` always enables YOLO for the current run; otherwise `[potter].yolo` is used
- `exec` verbosity: `codex-potter exec --verbosity` > `[tui].verbosity` > `"minimal"`

## Full reference

| Key | Type | Default | Description |
| --- | --- | --- | --- |
| `rounds` | integer (>= 1) | `10` | Default round budget for runs that do not specify `--rounds`. |
| `check_for_update_on_startup` | bool | `true` | Enable update checks and prompts on startup. |
| `[notice].hide_gitignore_prompt` | bool | `false` | Hide the prompt that suggests adding `.codexpotter/` to your global gitignore. |
| `[tui].verbosity` | string | `"minimal"` | Default transcript verbosity (`"minimal"` or `"simple"`). |
| `[potter].yolo` | bool | `false` | Enable YOLO by default (unsafe; disables approvals and sandboxing). |

