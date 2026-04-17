# npm releases

Use the staging helper in `npm/scripts/` to generate npm tarballs for a release. For
example, after the release workflow has downloaded build artifacts into `dist/`:

```bash
./npm/scripts/stage_npm_packages.py \
  --release-version 0.1.25 \
  --dist-root dist \
  --package codex-potter
```

This writes tarballs to `dist/npm/`.

When `--package codex-potter` is provided, the staging helper builds the
lightweight `codex-potter` meta package plus all platform-native `codex-potter`
variants that are later published under platform-specific dist-tags.

The `--dist-root` directory must contain one artifact directory per requested
target triple, using the same layout produced by `.github/workflows/release.yml`:

```text
dist/
  codex-potter-x86_64-unknown-linux-musl/codex-potter
  codex-potter-aarch64-unknown-linux-musl/codex-potter
  codex-potter-x86_64-apple-darwin/codex-potter
  codex-potter-aarch64-apple-darwin/codex-potter
  codex-potter-x86_64-pc-windows-msvc/codex-potter.exe
  codex-potter-aarch64-pc-windows-msvc/codex-potter.exe
```

If only one platform artifact is available, stage that package explicitly, for
example `--package codex-potter-linux-x64`.

If you need to invoke `npm/scripts/build_npm_package.py` directly, first build a
`vendor/` tree and pass it via `--vendor-src`. Platform packages require the
following layout:

```text
vendor/
  <target-triple>/
    codex-potter/
      codex-potter(.exe)
```

The main `codex-potter` package does not bundle native binaries directly; it
only stages `bin/codex-potter.js`, `README.md`, and optional dependency
metadata pointing at the platform tarballs.
