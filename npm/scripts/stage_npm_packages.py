#!/usr/bin/env python3
"""Stage one or more codex-potter npm packages for release."""

from __future__ import annotations

import argparse
import os
import shutil
import subprocess
import tempfile
from pathlib import Path

import build_npm_package

REPO_ROOT = build_npm_package.REPO_ROOT
BUILD_SCRIPT = Path(build_npm_package.__file__).resolve()

PACKAGE_NATIVE_COMPONENTS = build_npm_package.PACKAGE_NATIVE_COMPONENTS
PACKAGE_EXPANSIONS = build_npm_package.PACKAGE_EXPANSIONS
CODEX_POTTER_PLATFORM_PACKAGES = build_npm_package.CODEX_POTTER_PLATFORM_PACKAGES


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


def build_vendor_src(dist_root: Path, vendor_root: Path, target_triples: set[str]) -> None:
    if not CODEX_POTTER_PLATFORM_PACKAGES:
        raise RuntimeError("No platform packages registered in build_npm_package.py")

    resolved_targets = {triple for triple in target_triples if triple}
    if not resolved_targets:
        resolved_targets = {
            package_config["target_triple"]
            for package_config in CODEX_POTTER_PLATFORM_PACKAGES.values()
        }

    for target_triple in sorted(resolved_targets):
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

    vendor_target_triples = {
        CODEX_POTTER_PLATFORM_PACKAGES[package]["target_triple"]
        for package in packages
        if package in CODEX_POTTER_PLATFORM_PACKAGES
    }

    vendor_temp_root: Path | None = None
    vendor_src: Path | None = None

    final_messages: list[str] = []

    try:
        if native_components:
            vendor_temp_root = Path(tempfile.mkdtemp(prefix="npm-vendor-", dir=runner_temp))
            vendor_src = vendor_temp_root / "vendor"
            vendor_src.mkdir(parents=True, exist_ok=True)
            build_vendor_src(dist_root, vendor_src, vendor_target_triples)

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
