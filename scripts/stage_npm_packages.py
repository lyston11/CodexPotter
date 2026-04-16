#!/usr/bin/env python3
"""Stage one or more codex-potter npm packages for release."""

from __future__ import annotations

import argparse
import importlib.util
import os
import shutil
import subprocess
import tempfile
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parent.parent
BUILD_SCRIPT = REPO_ROOT / "npm" / "scripts" / "build_npm_package.py"

_SPEC = importlib.util.spec_from_file_location("codex_potter_build_npm_package", BUILD_SCRIPT)
if _SPEC is None or _SPEC.loader is None:
    raise RuntimeError(f"Unable to load module from {BUILD_SCRIPT}")
_BUILD_MODULE = importlib.util.module_from_spec(_SPEC)
_SPEC.loader.exec_module(_BUILD_MODULE)

PACKAGE_NATIVE_COMPONENTS = getattr(_BUILD_MODULE, "PACKAGE_NATIVE_COMPONENTS", {})
PACKAGE_EXPANSIONS = getattr(_BUILD_MODULE, "PACKAGE_EXPANSIONS", {})
CODEX_POTTER_PLATFORM_PACKAGES = getattr(_BUILD_MODULE, "CODEX_POTTER_PLATFORM_PACKAGES", {})


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--release-version",
        required=True,
        help="Version to stage (e.g. 0.1.0 or 0.1.0-alpha.1).",
    )
    parser.add_argument(
        "--dist-root",
        type=Path,
        required=True,
        help="Directory containing built release artifacts (e.g. dist/).",
    )
    parser.add_argument(
        "--package",
        dest="packages",
        action="append",
        required=True,
        help="Package name to stage. May be provided multiple times.",
    )
    parser.add_argument(
        "--output-dir",
        type=Path,
        default=None,
        help="Directory where npm tarballs should be written (default: dist/npm).",
    )
    parser.add_argument(
        "--keep-staging-dirs",
        action="store_true",
        help="Retain temporary staging directories instead of deleting them.",
    )
    return parser.parse_args()


def collect_native_components(packages: list[str]) -> set[str]:
    components: set[str] = set()
    for package in packages:
        components.update(PACKAGE_NATIVE_COMPONENTS.get(package, []))
    return components


def expand_packages(packages: list[str]) -> list[str]:
    expanded: list[str] = []
    for package in packages:
        for expanded_package in PACKAGE_EXPANSIONS.get(package, [package]):
            if expanded_package in expanded:
                continue
            expanded.append(expanded_package)
    return expanded


def run_command(cmd: list[str]) -> None:
    print("+", " ".join(cmd))
    subprocess.run(cmd, cwd=REPO_ROOT, check=True)


def build_vendor_src(dist_root: Path, vendor_root: Path) -> None:
    if not CODEX_POTTER_PLATFORM_PACKAGES:
        raise RuntimeError("No platform packages registered in build_npm_package.py")

    for package_config in CODEX_POTTER_PLATFORM_PACKAGES.values():
        target_triple = package_config["target_triple"]
        artifact_dir = dist_root / f"codex-potter-{target_triple}"
        if "windows" in target_triple:
            binary_name = "codex-potter.exe"
        else:
            binary_name = "codex-potter"

        binary_src = artifact_dir / binary_name
        if not binary_src.exists():
            raise RuntimeError(f"Missing {binary_name} in {artifact_dir}")

        dest_dir = vendor_root / target_triple / "codex-potter"
        dest_dir.mkdir(parents=True, exist_ok=True)
        binary_dest = dest_dir / binary_name
        shutil.copy2(binary_src, binary_dest)

        if "windows" not in target_triple:
            binary_dest.chmod(0o755)


def tarball_name_for_package(package: str, version: str) -> str:
    if package in CODEX_POTTER_PLATFORM_PACKAGES:
        platform_tag = CODEX_POTTER_PLATFORM_PACKAGES[package]["npm_tag"]
        return f"codex-potter-npm-{platform_tag}-{version}.tgz"
    return f"{package}-npm-{version}.tgz"


def main() -> int:
    args = parse_args()

    output_dir = args.output_dir or (REPO_ROOT / "dist" / "npm")
    output_dir.mkdir(parents=True, exist_ok=True)

    dist_root = args.dist_root.resolve()
    if not dist_root.exists():
        raise RuntimeError(f"dist root does not exist: {dist_root}")

    runner_temp = Path(os.environ.get("RUNNER_TEMP", tempfile.gettempdir()))

    packages = expand_packages(list(args.packages))
    native_components = collect_native_components(packages)

    vendor_temp_root: Path | None = None
    vendor_src: Path | None = None

    final_messages: list[str] = []

    try:
        if native_components:
            vendor_temp_root = Path(tempfile.mkdtemp(prefix="npm-vendor-", dir=runner_temp))
            vendor_src = vendor_temp_root / "vendor"
            vendor_src.mkdir(parents=True, exist_ok=True)
            build_vendor_src(dist_root, vendor_src)

        for package in packages:
            staging_dir = Path(tempfile.mkdtemp(prefix=f"npm-stage-{package}-", dir=runner_temp))
            pack_output = output_dir / tarball_name_for_package(package, args.release_version)

            cmd = [
                str(BUILD_SCRIPT),
                "--package",
                package,
                "--release-version",
                args.release_version,
                "--staging-dir",
                str(staging_dir),
                "--pack-output",
                str(pack_output),
            ]

            if vendor_src is not None:
                cmd.extend(["--vendor-src", str(vendor_src)])

            try:
                run_command(cmd)
            finally:
                if not args.keep_staging_dirs:
                    shutil.rmtree(staging_dir, ignore_errors=True)

            final_messages.append(f"Staged {package} at {pack_output}")
    finally:
        if vendor_temp_root is not None and not args.keep_staging_dirs:
            shutil.rmtree(vendor_temp_root, ignore_errors=True)

    for msg in final_messages:
        print(msg)

    return 0


if __name__ == "__main__":
    raise SystemExit(main())

