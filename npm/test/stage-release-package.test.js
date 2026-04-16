import assert from "node:assert/strict";
import { execFileSync, execSync, spawnSync } from "node:child_process";
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
const currentWindowsTargetTriple = getCurrentWindowsTargetTriple();
const currentTargetTriple = currentUnixTargetTriple ?? currentWindowsTargetTriple;
const hasBun = isAvailable("bun", ["--version"]);
const requiredUnixRuntimeCommands = ["readlink", "uname"];
// Real Windows verification showed Bun's generated `.exe` shims can start a
// `.cmd` package bin with no args, but fail as soon as arguments are forwarded
// through the shim (`The system cannot find the path specified.`).
const supportsBunCmdBinArguments = process.platform !== "win32";

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
"%~dp0..\\vendor\\${currentWindowsTargetTriple ?? "unsupported"}\\codex-potter\\codex-potter.exe" %*
`,
    0o755,
  );
}

function listTarballFiles(tarballPath) {
  const tarballArgs =
    process.platform === "win32" ? [path.basename(tarballPath)] : [tarballPath];
  const cwd = process.platform === "win32" ? path.dirname(tarballPath) : undefined;
  return execFileSync("tar", ["-tf", ...tarballArgs], { cwd, encoding: "utf8" })
    .trim()
    .split(/\r?\n/)
    .filter(Boolean)
    .sort();
}

function getNpmCommand() {
  if (process.platform !== "win32") {
    return "npm";
  }

  const npmCmd = path.join(path.dirname(process.execPath), "npm.cmd");
  if (!fs.existsSync(npmCmd)) {
    throw new Error(`Missing npm.cmd next to node: ${npmCmd}`);
  }

  return npmCmd;
}

function packStage(stageRoot, outputDir) {
  const packMetadata = JSON.parse(
    runCommand(getNpmCommand(), ["pack", "--json", "--pack-destination", outputDir], {
      cwd: stageRoot,
      encoding: "utf8",
    }),
  );
  return path.join(outputDir, packMetadata[0].filename);
}

function extractPackage(tarballPath, extractRoot) {
  fs.mkdirSync(extractRoot, { recursive: true });
  const cwd = process.platform === "win32" ? path.dirname(tarballPath) : undefined;
  const tarballArg = process.platform === "win32" ? path.basename(tarballPath) : tarballPath;
  const extractArg =
    process.platform === "win32" ? path.relative(cwd ?? "", extractRoot) || "." : extractRoot;
  execFileSync("tar", ["-xf", tarballArg, "-C", extractArg], { cwd });
  return path.join(extractRoot, "package");
}

function installPackedPackageWithNpm(tarballPath, installRoot) {
  fs.mkdirSync(installRoot, { recursive: true });
  runCommand(getNpmCommand(), ["install", "--prefix", installRoot, tarballPath], {
    stdio: "ignore",
  });
  return resolveCommandPath(path.join(installRoot, "node_modules", ".bin", "codex-potter"));
}

function installPackedPackageWithBun(tarballPath, installRoot) {
  fs.mkdirSync(installRoot, { recursive: true });
  execFileSync("bun", ["add", tarballPath], {
    cwd: installRoot,
    stdio: "ignore",
  });
  return resolveCommandPath(path.join(installRoot, "node_modules", ".bin", "codex-potter"));
}

function installPackedPackageGloballyWithBun(tarballPath, installRoot) {
  const homeDir = path.join(installRoot, "home");
  const bunInstallDir = path.join(homeDir, ".bun");
  const installEnv = {
    ...process.env,
    HOME: homeDir,
    USERPROFILE: homeDir,
    BUN_INSTALL: bunInstallDir,
  };

  fs.mkdirSync(homeDir, { recursive: true });
  execFileSync("bun", ["install", "-g", tarballPath], {
    stdio: "ignore",
    env: installEnv,
  });

  return {
    binPath: resolveCommandPath(path.join(bunInstallDir, "bin", "codex-potter")),
    env: installEnv,
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

function getBunRuntimePath(runtimePath) {
  const bunCommand = findCommandOnPath(process.platform === "win32" ? "bun.exe" : "bun");
  const pathEntries = [path.dirname(bunCommand)];
  if (runtimePath) {
    pathEntries.push(runtimePath);
  }
  return pathEntries.join(path.delimiter);
}

function createUnixRuntimeBin(root) {
  const runtimeBin = path.join(root, "runtime-bin");
  fs.mkdirSync(runtimeBin, { recursive: true });

  for (const command of requiredUnixRuntimeCommands) {
    fs.symlinkSync(findCommandOnPath(command), path.join(runtimeBin, command));
  }

  return runtimeBin;
}

function launcherSmokeScript() {
  return "#!/bin/sh\nprintf 'launcher smoke ok\\n'\n";
}

function launcherProbeScript() {
  return `#!/bin/sh
