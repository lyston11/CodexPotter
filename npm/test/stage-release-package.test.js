import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import test from "node:test";
import { fileURLToPath } from "node:url";

import { stageReleasePackage } from "../scripts/stage-release-package.js";

const __filename = fileURLToPath(import.meta.url);
const __dirname = path.dirname(__filename);
const repoNpmRoot = path.resolve(__dirname, "..");

function writeFile(filePath, contents, mode) {
  fs.mkdirSync(path.dirname(filePath), { recursive: true });
  fs.writeFileSync(filePath, contents);
  if (mode !== undefined) {
    fs.chmodSync(filePath, mode);
  }
}

function createPackageFixture(npmRoot) {
  writeFile(path.join(npmRoot, "README.md"), "# fixture\n");
  writeFile(
    path.join(npmRoot, "package.json"),
    `${JSON.stringify(
      {
        name: "codex-potter",
        version: "0.0.0-dev",
        type: "module",
        bin: {
          "codex-potter": "bin/codex-potter.js",
        },
        files: ["bin", "lib", "vendor", "README.md"],
      },
      null,
      2,
    )}\n`,
  );
  writeFile(
    path.join(npmRoot, "bin", "codex-potter.js"),
    '#!/usr/bin/env node\nimport "../lib/signal-exit.js";\n',
    0o755,
  );
  writeFile(
    path.join(npmRoot, "lib", "signal-exit.js"),
    "export function reemitSignalOrExit() {}\n",
  );
}

function listTarballFiles(tarballPath) {
  return execFileSync("tar", ["-tf", tarballPath], { encoding: "utf8" })
    .trim()
    .split("\n")
    .filter(Boolean)
    .sort();
}

test("stageReleasePackage keeps runtime lib files in the packed tarball", () => {
  const tmpdir = fs.mkdtempSync(path.join(os.tmpdir(), "codex-potter-stage-"));

  try {
    const npmRoot = path.join(tmpdir, "npm-source");
    const distRoot = path.join(tmpdir, "dist");
    const stageRoot = path.join(tmpdir, "stage");

    createPackageFixture(npmRoot);
    writeFile(
      path.join(distRoot, "codex-potter-x86_64-unknown-linux-musl", "codex-potter"),
      "#!/bin/sh\nexit 0\n",
      0o755,
    );

    stageReleasePackage({
      npmRoot,
      stageRoot,
      distRoot,
      version: "0.1.25",
    });

    const stagedPackageJson = JSON.parse(
      fs.readFileSync(path.join(stageRoot, "package.json"), "utf8"),
    );
    assert.equal(stagedPackageJson.version, "0.1.25");
    assert.equal(
      fs.readFileSync(path.join(stageRoot, "lib", "signal-exit.js"), "utf8"),
      "export function reemitSignalOrExit() {}\n",
    );

    const packMetadata = JSON.parse(
      execFileSync("npm", ["pack", "--json", "--pack-destination", tmpdir], {
        cwd: stageRoot,
        encoding: "utf8",
      }),
    );
    const tarballPath = path.join(tmpdir, packMetadata[0].filename);

    assert.deepEqual(listTarballFiles(tarballPath), [
      "package/README.md",
      "package/bin/codex-potter.js",
      "package/lib/signal-exit.js",
      "package/package.json",
      "package/vendor/x86_64-unknown-linux-musl/codex-potter/codex-potter",
    ]);
  } finally {
    fs.rmSync(tmpdir, { recursive: true, force: true });
  }
});

test("stageReleasePackage preserves Windows executable names", () => {
  const tmpdir = fs.mkdtempSync(path.join(os.tmpdir(), "codex-potter-stage-"));

  try {
    const npmRoot = path.join(tmpdir, "npm-source");
    const distRoot = path.join(tmpdir, "dist");
    const stageRoot = path.join(tmpdir, "stage");

    createPackageFixture(npmRoot);
    writeFile(
      path.join(
        distRoot,
        "codex-potter-x86_64-pc-windows-msvc",
        "nested",
        "codex-potter.exe",
      ),
      "binary",
    );

    stageReleasePackage({
      npmRoot,
      stageRoot,
      distRoot,
      version: "0.1.25",
    });

    assert.equal(
      fs.readFileSync(
        path.join(
          stageRoot,
          "vendor",
          "x86_64-pc-windows-msvc",
          "codex-potter",
          "codex-potter.exe",
        ),
        "utf8",
      ),
      "binary",
    );
  } finally {
    fs.rmSync(tmpdir, { recursive: true, force: true });
  }
});

test("stageReleasePackage keeps the repository runtime files in the packed tarball", () => {
  const tmpdir = fs.mkdtempSync(path.join(os.tmpdir(), "codex-potter-stage-"));

  try {
    const distRoot = path.join(tmpdir, "dist");
    const stageRoot = path.join(tmpdir, "stage");

    writeFile(
      path.join(distRoot, "codex-potter-x86_64-unknown-linux-musl", "nested", "codex-potter"),
      "#!/bin/sh\nexit 0\n",
      0o755,
    );

    stageReleasePackage({
      npmRoot: repoNpmRoot,
      stageRoot,
      distRoot,
      version: "0.1.25",
    });

    const packMetadata = JSON.parse(
      execFileSync("npm", ["pack", "--json", "--pack-destination", tmpdir], {
        cwd: stageRoot,
        encoding: "utf8",
      }),
    );
    const tarballPath = path.join(tmpdir, packMetadata[0].filename);
    const tarballFiles = listTarballFiles(tarballPath);

    assert.ok(tarballFiles.includes("package/bin/codex-potter.js"));
    assert.ok(tarballFiles.includes("package/lib/signal-exit.js"));
    assert.ok(
      tarballFiles.includes(
        "package/vendor/x86_64-unknown-linux-musl/codex-potter/codex-potter",
      ),
    );
  } finally {
    fs.rmSync(tmpdir, { recursive: true, force: true });
  }
});
