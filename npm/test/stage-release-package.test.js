import assert from "node:assert/strict";
import { execFileSync, spawnSync } from "node:child_process";
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
const hasBun = isAvailable("bun", ["--version"]);
const requiredUnixRuntimeCommands = ["readlink", "uname"];

function isAvailable(command, args) {
  const result = spawnSync(command, args, { stdio: "ignore" });
  return !result.error && result.status === 0;
}

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
          "codex-potter": "bin/codex-potter.cmd",
        },
        files: ["bin", "vendor", "README.md"],
      },
      null,
      2,
    )}\n`,
  );
  writeFile(
    path.join(npmRoot, "bin", "codex-potter.cmd"),
    `: <<'::CMDLITERAL'
@goto :batch
::CMDLITERAL
basedir=\${0%/*}
[ "$basedir" = "$0" ] && basedir=.
exec "$basedir/../vendor/${currentUnixTargetTriple ?? "unsupported"}/codex-potter/codex-potter" "$@"
exit 1
:batch
@echo off
setlocal
"%~dp0..\\vendor\\x86_64-pc-windows-msvc\\codex-potter\\codex-potter.exe" %*
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

function extractPackage(tarballPath, extractRoot) {
  fs.mkdirSync(extractRoot, { recursive: true });
  execFileSync("tar", ["-xf", tarballPath, "-C", extractRoot]);
  return path.join(extractRoot, "package");
}

function installPackedPackageWithNpm(tarballPath, installRoot) {
  fs.mkdirSync(installRoot, { recursive: true });
  execFileSync("npm", ["install", "--prefix", installRoot, tarballPath], {
    stdio: "ignore",
  });
  return path.join(installRoot, "node_modules", ".bin", "codex-potter");
}

function installPackedPackageWithBun(tarballPath, installRoot) {
  fs.mkdirSync(installRoot, { recursive: true });
  execFileSync("bun", ["add", tarballPath], {
    cwd: installRoot,
    stdio: "ignore",
  });
  return path.join(installRoot, "node_modules", ".bin", "codex-potter");
}

function installPackedPackageGloballyWithBun(tarballPath, installRoot) {
  const homeDir = path.join(installRoot, "home");
  const bunInstallDir = path.join(homeDir, ".bun");

  fs.mkdirSync(homeDir, { recursive: true });
  execFileSync("bun", ["install", "-g", tarballPath], {
    stdio: "ignore",
    env: {
      ...process.env,
      HOME: homeDir,
      BUN_INSTALL: bunInstallDir,
    },
  });

  return {
    binPath: path.join(bunInstallDir, "bin", "codex-potter"),
    env: {
      HOME: homeDir,
      BUN_INSTALL: bunInstallDir,
    },
  };
}

function findCommandOnPath(command) {
  for (const dir of (process.env.PATH ?? "").split(path.delimiter).filter(Boolean)) {
    const candidate = path.join(dir, command);
    if (fs.existsSync(candidate)) {
      return fs.realpathSync(candidate);
    }
  }

  throw new Error(`Missing required runtime command: ${command}`);
}

function createUnixRuntimeBin(root) {
  const runtimeBin = path.join(root, "runtime-bin");
  fs.mkdirSync(runtimeBin, { recursive: true });

  for (const command of requiredUnixRuntimeCommands) {
    fs.symlinkSync(findCommandOnPath(command), path.join(runtimeBin, command));
  }

  return runtimeBin;
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
      const packageRoot = extractPackage(tarballPath, extractRoot);

      const launcherOutput = execFileSync(
        path.join(packageRoot, "bin", "codex-potter.cmd"),
        ["--version"],
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
  "stageReleasePackage launcher runs after npm installs the packed repository tarball without node on PATH",
  { skip: !currentUnixTargetTriple },
  () => {
    const tmpdir = fs.mkdtempSync(path.join(os.tmpdir(), "codex-potter-stage-"));

    try {
      const distRoot = path.join(tmpdir, "dist");
      const stageRoot = path.join(tmpdir, "stage");
      const installRoot = path.join(tmpdir, "install");

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
      const installedBinPath = installPackedPackageWithNpm(tarballPath, installRoot);
      const runtimeBin = createUnixRuntimeBin(tmpdir);

      const launcherOutput = execFileSync(installedBinPath, ["--version"], {
        encoding: "utf8",
        env: {
          ...process.env,
          PATH: runtimeBin,
        },
      });
      assert.equal(launcherOutput, "launcher smoke ok\n");
    } finally {
      fs.rmSync(tmpdir, { recursive: true, force: true });
    }
  },
);

test(
  "stageReleasePackage launcher runs after bun installs the packed repository tarball without node on PATH",
  { skip: !currentUnixTargetTriple || !hasBun },
  () => {
    const tmpdir = fs.mkdtempSync(path.join(os.tmpdir(), "codex-potter-stage-"));

    try {
      const distRoot = path.join(tmpdir, "dist");
      const stageRoot = path.join(tmpdir, "stage");
      const installRoot = path.join(tmpdir, "install");

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
      const installedBinPath = installPackedPackageWithBun(tarballPath, installRoot);
      const runtimeBin = createUnixRuntimeBin(tmpdir);

      const launcherOutput = execFileSync(installedBinPath, ["--version"], {
        encoding: "utf8",
        env: {
          ...process.env,
          PATH: runtimeBin,
        },
      });
      assert.equal(launcherOutput, "launcher smoke ok\n");
    } finally {
      fs.rmSync(tmpdir, { recursive: true, force: true });
    }
  },
);

test(
  "stageReleasePackage launcher runs after bun installs the packed repository tarball globally without node on PATH",
  { skip: !currentUnixTargetTriple || !hasBun },
  () => {
    const tmpdir = fs.mkdtempSync(path.join(os.tmpdir(), "codex-potter-stage-"));

    try {
      const distRoot = path.join(tmpdir, "dist");
      const stageRoot = path.join(tmpdir, "stage");
      const installRoot = path.join(tmpdir, "install");
      const runtimeBin = createUnixRuntimeBin(tmpdir);

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
      const { binPath, env: installEnv } = installPackedPackageGloballyWithBun(
        tarballPath,
        installRoot,
      );

      const launcherOutput = execFileSync("codex-potter", ["--version"], {
        encoding: "utf8",
        env: {
          ...process.env,
          ...installEnv,
          PATH: [path.dirname(binPath), runtimeBin].join(path.delimiter),
        },
      });

      assert.equal(launcherOutput, "launcher smoke ok\n");
    } finally {
      fs.rmSync(tmpdir, { recursive: true, force: true });
    }
  },
);
