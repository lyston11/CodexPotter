# Hooks

CodexPotter supports **hooks** [similar to Codex](https://developers.openai.com/codex/hooks). Hooks allow you to inject your own scripts into the agentic loop.

⚠️ This feature is **experimental**. Events and payload shape may change.

## Where CodexPotter looks for hooks

CodexPotter discovers hook configuration from two locations, same as codex:

- User-level config:
  - `$CODEX_HOME/hooks.json` when `CODEX_HOME` is set and non-empty
  - otherwise `~/.codex/hooks.json`
- Repo-level config:
  - `<repo>/.codex/hooks.json` where `<repo>` is discovered by walking up from the session `cwd`
    until a directory containing `.git` is found (fallback: the session `cwd` if no `.git` exists)

If more than one `hooks.json` file exists, CodexPotter loads all matching hooks, as same as codex. Higher-precedence config layers do not replace lower-precedence hooks.

## Config shape

Here is an example `hooks.json` with a single `Potter.ProjectStop` hook, which is triggered when a Potter project stops for any reason (success, failure, interruption, etc.):

```json
{
  "hooks": {
    "Potter.ProjectStop": [
      {
        "hooks": [
          {
            "type": "command",
            "command": "cat >> /tmp/potter-project-stop.json",
            "timeout": 30
          }
        ]
      }
    ]
  }
}
```

Notes:

- `timeout` is in seconds.
- `timeoutSec` is also accepted as an alias.
- If `timeout` is omitted, CodexPotter uses `600` seconds.
- `statusMessage` is optional and, when present, is shown while the hook is running.
- Commands run with the session `cwd` as their working directory.

## Hooks

### `Potter.ProjectStop`

Triggered when a Potter project stops (the same boundary where CodexPotter summary are emitted), including:

- success
- round budget exhausted
- user interrupt
- task failed / fatal failures

This event **does not support matchers** (any `matcher` fields are ignored) and **does not support
output fields** (hook output does not affect runtime behavior).

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
- `all_assistant_messages`: last assistant message for each round.
- `new_assistant_messages`: messages corresponding to `new_session_ids`.
- `stop_reason_code`: one of `succeeded`, `budget_exhausted`, `interrupted`, `task_failed`, `fatal`.
