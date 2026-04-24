<!--

For agents: This file is carefully maintained and polished for better readability. Don't edit this file.

-->

<p align="center">
  <img src="./etc/banner.svg" alt="CodexPotter banner" />
</p>

<p align="center">
  <img src="./etc/screenshot.png" alt="CodexPotter screenshot" width="80%" />
</p>

&ensp;

## 💡 Why CodexPotter

[![Platform](https://img.shields.io/badge/Platform-Linux%20%7C%20macOS%20%7C%20Windows-blue?style=flat-square)](#)
[![npm](https://img.shields.io/npm/v/codex-potter?label=Release&style=flat-square)](https://www.npmjs.com/package/codex-potter)
[![CI](https://img.shields.io/github/actions/workflow/status/breezewish/CodexPotter/ci.yml?branch=main&label=CI&style=flat-square)](https://github.com/breezewish/CodexPotter/actions/workflows/ci.yml)
[![License](https://img.shields.io/github/license/breezewish/CodexPotter?label=License&style=flat-square)](./LICENSE)
[![LinuxDo](https://img.shields.io/badge/Community-LINUX%20DO-blue?style=flat-square)](https://linux.do)

**CodexPotter** continuously **reconciles** code base toward your instructed state ([Ralph Wiggum pattern](https://ghuntley.com/ralph/)):

- 🤖 **Codex-first** — Codex subscription is all you need; no extra LLM needed.
- 🧭 **Auto-review / reconcile** — Review and polish multi rounds until fully aligned with your instruction.
- 💦 **Clean-room** — Use clean context in each round, avoid context poisoning, maximize IQ.
- 🎯 **Attention is all you need** — Keep you focused on _crafting_ tasks, instead of _cleaning up_ unfinished work.
- 🚀 **Never worse than Codex** — Drive Codex, nothing more; no business prompts which may not suit you.
- 🧩 **Seamless integration** — AGENTS.md, skills & MCPs just work™ ; opt in to improve plan / review.
- 🧠 **File system as memory** — Store instructions in files to resist compaction and preserve all details.
- 🪶 **Tiny footprint** — Use [<1k tokens](./cli/prompts/developer_prompt.md), ensuring LLM context fully serves your business logic.
- 📚 **Built-in knowledge base** — Keep a local KB as index so Codex learns project fast in clean contexts.

&ensp;

## 👀 How does it work

```plain

                                                  𝒀𝑶𝑼𝑹 𝑷𝑹𝑶𝑴𝑷𝑻:
                                                  𝘚𝘪𝘮𝘱𝘭𝘪𝘧𝘺 𝘵𝘩𝘦 𝘲𝘶𝘦𝘳𝘺 𝘦𝘯𝘨𝘪𝘯𝘦 𝘣𝘺 𝘧𝘰𝘭𝘭𝘰𝘸𝘪𝘯𝘨 ...
                                                                 │
                                                                 │
     codex: Work or review according to MAIN.md                  │
            ┌──────────────────────────┐                         │
            │                          │                         ▼
  ┌─────────┴─────────┐     ┌──────────▼────────┐       ┌───────────────────┐
  │    CodexPotter    │     │       codex       │◄─────►│      MAIN.md      │
  └─────────▲─────────┘     └──────────┬────────┘       └───────────────────┘
            │                          │
            │      Work finished       │
            └──────────────────────────┘

```

&ensp;

## ⚡️ Getting started

**1. Prerequisites:** ensure you have [codex CLI](https://developers.openai.com/codex/quickstart?setup=cli) locally. CodexPotter drives your local codex to perform tasks.

**2. Install CodexPotter via npm or bun:**

```shell
# Install via npm
npm install -g codex-potter
```

```shell
# Install via bun
bun install -g codex-potter
```

**3. Run:** Start CodexPotter in your project directory, just like Codex:

```sh
# --yolo is recommended to be fully autonomous
codex-potter --yolo
```

⚠️ **Note:** Unlike Codex, every follow up prompt turns into a **new** task, **not sharing previous contexts**. Assign tasks to CodexPotter, instead of chat with it.

⚠️ **Note:** CodexPotter is **not a replacement** for codex, because CodexPotter is a loop executor — it executes tasks instead of chatting with you. See below for details.

&ensp;

## Tips

### Prompt Examples

**✅ tasks with clear goals or scopes:**

- "port upstream codex's /resume into this project, keep code aligned"

**✅ persist results to review in later rounds:**

- "create a design doc for ... **in DESIGN.md**"

**❌ interactive tasks with human feedback loops:**

CodexPotter is not suitable for such tasks, use codex instead:

- Front-end development with human UI feedback
- Question-answering
- Brainstorming sessions

### Howto

<details>
<summary>Ask followup questions in codex</summary>

Just pass the project file to codex, like:

```plain
based on .codexpotter/projects/2026/03/18/1/MAIN.md,
please explain more about the root cause of the issue
```

</details>

<details>
<summary>Plan and execute</summary>

Simpliy queue two tasks in CodexPotter, one is plan, one is implement, CodexPotter will execute one by one, for example:

Task prompt 1 (CodexPotter):

```plain
Analyze the codebase, research and design a solution for introducing subscription system.
Output plan to docs/subscription_design.md.

Your solution should meet the following requirements: ...

Do not implement the plan, just design a good and simple solution.
```

↑ Your existing facility to write good plans will be utilized, including skills, plan doc principles
in AGENTS.md, etc. **Writing plan to a file is CRITICAL** so that the plan can be iterated multiple rounds and task 2 can pick it up.

Task prompt 2 (CodexPotter):

```plain
Implement according to docs/subscription_design.md

Make sure all user journeys are properly covered by e2e tests and pass.
```

If you even don't know what you are designing for, just discuss with **codex** to carry out a basic plan first, then use **CodexPotter** to continously polish and implement it.

</details>

&ensp;

## Configuration

- [Config File](./docs/config.md)
- [Hooks](./docs/hooks.md)

&ensp;

## Other Features

- `--xmodel` (experimental): Use gpt-5.2 first, then use gpt-5.5 to cross review gpt-5.2's work in later rounds. In clear coding tasks this may produce better results than only using gpt-5.2 or gpt-5.5.

- `/yolo`: Toggle whether YOLO (no sandbox) is enabled by default for all sessions.

- `/list` or `ctrl+l`: View all projects (tasks) and their results.

&ensp;

## Roadmap

- [x] Skill popup
- [x] Resume (history replay + continue iterating)
- [x] Better handling of stream disconnect / similar network issues
- [x] Agent-call friendly (non-interactive exec and resume)
- [x] Interoperability with codex CLI sessions (for follow-up prompts)
- [ ] Better plan / user selection support
- [ ] Better sandbox support

&ensp;

## Development

```sh
# Formatting
cargo fmt

# Lints
cargo clippy

# Tests
cargo nextest run

# Build
cargo build
```

&ensp;

## Community & License

- This project is community-driven fork of [openai/codex](https://github.com/openai/codex) repository, licensed under the same Apache-2.0 License.
