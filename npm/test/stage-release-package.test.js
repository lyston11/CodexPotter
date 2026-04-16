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
const currentUnixTargetTriple = getCurrentUnixTargetTriple();

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
        files: ["bin", "vendor", "README.md"],
      },
      null,
      2,
    )}\n`,
  );
  writeFile(
    path.join(npmRoot, "bin", "codex-potter.js"),
    `#!/usr/bin/env node
import { execFileSync } from "node:child_process";
import path from "node:path";
import { fileURLToPath } from "node:url";

const __filename = fileURLToPath(import.meta.url);
const __dirname = path.dirname(__filename);
const binaryPath = path.join(
  __dirname,
  "..",
  "vendor",
  "${currentUnixTargetTriple ?? "unsupported"}",
  "codex-potter",
  "codex-potter",
);

process.stdout.write(execFileSync(binaryPath, process.argv.slice(2), { encoding: "utf8" }));
`,
    0o755,
  );
}

function listTarballFiles(tarballPath) {
  return execFileSync("tar", ["-tf", tarballPath], { encoding: "utf8" })
    .trim()
    .split("\n")
    .filter(Boolean)
    .sort();
}

function packStage(stageRoot, outputDir) {
  const packMetadata = JSON.parse(
    execFileSync("npm", ["pack", "--json", "--pack-destination", outputDir], {
      cwd: stageRoot,
      encoding: "utf8",
    }),
  );
  return path.join(outputDir, packMetadata[0].filename);
}

function getCurrentUnixTargetTriple() {
  switch (process.platform) {
    case "linux":
      switch (process.arch) {
        case "x64":
          return "x86_64-unknown-linux-musl";
        case "arm64":
          return "aarch64-unknown-linux-musl";
        default:
          return null;
      }
    case "darwin":
      switch (process.arch) {
        case "x64":
          return "x86_64-apple-darwin";
        case "arm64":
          return "aarch64-apple-darwin";
        default:
          return null;
      }
    default:
      return null;
  }
}

test(
  "stageReleasePackage packs a runnable launcher fixture",
  { skip: !currentUnixTargetTriple },
  () => {
    const tmpdir = fs.mkdtempSync(path.join(os.tmpdir(), "codex-potter-stage-"));

    try {
      const npmRoot = path.join(tmpdir, "npm-source");
      const distRoot = path.join(tmpdir, "dist");
      const stageRoot = path.join(tmpdir, "stage");
      const extractRoot = path.join(tmpdir, "extract");

      createPackageFixture(npmRoot);
      writeFile(
        path.join(distRoot, `codex-potter-${currentUnixTargetTriple}`, "codex-potter"),
        "#!/bin/sh\nprintf 'fixture smoke ok\\n'\n",
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

      const tarballPath = packStage(stageRoot, tmpdir);
      fs.mkdirSync(extractRoot, { recursive: true });
      execFileSync("tar", ["-xf", tarballPath, "-C", extractRoot]);

      const launcherOutput = execFileSync(
        "node",
        [path.join(extractRoot, "package", "bin", "codex-potter.js"), "--version"],
        { encoding: "utf8" },
      );
      assert.equal(launcherOutput, "fixture smoke ok\n");
    } finally {
      fs.rmSync(tmpdir, { recursive: true, force: true });
    }
  },
);

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

    const tarballPath = packStage(stageRoot, tmpdir);
    assert.ok(
      listTarballFiles(tarballPath).includes(
        "package/vendor/x86_64-pc-windows-msvc/codex-potter/codex-potter.exe",
      ),
    );
  } finally {
    fs.rmSync(tmpdir, { recursive: true, force: true });
  }
});

test(
  "stageReleasePackage launcher runs from the packed repository tarball on the current unix platform",
  { skip: !currentUnixTargetTriple },
  () => {
    const tmpdir = fs.mkdtempSync(path.join(os.tmpdir(), "codex-potter-stage-"));

    try {
      const distRoot = path.join(tmpdir, "dist");
      const stageRoot = path.join(tmpdir, "stage");
      const extractRoot = path.join(tmpdir, "extract");

      writeFile(
        path.join(
          distRoot,
          `codex-potter-${currentUnixTargetTriple}`,
          "nested",
          "codex-potter",
        ),
        "#!/bin/sh\nprintf 'launcher smoke ok\\n'\n",
        0o755,
      );

      stageReleasePackage({
        npmRoot: repoNpmRoot,
        stageRoot,
        distRoot,
        version: "0.1.25",
      });

      const tarballPath = packStage(stageRoot, tmpdir);
      fs.mkdirSync(extractRoot, { recursive: true });
      execFileSync("tar", ["-xf", tarballPath, "-C", extractRoot]);

      const launcherOutput = execFileSync(
        "node",
        [path.join(extractRoot, "package", "bin", "codex-potter.js"), "--version"],
        { encoding: "utf8" },
      );
      assert.equal(launcherOutput, "launcher smoke ok\n");
    } finally {
      fs.rmSync(tmpdir, { recursive: true, force: true });
    }
  },
);