printf 'launcher smoke ok\\n'
printf 'npm=%s bun=%s\\n' "\${CODEX_POTTER_MANAGED_BY_NPM-}" "\${CODEX_POTTER_MANAGED_BY_BUN-}"
`;
}

function expectedLauncherOutput({ managedByNpm, managedByBun }) {
  if (process.platform === "win32") {
    let managedByLine = "";
    if (managedByNpm === "1") {
      managedByLine = "CODEX_POTTER_MANAGED_BY_NPM=1\n";
    } else if (managedByBun === "1") {
      managedByLine = "CODEX_POTTER_MANAGED_BY_BUN=1\n";
    }

    return `launcher smoke ok\n${managedByLine}`;
  }

  return `launcher smoke ok\nnpm=${managedByNpm} bun=${managedByBun}\n`;
}

function installPackedPackageGloballyWithNpm(tarballPath, installRoot) {
  fs.mkdirSync(installRoot, { recursive: true });
  runCommand(getNpmCommand(), ["install", "--global", "--prefix", installRoot, tarballPath], {
    stdio: "ignore",
  });
  return {
    binPath: resolveCommandPath(getGlobalNpmBinBasePath(installRoot)),
  };
}

function resolveCommandPath(basePath) {
  const candidates =
    process.platform === "win32"
      ? [`${basePath}.cmd`, `${basePath}.exe`, basePath]
      : [basePath];

  for (const candidate of candidates) {
    if (fs.existsSync(candidate)) {
      return candidate;
    }
  }

  throw new Error(`Missing installed command: ${candidates.join(", ")}`);
}

function getGlobalNpmBinBasePath(installRoot) {
  return process.platform === "win32"
    ? path.join(installRoot, "codex-potter")
    : path.join(installRoot, "bin", "codex-potter");
}

function getCurrentWindowsTargetTriple() {
  if (process.platform !== "win32") {
    return null;
  }

  switch (process.arch) {
    case "x64":
      return "x86_64-pc-windows-msvc";
    case "arm64":
      return "aarch64-pc-windows-msvc";
    default:
      return null;
  }
}

function getWindowsCommandProcessorPath() {
  const candidate =
    process.env.ComSpec ??
    path.join(process.env.SystemRoot ?? "C:\\Windows", "System32", "cmd.exe");

  if (!fs.existsSync(candidate)) {
    throw new Error(`Missing Windows command processor: ${candidate}`);
  }

  return candidate;
}

function quoteWindowsCmdArgument(argument) {
  if (argument.length === 0) {
    return '""';
  }

  if (!/[\s"&()<>^|]/.test(argument)) {
    return argument;
  }

  return `"${argument.replaceAll('"', '""')}"`;
}

function buildWindowsCommandInvocation(command, args) {
  return [command, ...args].map(quoteWindowsCmdArgument).join(" ");
}

function quotePosixShellArgument(argument) {
  if (argument.length === 0) {
    return "''";
  }

  return `'${argument.replaceAll("'", `'"'"'`)}'`;
}

function buildPosixShellInvocation(command, args) {
  return [command, ...args].map(quotePosixShellArgument).join(" ");
}

