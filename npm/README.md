# CodexPotter

Install:

```sh
npm install -g codex-potter
```

If you install the npm package with Bun, keep `node` on your PATH for now:

```sh
bun install -g codex-potter
```

The published npm package currently exposes `codex-potter` through a JavaScript launcher, and Bun
links that launcher directly. On machines that only have Bun and do not have `node`, the installed
`codex-potter` command fails before the launcher starts. Until the package layout is redesigned to
ship a native top-level bin, Bun-only machines should use the standalone release archives instead
of the npm package.

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
