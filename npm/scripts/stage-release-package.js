import fs from "node:fs";
import path from "node:path";
import os from "node:os";
import { execFileSync } from "node:child_process";
import { fileURLToPath } from "node:url";

const __filename = fileURLToPath(import.meta.url);
const __dirname = path.dirname(__filename);

const PACKAGE_NAME = "codex-potter";

export const PLATFORM_VARIANTS = [
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

function readPackageJson(packageJsonPath) {
  return JSON.parse(fs.readFileSync(packageJsonPath, "utf8"));
}

function writePackageJson(packageJsonPath, packageJson) {
  fs.writeFileSync(packageJsonPath, JSON.stringify(packageJson, null, 2) + "\n");
}

function packStage(stageRoot, outputDir) {
  const packMetadata = JSON.parse(
    execFileSync(
      getNpmCommand(),
      ["pack", "--json", "--pack-destination", outputDir],
      { cwd: stageRoot, encoding: "utf8" },
    ),
  );

  const filename = packMetadata?.[0]?.filename;
  if (!filename) {
    throw new Error(`Failed to determine npm pack output for ${stageRoot}`);
  }

  return path.join(outputDir, filename);
}

function aliasNameForPlatform(platformTag) {
  return `${PACKAGE_NAME}-${platformTag}`;
}

function computePlatformPackageVersion(version, platformTag) {
  // npm forbids republishing the same package name/version, so each platform
  // tarball needs a unique version string.
  return `${version}-${platformTag}`;
}

function copyArtifactBinary({ distRoot, stageRoot, targetTriple }) {
  const artifactDir = path.join(distRoot, `codex-potter-${targetTriple}`);
  const binaryName = targetTriple.includes("windows")
    ? "codex-potter.exe"
    : "codex-potter";
  const binarySource = path.join(artifactDir, binaryName);

  if (!fs.existsSync(binarySource)) {
    throw new Error(`Missing ${binaryName} in ${artifactDir}`);
  }

  const vendorTargetDir = path.join(
    stageRoot,
    "vendor",
    targetTriple,
    "codex-potter",
  );
  fs.mkdirSync(vendorTargetDir, { recursive: true });

  const binaryDestination = path.join(vendorTargetDir, binaryName);
  fs.copyFileSync(binarySource, binaryDestination);
  if (!targetTriple.includes("windows") && process.platform !== "win32") {
    fs.chmodSync(binaryDestination, 0o755);
  }
}

function stageMainPackage({ npmRoot, stageRoot, version }) {
  fs.rmSync(stageRoot, { recursive: true, force: true });
  fs.mkdirSync(stageRoot, { recursive: true });

  const binSource = path.join(npmRoot, "bin");
  if (!fs.existsSync(binSource)) {
    throw new Error(`Missing npm bin directory: ${binSource}`);
  }
  fs.cpSync(binSource, path.join(stageRoot, "bin"), { recursive: true });

  const readmeSource = path.join(npmRoot, "README.md");
  if (fs.existsSync(readmeSource)) {
    fs.copyFileSync(readmeSource, path.join(stageRoot, "README.md"));
  }

  const packageJsonPath = path.join(npmRoot, "package.json");
  const packageJson = readPackageJson(packageJsonPath);
  packageJson.name = PACKAGE_NAME;
  packageJson.version = version;
  packageJson.files = ["bin"];
  packageJson.optionalDependencies = Object.fromEntries(
    PLATFORM_VARIANTS.map((variant) => {
      const aliasName = aliasNameForPlatform(variant.platformTag);
      const platformVersion = computePlatformPackageVersion(
        version,
        variant.platformTag,
      );
      return [aliasName, `npm:${PACKAGE_NAME}@${platformVersion}`];
    }),
  );

  writePackageJson(path.join(stageRoot, "package.json"), packageJson);
}

function stagePlatformPackage({
  npmRoot,
  distRoot,
  stageRoot,
  version,
  platformTag,
  targetTriple,
  os: npmOs,
  cpu: npmCpu,
}) {
  fs.rmSync(stageRoot, { recursive: true, force: true });
  fs.mkdirSync(stageRoot, { recursive: true });

  const sourcePackageJson = readPackageJson(path.join(npmRoot, "package.json"));

  const readmeSource = path.join(npmRoot, "README.md");
  if (fs.existsSync(readmeSource)) {
    fs.copyFileSync(readmeSource, path.join(stageRoot, "README.md"));
  }

  const packageJson = {
    name: PACKAGE_NAME,
    version: computePlatformPackageVersion(version, platformTag),
    license: sourcePackageJson.license ?? "Apache-2.0",
    os: [npmOs],
    cpu: [npmCpu],
    files: ["vendor"],
    repository: sourcePackageJson.repository,
  };

  if (typeof sourcePackageJson.packageManager === "string") {
    packageJson.packageManager = sourcePackageJson.packageManager;
  }

  if (
    sourcePackageJson.engines &&
    typeof sourcePackageJson.engines === "object" &&
    !Array.isArray(sourcePackageJson.engines)
  ) {
    packageJson.engines = sourcePackageJson.engines;
  }

  writePackageJson(path.join(stageRoot, "package.json"), packageJson);

  copyArtifactBinary({ distRoot, stageRoot, targetTriple });
}

/**
 * Build release tarballs for the main npm package and per-platform optional
 * dependencies (upstream-aligned design).
 *
 * Output filenames:
 * - codex-potter-npm-<version>.tgz
 * - codex-potter-npm-<platformTag>-<version>.tgz
 */
export function buildReleaseTarballs({ npmRoot, distRoot, outputDir, version }) {
  if (!npmRoot || !distRoot || !outputDir || !version) {
    throw new Error("buildReleaseTarballs requires npmRoot, distRoot, outputDir, version");
  }

  fs.mkdirSync(outputDir, { recursive: true });

  const stagingRoot = fs.mkdtempSync(
    path.join(os.tmpdir(), "codex-potter-npm-stage-"),
  );

  try {
    const mainStageRoot = path.join(stagingRoot, "main");
    stageMainPackage({ npmRoot, stageRoot: mainStageRoot, version });
    const mainTarball = packStage(mainStageRoot, outputDir);
    const mainOut = path.join(outputDir, `codex-potter-npm-${version}.tgz`);
    fs.renameSync(mainTarball, mainOut);

    const platformTarballs = [];
    for (const variant of PLATFORM_VARIANTS) {
      const stageRoot = path.join(stagingRoot, `platform-${variant.platformTag}`);
      stagePlatformPackage({
        npmRoot,
        distRoot,
        stageRoot,
        version,
        ...variant,
      });

      const tarball = packStage(stageRoot, outputDir);
      const outPath = path.join(
        outputDir,
        `codex-potter-npm-${variant.platformTag}-${version}.tgz`,
      );
      fs.renameSync(tarball, outPath);
      platformTarballs.push(outPath);
    }

    return { mainTarball: mainOut, platformTarballs };
  } finally {
    fs.rmSync(stagingRoot, { recursive: true, force: true });
  }
}

function main(argv) {
  if (argv.length < 2 || argv.length > 3) {
    throw new Error(
      "Usage: node npm/scripts/stage-release-package.js <dist-root> <version> [output-dir]",
    );
  }

  const [distRoot, version, outputDirArg] = argv;
  const outputDir = outputDirArg
    ? path.resolve(outputDirArg)
    : path.resolve(__dirname, "..", "..", "dist", "npm");

  buildReleaseTarballs({
    npmRoot: path.resolve(__dirname, ".."),
    distRoot: path.resolve(distRoot),
    outputDir,
    version,
  });
}

if (process.argv[1] && path.resolve(process.argv[1]) === __filename) {
  main(process.argv.slice(2));
}
