# CodexPotter

Install:

```sh
npm install -g codex-potter
```

```sh
bun install -g codex-potter
```

The published package launches the bundled native binary directly, so Bun-managed installs do not
require `node` on `PATH`.

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
