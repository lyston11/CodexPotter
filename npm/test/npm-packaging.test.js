import assert from "node:assert/strict";
import { execFileSync, execSync, spawnSync } from "node:child_process";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import test from "node:test";
import { fileURLToPath } from "node:url";

const __filename = fileURLToPath(import.meta.url);
const __dirname = path.dirname(__filename);
const repoNpmRoot = path.resolve(__dirname, "..");

const PLATFORM_VARIANTS = [
  {
    platformTag: "linux-x64",
    targetTriple: "x86_64-unknown-linux-musl",
    os: "linux",
    cpu: "x64",
  },
  {
    platformTag: "linux-arm64",
    targetTriple: "aarch64-unknown-linux-musl",
    os: "linux",
    cpu: "arm64",
  },
  {
    platformTag: "darwin-x64",
    targetTriple: "x86_64-apple-darwin",
    os: "darwin",
    cpu: "x64",
  },
  {
    platformTag: "darwin-arm64",
    targetTriple: "aarch64-apple-darwin",
    os: "darwin",
    cpu: "arm64",
  },
  {
    platformTag: "win32-x64",
    targetTriple: "x86_64-pc-windows-msvc",
    os: "win32",
    cpu: "x64",
  },
  {
    platformTag: "win32-arm64",
    targetTriple: "aarch64-pc-windows-msvc",
    os: "win32",
    cpu: "arm64",
  },
];

const currentUnixTargetTriple = getCurrentUnixTargetTriple();
const currentWindowsTargetTriple = getCurrentWindowsTargetTriple();
const currentTargetTriple = currentUnixTargetTriple ?? currentWindowsTargetTriple;
const currentVariant = currentTargetTriple
  ? PLATFORM_VARIANTS.find((variant) => variant.targetTriple === currentTargetTriple) ??
    null
  : null;

const hasBun = isAvailable("bun", ["--version"]);

// Bun's Windows shim behavior has historically been brittle. Keep the bun
// launcher smoke tests disabled on Windows until verified end-to-end.
const supportsBunCmdBinArguments = process.platform !== "win32";

function isAvailable(command, args) {
  const result = spawnSync(command, args, { stdio: "ignore" });
  return !result.error && result.status === 0;
}

function getPythonCommand() {
  const candidates =
    process.platform === "win32" ? ["python", "python3"] : ["python3", "python"];
  for (const candidate of candidates) {
    if (isAvailable(candidate, ["--version"])) {
      return candidate;
    }
  }

  throw new Error("Missing Python on PATH (expected python3 or python).");
}

function stageReleaseTarballs({ distRoot, outputDir, version }) {
  if (!distRoot || !outputDir || !version) {
    throw new Error("stageReleaseTarballs requires distRoot, outputDir, version");
  }

  const python = getPythonCommand();
  const stageScript = path.resolve(repoNpmRoot, "..", "scripts", "stage_npm_packages.py");

  runCommand(
    python,
    [
      stageScript,
      "--release-version",
      version,
      "--dist-root",
      distRoot,
      "--package",
      "codex-potter",
      "--output-dir",
      outputDir,
    ],
    { stdio: "ignore" },
  );

  const mainTarball = path.join(outputDir, `codex-potter-npm-${version}.tgz`);
  const platformTarballs = PLATFORM_VARIANTS.map((variant) =>
    path.join(outputDir, `codex-potter-npm-${variant.platformTag}-${version}.tgz`),
  );
  return { mainTarball, platformTarballs };
}

