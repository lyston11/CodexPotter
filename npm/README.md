# CodexPotter

Install:

```sh
npm install -g codex-potter
```

Supported platforms:

- macOS: Apple Silicon + Intel
- Linux: x86_64 + aarch64
- Windows: x86_64 + aarch64 (ARM64)

Packaging note:

- The `codex-potter` npm package ships a small cross-platform launcher.
- The native binary payload is delivered via platform-specific optional dependencies.
  If you see an error like `Missing optional dependency codex-potter-<platform>`, reinstall
  CodexPotter so your package manager can fetch the correct platform package.

---

See https://github.com/breezewish/CodexPotter for full usage.