function runCommand(command, args, options) {
  if (process.platform !== "win32") {
    try {
      return execFileSync(command, args, options);
    } catch (error) {
      if (error?.code !== "ENOEXEC") {
        throw error;
      }

      return execFileSync("/bin/sh", ["-c", buildPosixShellInvocation(command, args)], options);
    }
  }

  // Let cmd.exe parse the already-escaped command line itself; feeding the same
  // payload back through execFileSync(cmd.exe, argv) adds another quoting layer
  // in Node's CreateProcess bridge and breaks real Windows `.cmd` launchers.
  return execSync(buildWindowsCommandInvocation(command, args), {
    ...options,
    shell: getWindowsCommandProcessorPath(),
  });
}

function normalizeOutput(output) {
  return output.replaceAll("\r\n", "\n");
}

function stageLauncherBinary(distRoot, kind) {
  if (currentUnixTargetTriple) {
    writeFile(
      path.join(distRoot, `codex-potter-${currentUnixTargetTriple}`, "codex-potter"),
      kind === "probe" ? launcherProbeScript() : launcherSmokeScript(),
      0o755,
    );
    return;
  }

  if (!currentWindowsTargetTriple) {
    throw new Error(`Unsupported test platform: ${process.platform} (${process.arch})`);
  }

  const binaryPath = path.join(
    distRoot,
    `codex-potter-${currentWindowsTargetTriple}`,
    "codex-potter.exe",
  );
  fs.mkdirSync(path.dirname(binaryPath), { recursive: true });
  fs.copyFileSync(getWindowsCommandProcessorPath(), binaryPath);
}

function launcherSmokeArgs() {
  return process.platform === "win32"
    ? ["/d", "/s", "/c", "echo launcher smoke ok"]
    : ["--version"];
}

function launcherProbeArgs() {
  return process.platform === "win32"
    ? ["/d", "/s", "/c", "echo launcher smoke ok&& set CODEX_POTTER_MANAGED_BY"]
    : ["--version"];
}

function createRuntimePath(root) {
  return process.platform === "win32" ? "" : createUnixRuntimeBin(root);
}

