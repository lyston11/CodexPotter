import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const __filename = fileURLToPath(import.meta.url);
const __dirname = path.dirname(__filename);

function copyArtifactBinary(artifactDir, vendorRoot) {
  const target = path.basename(artifactDir).replace(/^codex-potter-/, "");
  const vendorTargetDir = path.join(vendorRoot, target, "codex-potter");
  const binaryName = target.includes("windows")
    ? "codex-potter.exe"
    : "codex-potter";
  const binarySource = path.join(artifactDir, binaryName);

  if (!fs.existsSync(binarySource)) {
    throw new Error(`Missing ${binaryName} in ${artifactDir}`);
  }

  fs.mkdirSync(vendorTargetDir, { recursive: true });
  const binaryDestination = path.join(vendorTargetDir, binaryName);
  fs.copyFileSync(binarySource, binaryDestination);
  if (!target.includes("windows")) {
    fs.chmodSync(binaryDestination, 0o755);
  }
}

function rewritePackageVersion(packageJsonPath, version) {
  const packageJson = JSON.parse(fs.readFileSync(packageJsonPath, "utf8"));
  packageJson.version = version;
  fs.writeFileSync(packageJsonPath, JSON.stringify(packageJson, null, 2) + "\n");
}

/**
 * Stage the npm package from the checked-in npm sources, then inject the built
 * native binaries. Copying the npm directory as a whole keeps the staged
 * package aligned with the launcher's runtime imports and npm's files allowlist.
 */
export function stageReleasePackage({ npmRoot, stageRoot, distRoot, version }) {
  fs.rmSync(stageRoot, { recursive: true, force: true });
  fs.cpSync(npmRoot, stageRoot, { recursive: true });

  rewritePackageVersion(path.join(stageRoot, "package.json"), version);

  const vendorRoot = path.join(stageRoot, "vendor");
  fs.rmSync(vendorRoot, { recursive: true, force: true });
  fs.mkdirSync(vendorRoot, { recursive: true });

  const artifactDirs = fs
    .readdirSync(distRoot, { withFileTypes: true })
    .filter((entry) => entry.isDirectory() && entry.name.startsWith("codex-potter-"))
    .map((entry) => path.join(distRoot, entry.name))
    .sort();

  if (artifactDirs.length === 0) {
    throw new Error(`No codex-potter artifacts found in ${distRoot}`);
  }

  for (const artifactDir of artifactDirs) {
    copyArtifactBinary(artifactDir, vendorRoot);
  }
}

function main(argv) {
  if (argv.length !== 3) {
    throw new Error(
      "Usage: node npm/scripts/stage-release-package.js <stage-root> <dist-root> <version>",
    );
  }

  const [stageRoot, distRoot, version] = argv;
  stageReleasePackage({
    npmRoot: path.resolve(__dirname, ".."),
    stageRoot: path.resolve(stageRoot),
    distRoot: path.resolve(distRoot),
    version,
  });
}

if (process.argv[1] && path.resolve(process.argv[1]) === __filename) {
  main(process.argv.slice(2));
}
