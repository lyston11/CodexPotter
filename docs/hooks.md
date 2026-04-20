# Hooks (Experimental)

CodexPotter supports **hooks** similar to upstream Codex. Hooks are user-configured commands that
run on specific events and receive a single JSON object on **stdin**.

This feature is **experimental**. The supported events and payload shape may change.

Upstream reference: https://developers.openai.com/codex/hooks

## Configuration files (`hooks.json`)

CodexPotter discovers hook configuration from two locations (both are loaded when present):

1. **User-level**
   - `$CODEX_HOME/hooks.json` when `CODEX_HOME` is set and non-empty
   - otherwise `~/.codex/hooks.json`
2. **Repo-level**
   - `<repo>/.codex/hooks.json`
   - `<repo>` is the nearest parent directory (starting from `cwd`) that contains a `.git`
     directory (fallback: `cwd` if no `.git` exists)

## Supported event: `Potter.ProjectStop`

`Potter.ProjectStop` runs when a Potter project stops (the same boundary where project completion
markers are emitted), including:

- success
- budget exhausted
- interrupted + user chose "stop iterate"
- task failed / fatal failures

This event **does not support matchers** (any `matcher` fields are ignored) and **does not support
output fields** (hook output does not affect runtime behavior).

### `hooks.json` example

Create `~/.codex/hooks.json` (or `$CODEX_HOME/hooks.json`) with a `command` hook:

```json
{
  "hooks": {
    "Potter.ProjectStop": [
      {
        "hooks": [
          {
            "type": "command",
            "command": "cat > /tmp/potter-project-stop.json",
            "timeout": 30,
            "async": false,
            "statusMessage": "Saving project stop payload..."
          }
        ]
      }
    ]
  }
}
```

Notes:

- `timeout` is in seconds (default: 600; minimum: 1).
- Only synchronous `command` hooks are supported today:
  - `async: true` hooks are skipped with a warning.
  - `prompt` and `agent` hook types are skipped with a warning.

### Payload

Every `command` hook receives a single JSON object on stdin:

```json
{
  "project_dir": "/home/you/.codexpotter/projects/2026/04/20/4",
  "project_file_path": "/home/you/.codexpotter/projects/2026/04/20/4/MAIN.md",
  "cwd": "/home/you/workspace",
  "hook_event_name": "Potter.ProjectStop",
  "user_prompt": "…",
  "all_session_ids": ["…"],
  "new_session_ids": ["…"],
  "all_assistant_messages": ["…"],
  "new_assistant_messages": ["…"],
  "stop_reason_code": "succeeded"
}
```

Field notes:

- `project_dir`: directory containing the project progress file.
- `project_file_path`: absolute path to the project's `MAIN.md`.
- `cwd`: project working directory.
- `all_session_ids`: per-round session/thread ids (best-effort).
- `new_session_ids`: session ids created since the current iteration window began:
  - fresh project: equals `all_session_ids`
  - resumed project: contains only rounds executed after resume
- `all_assistant_messages`: last assistant message for each round (best-effort; empty string when
  extraction fails).
- `new_assistant_messages`: messages corresponding to `new_session_ids`.
- `stop_reason_code`: one of `succeeded`, `budget_exhausted`, `interrupted`, `task_failed`, `fatal`.