function createRuntimeEnv(runtimePath, extraEnv = {}) {
  return {
    ...process.env,
    ...extraEnv,
    PATH: extraEnv.PATH ?? runtimePath,
  };
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
  { skip: !currentTargetTriple },
  () => {
    const tmpdir = fs.mkdtempSync(path.join(os.tmpdir(), "codex-potter-stage-"));

    try {
      const npmRoot = path.join(tmpdir, "npm-source");
      const distRoot = path.join(tmpdir, "dist");
      const stageRoot = path.join(tmpdir, "stage");
      const extractRoot = path.join(tmpdir, "extract");

      createPackageFixture(npmRoot);
      stageLauncherBinary(distRoot, "smoke");

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

      const launcherOutput = runCommand(
        path.join(packageRoot, "bin", "codex-potter.cmd"),
        launcherSmokeArgs(),
        { encoding: "utf8" },
      );
      assert.equal(normalizeOutput(launcherOutput), "launcher smoke ok\n");
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
  { skip: !currentTargetTriple },
  () => {
    const tmpdir = fs.mkdtempSync(path.join(os.tmpdir(), "codex-potter-stage-"));

    try {
      const distRoot = path.join(tmpdir, "dist");
      const stageRoot = path.join(tmpdir, "stage");
      const installRoot = path.join(tmpdir, "install");
      const runtimePath = createRuntimePath(tmpdir);

      stageLauncherBinary(distRoot, "smoke");

      stageReleasePackage({
        npmRoot: repoNpmRoot,
        stageRoot,
        distRoot,
        version: "0.1.25",
      });

      const tarballPath = packStage(stageRoot, tmpdir);
      const installedBinPath = installPackedPackageWithNpm(tarballPath, installRoot);

      const launcherOutput = runCommand(installedBinPath, launcherSmokeArgs(), {
        encoding: "utf8",
        env: createRuntimeEnv(runtimePath),
      });
      assert.equal(normalizeOutput(launcherOutput), "launcher smoke ok\n");
    } finally {
      fs.rmSync(tmpdir, { recursive: true, force: true });
    }
  },
);

test(
  "stageReleasePackage launcher runs after bun installs the packed repository tarball without node on PATH",
  { skip: !currentTargetTriple || !hasBun || !supportsBunCmdBinArguments },
  () => {
    const tmpdir = fs.mkdtempSync(path.join(os.tmpdir(), "codex-potter-stage-"));

    try {
      const distRoot = path.join(tmpdir, "dist");
      const stageRoot = path.join(tmpdir, "stage");
      const installRoot = path.join(tmpdir, "install");
      const runtimePath = createRuntimePath(tmpdir);

      stageLauncherBinary(distRoot, "smoke");

      stageReleasePackage({
        npmRoot: repoNpmRoot,
        stageRoot,
        distRoot,
        version: "0.1.25",
      });

      const tarballPath = packStage(stageRoot, tmpdir);
      const installedBinPath = installPackedPackageWithBun(tarballPath, installRoot);

      const launcherOutput = runCommand(installedBinPath, launcherSmokeArgs(), {
        encoding: "utf8",
        env: createRuntimeEnv(runtimePath, {
          PATH: getBunRuntimePath(runtimePath),
        }),
      });
      assert.equal(normalizeOutput(launcherOutput), "launcher smoke ok\n");
    } finally {
      fs.rmSync(tmpdir, { recursive: true, force: true });
    }
  },
);

test(
  "stageReleasePackage launcher reports npm-managed env after npm installs the packed repository tarball globally without node on PATH",
  { skip: !currentTargetTriple },
  () => {
    const tmpdir = fs.mkdtempSync(path.join(os.tmpdir(), "codex-potter-stage-"));

    try {
      const distRoot = path.join(tmpdir, "dist");
      const stageRoot = path.join(tmpdir, "stage");
      const installRoot = path.join(tmpdir, "install");
      const runtimePath = createRuntimePath(tmpdir);

      stageLauncherBinary(distRoot, "probe");

      stageReleasePackage({
        npmRoot: repoNpmRoot,
        stageRoot,
        distRoot,
        version: "0.1.25",
      });

      const tarballPath = packStage(stageRoot, tmpdir);
      const { binPath } = installPackedPackageGloballyWithNpm(tarballPath, installRoot);

      const launcherOutput = runCommand("codex-potter", launcherProbeArgs(), {
        encoding: "utf8",
        env: createRuntimeEnv(runtimePath, {
          PATH: runtimePath
            ? [path.dirname(binPath), runtimePath].join(path.delimiter)
            : path.dirname(binPath),
        }),
      });

      assert.equal(
        normalizeOutput(launcherOutput),
        expectedLauncherOutput({ managedByNpm: "1", managedByBun: "" }),
      );
    } finally {
      fs.rmSync(tmpdir, { recursive: true, force: true });
    }
  },
);

test(
  "stageReleasePackage launcher reports bun-managed env after bun installs the packed repository tarball globally without node on PATH",
  { skip: !currentTargetTriple || !hasBun || !supportsBunCmdBinArguments },
  () => {
    const tmpdir = fs.mkdtempSync(path.join(os.tmpdir(), "codex-potter-stage-"));

    try {
      const distRoot = path.join(tmpdir, "dist");
      const stageRoot = path.join(tmpdir, "stage");
      const installRoot = path.join(tmpdir, "install");
      const runtimePath = createRuntimePath(tmpdir);

      stageLauncherBinary(distRoot, "probe");

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

      const launcherOutput = runCommand("codex-potter", launcherProbeArgs(), {
        encoding: "utf8",
        env: createRuntimeEnv(runtimePath, {
          ...installEnv,
          PATH: [path.dirname(binPath), getBunRuntimePath(runtimePath)]
            .filter(Boolean)
            .join(path.delimiter),
        }),
      });

      assert.equal(
        normalizeOutput(launcherOutput),
        expectedLauncherOutput({ managedByNpm: "", managedByBun: "1" }),
      );
    } finally {
      fs.rmSync(tmpdir, { recursive: true, force: true });
    }
  },
);
