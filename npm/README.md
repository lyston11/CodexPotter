# CodexPotter

Install:

```sh
npm install -g codex-potter
```

```sh
bun install -g codex-potter
```

The published package launches the bundled native binary directly, so Bun-managed installs on Unix
do not require `node` on `PATH`.

On Windows, use `npm install -g codex-potter` for now. Real Windows verification showed Bun's
generated `.exe` shims still fail to forward arguments to `.cmd` package bins, so Bun-managed
Windows installs remain upstream-limited even though the packaged launcher itself works when run
directly.

Run:

```sh
codex-potter --yolo
```

Supported platforms (via bundled native binaries):

- macOS: Apple Silicon + Intel
- Linux: x86_64 + aarch64
- Windows: x86_64 + aarch64 (ARM64)
- Android: treated as Linux (uses the bundled Linux musl binaries)

Build from source:

```sh
cargo build
./target/debug/codex-potter --yolo
```
