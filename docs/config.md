# Configuration

CodexPotter reads a per-user TOML config file:

- Path: `~/.codexpotter/config.toml`

## Common settings

```toml
# Default number of rounds to run when `--rounds` is not provided.
# (default: 10)
# rounds = 10

# Check for updates on startup. (default: true)
# check_for_update_on_startup = true

# Enable YOLO by default. (default: false)
# Warning: YOLO disables approvals and sandboxing.
# yolo = false

[tui]
# Default transcript verbosity. (default: "minimal")
# One of: "minimal", "simple"
# verbosity = "minimal"
```
