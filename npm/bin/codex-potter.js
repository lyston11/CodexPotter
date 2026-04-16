#!/usr/bin/env node
// Unified entry point for the CodexPotter CLI.

import os from "node:os";
import { spawn } from "node:child_process";
import { existsSync } from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const __filename = fileURLToPath(import.meta.url);
const __dirname = path.dirname(__filename);

function getTargetTriple(platform, arch) {
  switch (platform) {
    case "linux":
    case "android":
      switch (arch) {
        case "x64":
          return "x86_64-unknown-linux-musl";
        case "arm64":
          return "aarch64-unknown-linux-musl";
        default:
          return null;
      }
    case "darwin":
      switch (arch) {
        case "x64":
          return "x86_64-apple-darwin";
        case "arm64":
          return "aarch64-apple-darwin";
        default:
          return null;
      }
    case "win32":
      switch (arch) {
        case "x64":
          return "x86_64-pc-windows-msvc";
        case "arm64":
          return "aarch64-pc-windows-msvc";
        default:
          return null;
      }
    default:
      return null;
  }
}

function getUpdatedPath(newDirs) {
  const pathSep = process.platform === "win32" ? ";" : ":";
  const existingPath = process.env.PATH || "";
  const updatedPath = [
    ...newDirs,
    ...existingPath.split(pathSep).filter(Boolean),
  ].join(pathSep);
  return updatedPath;
}

/**
 * Use heuristics to detect the package manager that was used to install CodexPotter
 * in order to give the user a hint about how to update it.
 */
function detectPackageManager() {
  const userAgent = process.env.npm_config_user_agent || "";
  if (/\bbun\//.test(userAgent)) {
    return "bun";
  }

  const execPath = process.env.npm_execpath || "";
  if (execPath.includes("bun")) {
    return "bun";
  }

  if (
    __dirname.includes(".bun/install/global") ||
    __dirname.includes(".bun\\install\\global")
  ) {
    return "bun";
  }

  return userAgent ? "npm" : null;
}

export function exitCodeFromSignal(signal) {
  const signalNumber = os.constants.signals[signal];
  return typeof signalNumber === "number" ? 128 + signalNumber : 1;
}

export function reemitSignalOrExit(processLike, signal) {
  try {
    processLike.kill(processLike.pid, signal);
  } catch {
    processLike.exit(exitCodeFromSignal(signal));
  }
}

async function main(argv) {
  const targetTriple = getTargetTriple(process.platform, process.arch);
  if (!targetTriple) {
    throw new Error(`Unsupported platform: ${process.platform} (${process.arch})`);
  }

  const vendorRoot = path.join(__dirname, "..", "vendor");
  const archRoot = path.join(vendorRoot, targetTriple);
  const binaryName =
    process.platform === "win32" ? "codex-potter.exe" : "codex-potter";
  const binaryPath = path.join(archRoot, "codex-potter", binaryName);
  const additionalDirs = [];
  const pathDir = path.join(archRoot, "path");
  if (existsSync(pathDir)) {
    additionalDirs.push(pathDir);
  }
  const updatedPath = getUpdatedPath(additionalDirs);
  const env = { ...process.env, PATH: updatedPath };
  const packageManagerEnvVar =
    detectPackageManager() === "bun"
      ? "CODEX_POTTER_MANAGED_BY_BUN"
      : "CODEX_POTTER_MANAGED_BY_NPM";
  env[packageManagerEnvVar] = "1";

  // Use an asynchronous spawn instead of spawnSync so that Node is able to
  // respond to signals (e.g. Ctrl-C / SIGINT) while the native binary is
  // executing. This allows us to forward those signals to the child process
  // and guarantees that when either the child terminates or the parent
  // receives a fatal signal, both processes exit in a predictable manner.
  const child = spawn(binaryPath, argv, {
    stdio: "inherit",
    env,
  });

  child.on("error", (err) => {
    // eslint-disable-next-line no-console
    console.error(err);
    process.exit(1);
  });

  const forwardSignal = (signal) => {
    if (child.killed) {
      return;
    }
    try {
      child.kill(signal);
    } catch {
      /* ignore */
    }
  };

  ["SIGINT", "SIGTERM", "SIGHUP"].forEach((sig) => {
    process.on(sig, () => forwardSignal(sig));
  });

  const childResult = await new Promise((resolve) => {
    child.on("exit", (code, signal) => {
      if (signal) {
        resolve({ type: "signal", signal });
      } else {
        resolve({ type: "code", exitCode: code ?? 1 });
      }
    });
  });

  if (childResult.type === "signal") {
    reemitSignalOrExit(process, childResult.signal);
    return;
  }

  process.exit(childResult.exitCode);
}

if (process.argv[1] && path.resolve(process.argv[1]) === __filename) {
  await main(process.argv.slice(2));
}