function writeFile(filePath, contents, mode) {
  fs.mkdirSync(path.dirname(filePath), { recursive: true });
  fs.writeFileSync(filePath, contents);
  if (mode !== undefined) {
    fs.chmodSync(filePath, mode);
  }
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

function extractPackage(tarballPath, extractRoot) {
  fs.mkdirSync(extractRoot, { recursive: true });
  const cwd = process.platform === "win32" ? path.dirname(tarballPath) : undefined;
  const tarballArg = process.platform === "win32" ? path.basename(tarballPath) : tarballPath;
  const extractArg =
    process.platform === "win32" ? path.relative(cwd ?? "", extractRoot) || "." : extractRoot;
  execFileSync("tar", ["-xf", tarballArg, "-C", extractArg], { cwd });
  return path.join(extractRoot, "package");
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

function installPackedMainWithNpm(tarballPath, installRoot) {
  fs.mkdirSync(installRoot, { recursive: true });
  runCommand(getNpmCommand(), ["install", "--omit=optional", "--prefix", installRoot, tarballPath], {
    stdio: "ignore",
  });
  return resolveCommandPath(path.join(installRoot, "node_modules", ".bin", "codex-potter"));
}

function installPackedMainWithBun(tarballPath, installRoot) {
  fs.mkdirSync(installRoot, { recursive: true });
  execFileSync("bun", ["add", "--omit=optional", "--no-save", tarballPath], {
    cwd: installRoot,
    stdio: "ignore",
  });
  return resolveCommandPath(path.join(installRoot, "node_modules", ".bin", "codex-potter"));
}

function installPackedMainGloballyWithNpm(tarballPath, installRoot) {
  fs.mkdirSync(installRoot, { recursive: true });
  runCommand(
    getNpmCommand(),
    ["install", "--global", "--omit=optional", "--prefix", installRoot, tarballPath],
    { stdio: "ignore" },
  );
  return {
    binPath: resolveCommandPath(getGlobalNpmBinBasePath(installRoot)),
    nodeModulesRoot: getGlobalNpmNodeModulesRoot(installRoot),
  };
}

function installPackedMainGloballyWithBun(tarballPath, installRoot) {
  const homeDir = path.join(installRoot, "home");
  const bunInstallDir = path.join(homeDir, ".bun");
  const installEnv = {
    ...process.env,
    HOME: homeDir,
    USERPROFILE: homeDir,
    BUN_INSTALL: bunInstallDir,
  };

  fs.mkdirSync(homeDir, { recursive: true });
  execFileSync("bun", ["add", "--global", "--omit=optional", "--no-save", tarballPath], {
    stdio: "ignore",
    env: installEnv,
  });

  return {
    binPath: resolveCommandPath(path.join(bunInstallDir, "bin", "codex-potter")),
    nodeModulesRoot: path.join(bunInstallDir, "install", "global", "node_modules"),
    env: installEnv,
  };
}

function getGlobalNpmBinBasePath(installRoot) {
  return process.platform === "win32"
    ? path.join(installRoot, "codex-potter")
    : path.join(installRoot, "bin", "codex-potter");
}

function getGlobalNpmNodeModulesRoot(installRoot) {
  const stdout = runCommand(
    getNpmCommand(),
    ["root", "--global", "--prefix", installRoot],
    { encoding: "utf8" },
  );
  return stdout.trim();
}

function installPlatformAliasFromTarball(tarballPath, nodeModulesRoot, aliasName) {
  const extractRoot = fs.mkdtempSync(path.join(os.tmpdir(), "codex-potter-alias-"));
  try {
    const packageRoot = extractPackage(tarballPath, extractRoot);
    const aliasRoot = path.join(nodeModulesRoot, aliasName);
    fs.rmSync(aliasRoot, { recursive: true, force: true });
    fs.cpSync(packageRoot, aliasRoot, { recursive: true });
    return aliasRoot;
  } finally {
    fs.rmSync(extractRoot, { recursive: true, force: true });
  }
}

function prependPath(prefixDirs, basePath = process.env.PATH ?? "") {
  const entries = prefixDirs.filter(Boolean);
  if (basePath) {
    entries.push(basePath);
  }
  return entries.join(path.delimiter);
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

function writeCurrentLauncherBinary(artifactDir, targetTriple, kind) {
  const binaryName = targetTriple.includes("windows") ? "codex-potter.exe" : "codex-potter";
  const binaryPath = path.join(artifactDir, binaryName);

  if (currentUnixTargetTriple) {
    writeFile(
      binaryPath,
      kind === "probe" ? launcherProbeScript() : launcherSmokeScript(),
      0o755,
    );
    return;
  }

  if (!currentWindowsTargetTriple) {
    throw new Error(`Unsupported test platform: ${process.platform} (${process.arch})`);
  }

  fs.mkdirSync(path.dirname(binaryPath), { recursive: true });
  fs.copyFileSync(getWindowsCommandProcessorPath(), binaryPath);
}

function createArtifactFixtures(distRoot, kind) {
  for (const variant of PLATFORM_VARIANTS) {
    const artifactDir = path.join(distRoot, `codex-potter-${variant.targetTriple}`);
    fs.mkdirSync(artifactDir, { recursive: true });

    const binaryName = variant.targetTriple.includes("windows")
      ? "codex-potter.exe"
      : "codex-potter";
    const binaryPath = path.join(artifactDir, binaryName);

    if (currentTargetTriple && variant.targetTriple === currentTargetTriple) {
      writeCurrentLauncherBinary(artifactDir, variant.targetTriple, kind);
      continue;
    }

    const mode =
      process.platform === "win32" || variant.targetTriple.includes("windows") ? undefined : 0o755;
    writeFile(binaryPath, "binary", mode);
  }
}

test("stageReleaseTarballs produces main + platform npm tarballs", () => {
  const tmpdir = fs.mkdtempSync(path.join(os.tmpdir(), "codex-potter-stage-"));

  try {
    const distRoot = path.join(tmpdir, "dist");
    const outputDir = path.join(tmpdir, "npm-dist");

    createArtifactFixtures(distRoot, "smoke");

    const { mainTarball, platformTarballs } = stageReleaseTarballs({
      distRoot,
      outputDir,
      version: "0.1.25",
    });

    assert.ok(fs.existsSync(mainTarball));
    assert.equal(platformTarballs.length, PLATFORM_VARIANTS.length);

    const mainFiles = listTarballFiles(mainTarball);
    assert.ok(mainFiles.includes("package/bin/codex-potter.js"));
    assert.ok(!mainFiles.some((entry) => entry.startsWith("package/vendor/")));

    const mainExtractRoot = path.join(tmpdir, "extract-main");
    const mainPackageRoot = extractPackage(mainTarball, mainExtractRoot);
    const mainPackageJson = JSON.parse(
      fs.readFileSync(path.join(mainPackageRoot, "package.json"), "utf8"),
    );

    assert.equal(mainPackageJson.name, "codex-potter");
    assert.equal(mainPackageJson.version, "0.1.25");
    assert.ok(mainPackageJson.optionalDependencies);
    for (const variant of PLATFORM_VARIANTS) {
      const aliasName = `codex-potter-${variant.platformTag}`;
      assert.equal(
        mainPackageJson.optionalDependencies[aliasName],
        `npm:codex-potter@0.1.25-${variant.platformTag}`,
      );
    }

    for (const variant of PLATFORM_VARIANTS) {
      const tarballPath = path.join(
        outputDir,
        `codex-potter-npm-${variant.platformTag}-0.1.25.tgz`,
      );
      assert.ok(fs.existsSync(tarballPath));

      const extractRoot = path.join(tmpdir, `extract-${variant.platformTag}`);
      const packageRoot = extractPackage(tarballPath, extractRoot);
      const packageJson = JSON.parse(
        fs.readFileSync(path.join(packageRoot, "package.json"), "utf8"),
      );
      assert.equal(packageJson.name, "codex-potter");
      assert.equal(packageJson.version, `0.1.25-${variant.platformTag}`);
      assert.deepEqual(packageJson.os, [variant.os]);
      assert.deepEqual(packageJson.cpu, [variant.cpu]);
      assert.deepEqual(packageJson.files, ["vendor"]);

      const vendorPath = variant.targetTriple.includes("windows")
        ? `package/vendor/${variant.targetTriple}/codex-potter/codex-potter.exe`
        : `package/vendor/${variant.targetTriple}/codex-potter/codex-potter`;
      assert.ok(listTarballFiles(tarballPath).includes(vendorPath));
    }
  } finally {
    fs.rmSync(tmpdir, { recursive: true, force: true });
  }
});

test("stageReleaseTarballs preserves prerelease version suffixes", () => {
  const tmpdir = fs.mkdtempSync(path.join(os.tmpdir(), "codex-potter-stage-"));

  try {
    const distRoot = path.join(tmpdir, "dist");
    const outputDir = path.join(tmpdir, "npm-dist");
    const version = "0.1.25-alpha.1";

    createArtifactFixtures(distRoot, "smoke");

    const { mainTarball, platformTarballs } = stageReleaseTarballs({
      distRoot,
      outputDir,
      version,
    });

    assert.equal(path.basename(mainTarball), `codex-potter-npm-${version}.tgz`);
    assert.deepEqual(
      platformTarballs.map((tarball) => path.basename(tarball)).sort(),
      PLATFORM_VARIANTS.map(
        (variant) => `codex-potter-npm-${variant.platformTag}-${version}.tgz`,
      ).sort(),
    );

    const mainExtractRoot = path.join(tmpdir, "extract-main-prerelease");
    const mainPackageRoot = extractPackage(mainTarball, mainExtractRoot);
    const mainPackageJson = JSON.parse(
      fs.readFileSync(path.join(mainPackageRoot, "package.json"), "utf8"),
    );

    assert.equal(mainPackageJson.version, version);
    for (const variant of PLATFORM_VARIANTS) {
      const aliasName = `codex-potter-${variant.platformTag}`;
      assert.equal(
        mainPackageJson.optionalDependencies[aliasName],
        `npm:codex-potter@${version}-${variant.platformTag}`,
      );

      const tarballPath = path.join(
        outputDir,
        `codex-potter-npm-${variant.platformTag}-${version}.tgz`,
      );
      const extractRoot = path.join(tmpdir, `extract-prerelease-${variant.platformTag}`);
      const packageRoot = extractPackage(tarballPath, extractRoot);
      const packageJson = JSON.parse(
        fs.readFileSync(path.join(packageRoot, "package.json"), "utf8"),
      );
      assert.equal(packageJson.version, `${version}-${variant.platformTag}`);
    }
  } finally {
    fs.rmSync(tmpdir, { recursive: true, force: true });
  }
});

test("stageReleaseTarballs propagates engines metadata", () => {
  const tmpdir = fs.mkdtempSync(path.join(os.tmpdir(), "codex-potter-stage-"));

  try {
    const distRoot = path.join(tmpdir, "dist");
    const outputDir = path.join(tmpdir, "npm-dist");
    createArtifactFixtures(distRoot, "smoke");

    const { mainTarball, platformTarballs } = stageReleaseTarballs({
      distRoot,
      outputDir,
      version: "0.1.25",
    });

    const mainExtractRoot = path.join(tmpdir, "extract-main-metadata");
    const mainPackageRoot = extractPackage(mainTarball, mainExtractRoot);
    const mainPackageJson = JSON.parse(
      fs.readFileSync(path.join(mainPackageRoot, "package.json"), "utf8"),
    );

    assert.deepEqual(mainPackageJson.engines, { node: ">=16" });

    assert.equal(platformTarballs.length, PLATFORM_VARIANTS.length);
    for (const variant of PLATFORM_VARIANTS) {
      const tarballPath = path.join(
        outputDir,
        `codex-potter-npm-${variant.platformTag}-0.1.25.tgz`,
      );
      const extractRoot = path.join(tmpdir, `extract-metadata-${variant.platformTag}`);
      const packageRoot = extractPackage(tarballPath, extractRoot);
      const packageJson = JSON.parse(
        fs.readFileSync(path.join(packageRoot, "package.json"), "utf8"),
      );

      assert.deepEqual(packageJson.engines, { node: ">=16" });
    }
  } finally {
    fs.rmSync(tmpdir, { recursive: true, force: true });
  }
});

test(
  "launcher runs after npm installs main tarball + local platform alias",
  { skip: !currentVariant },
  () => {
    const tmpdir = fs.mkdtempSync(path.join(os.tmpdir(), "codex-potter-stage-"));

    try {
      const distRoot = path.join(tmpdir, "dist");
      const outputDir = path.join(tmpdir, "npm-dist");
      const installRoot = path.join(tmpdir, "install");

      createArtifactFixtures(distRoot, "smoke");

      const { mainTarball } = stageReleaseTarballs({
        distRoot,
        outputDir,
        version: "0.1.25",
      });

      const platformTarball = path.join(
        outputDir,
        `codex-potter-npm-${currentVariant.platformTag}-0.1.25.tgz`,
      );

      const installedBinPath = installPackedMainWithNpm(mainTarball, installRoot);
      installPlatformAliasFromTarball(
        platformTarball,
        path.join(installRoot, "node_modules"),
        `codex-potter-${currentVariant.platformTag}`,
      );

      const launcherOutput = runCommand(installedBinPath, launcherSmokeArgs(), {
        encoding: "utf8",
        env: { ...process.env },
      });
      assert.equal(normalizeOutput(launcherOutput), "launcher smoke ok\n");
    } finally {
      fs.rmSync(tmpdir, { recursive: true, force: true });
    }
  },
);

test(
  "launcher runs after bun installs main tarball + local platform alias",
  { skip: !currentVariant || !hasBun || !supportsBunCmdBinArguments },
  () => {
    const tmpdir = fs.mkdtempSync(path.join(os.tmpdir(), "codex-potter-stage-"));

    try {
      const distRoot = path.join(tmpdir, "dist");
      const outputDir = path.join(tmpdir, "npm-dist");
      const installRoot = path.join(tmpdir, "install");

      createArtifactFixtures(distRoot, "smoke");

      const { mainTarball } = stageReleaseTarballs({
        distRoot,
        outputDir,
        version: "0.1.25",
      });

      const platformTarball = path.join(
        outputDir,
        `codex-potter-npm-${currentVariant.platformTag}-0.1.25.tgz`,
      );

      const installedBinPath = installPackedMainWithBun(mainTarball, installRoot);
      installPlatformAliasFromTarball(
        platformTarball,
        path.join(installRoot, "node_modules"),
        `codex-potter-${currentVariant.platformTag}`,
      );

      const launcherOutput = runCommand(installedBinPath, launcherSmokeArgs(), {
        encoding: "utf8",
        env: { ...process.env },
      });
      assert.equal(normalizeOutput(launcherOutput), "launcher smoke ok\n");
    } finally {
      fs.rmSync(tmpdir, { recursive: true, force: true });
    }
  },
);

test(
  "launcher reports npm-managed env after npm installs main tarball globally + local platform alias",
  { skip: !currentVariant },
  () => {
    const tmpdir = fs.mkdtempSync(path.join(os.tmpdir(), "codex-potter-stage-"));

    try {
      const distRoot = path.join(tmpdir, "dist");
      const outputDir = path.join(tmpdir, "npm-dist");
      const installRoot = path.join(tmpdir, "install");

      createArtifactFixtures(distRoot, "probe");

      const { mainTarball } = stageReleaseTarballs({
        distRoot,
        outputDir,
        version: "0.1.25",
      });

      const platformTarball = path.join(
        outputDir,
        `codex-potter-npm-${currentVariant.platformTag}-0.1.25.tgz`,
      );

      const { binPath, nodeModulesRoot } = installPackedMainGloballyWithNpm(mainTarball, installRoot);
      installPlatformAliasFromTarball(
        platformTarball,
        nodeModulesRoot,
        `codex-potter-${currentVariant.platformTag}`,
      );

      const launcherOutput = runCommand("codex-potter", launcherProbeArgs(), {
        encoding: "utf8",
        env: {
          ...process.env,
          PATH: prependPath([path.dirname(binPath)]),
        },
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
  "launcher reports bun-managed env after bun installs main tarball globally + local platform alias",
  { skip: !currentVariant || !hasBun || !supportsBunCmdBinArguments },
  () => {
    const tmpdir = fs.mkdtempSync(path.join(os.tmpdir(), "codex-potter-stage-"));

    try {
      const distRoot = path.join(tmpdir, "dist");
      const outputDir = path.join(tmpdir, "npm-dist");
      const installRoot = path.join(tmpdir, "install");

      createArtifactFixtures(distRoot, "probe");

      const { mainTarball } = stageReleaseTarballs({
        distRoot,
        outputDir,
        version: "0.1.25",
      });

      const platformTarball = path.join(
        outputDir,
        `codex-potter-npm-${currentVariant.platformTag}-0.1.25.tgz`,
      );

      const { binPath, nodeModulesRoot, env: installEnv } =
        installPackedMainGloballyWithBun(mainTarball, installRoot);

      installPlatformAliasFromTarball(
        platformTarball,
        nodeModulesRoot,
        `codex-potter-${currentVariant.platformTag}`,
      );

      const launcherOutput = runCommand("codex-potter", launcherProbeArgs(), {
        encoding: "utf8",
        env: {
          ...process.env,
          ...installEnv,
          PATH: prependPath([path.dirname(binPath)]),
        },
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

test(
  "launcher suggests bun reinstall after bun global install omits platform alias",
  { skip: !currentVariant || !hasBun },
  () => {
    const tmpdir = fs.mkdtempSync(path.join(os.tmpdir(), "codex-potter-stage-"));

    try {
      const distRoot = path.join(tmpdir, "dist");
      const outputDir = path.join(tmpdir, "npm-dist");
      const installRoot = path.join(tmpdir, "install");

      createArtifactFixtures(distRoot, "smoke");

      const { mainTarball } = stageReleaseTarballs({
        distRoot,
        outputDir,
        version: "0.1.25",
      });

      const { binPath, env: installEnv } = installPackedMainGloballyWithBun(mainTarball, installRoot);

      let installError;
      try {
        runCommand("codex-potter", [], {
          encoding: "utf8",
          env: {
            ...process.env,
            ...installEnv,
            PATH: prependPath([path.dirname(binPath)]),
          },
        });
      } catch (error) {
        installError = error;
      }

      assert.ok(installError);
      const stderr =
        typeof installError.stderr === "string"
          ? installError.stderr
          : installError.stderr?.toString("utf8") ?? "";
      const normalizedStderr = normalizeOutput(stderr);
      assert.ok(
        normalizedStderr.includes(
          `Missing optional dependency codex-potter-${currentVariant.platformTag}. Reinstall CodexPotter: bun install -g codex-potter@latest`,
        ),
      );
    } finally {
      fs.rmSync(tmpdir, { recursive: true, force: true });
    }
  },
);
