: <<'::CMDLITERAL'
@goto :batch
::CMDLITERAL
# This file intentionally has no shebang. Bun links package bins directly, so
# the top-level npm bin must be runnable without `node` while still remaining a
# valid Windows batch launcher for bare Windows installs.

script=$0
case $script in
  */*) ;;
  *) script=$(command -v "$script") || exit 1 ;;
esac
launcher_path=$script

# Resolve package-manager symlinks before locating the bundled native binary.
while [ -L "$script" ]; do
  link=$(readlink "$script") || {
    printf 'Failed to resolve launcher symlink: %s\n' "$script" >&2
    exit 1
  }
  case $link in
    /*) script=$link ;;
    *) script=${script%/*}/$link ;;
  esac
done

basedir=${script%/*}
[ "$basedir" = "$script" ] && basedir=.

platform=$(uname -s 2>/dev/null || printf 'unknown')
arch=$(uname -m 2>/dev/null || printf 'unknown')

case "$platform:$arch" in
  Linux:x86_64|Linux:amd64) target_triple=x86_64-unknown-linux-musl ;;
  Linux:aarch64|Linux:arm64) target_triple=aarch64-unknown-linux-musl ;;
  Darwin:x86_64) target_triple=x86_64-apple-darwin ;;
  Darwin:aarch64|Darwin:arm64) target_triple=aarch64-apple-darwin ;;
  *)
    printf 'Unsupported platform: %s (%s)\n' "$platform" "$arch" >&2
    exit 1
    ;;
esac

binary_path=$basedir/../vendor/$target_triple/codex-potter/codex-potter
path_dir=$basedir/../vendor/$target_triple/path

if [ -d "$path_dir" ]; then
  PATH=$path_dir${PATH+:$PATH}
  export PATH
fi

managed_by_bun=0
case ${npm_config_user_agent-} in
  *bun/*) managed_by_bun=1 ;;
esac
case ${npm_execpath-} in
  *bun*) managed_by_bun=1 ;;
esac
case $launcher_path in
  *".bun/bin/"*|*".bun\\bin\\"*|*".bun/install/global/"*|*".bun\\install\\global\\"*)
    managed_by_bun=1
    ;;
esac
case $basedir in
  *".bun/install/global"*|*".bun\\install\\global"*) managed_by_bun=1 ;;
esac

unset CODEX_POTTER_MANAGED_BY_NPM CODEX_POTTER_MANAGED_BY_BUN
if [ "$managed_by_bun" -eq 1 ]; then
  export CODEX_POTTER_MANAGED_BY_BUN=1
else
  export CODEX_POTTER_MANAGED_BY_NPM=1
fi

if [ ! -x "$binary_path" ]; then
  printf 'Missing packaged binary: %s\n' "$binary_path" >&2
  exit 1
fi

exec "$binary_path" "$@"
exit 1

:batch
@echo off
setlocal
rem Keep the Windows batch path in the same file so npm/cmd-shim can invoke the
rem launcher directly without requiring a separate interpreter.

set "arch=%PROCESSOR_ARCHITECTURE%"
if defined PROCESSOR_ARCHITEW6432 set "arch=%PROCESSOR_ARCHITEW6432%"

set "target_triple="
if /I "%arch%"=="AMD64" set "target_triple=x86_64-pc-windows-msvc"
if /I "%arch%"=="ARM64" set "target_triple=aarch64-pc-windows-msvc"

if not defined target_triple (
  >&2 echo Unsupported platform: Windows %arch%
  exit /b 1
)

set "package_root=%~dp0.."
set "binary_path=%package_root%\vendor\%target_triple%\codex-potter\codex-potter.exe"
set "path_dir=%package_root%\vendor\%target_triple%\path"
set "launcher_dir=%~dp0"

if not exist "%binary_path%" (
  set "package_root=%~dp0..\codex-potter"
  set "binary_path=%package_root%\vendor\%target_triple%\codex-potter\codex-potter.exe"
  set "path_dir=%package_root%\vendor\%target_triple%\path"
)

if not exist "%binary_path%" (
  set "package_root=%~dp0..\install\global\node_modules\codex-potter"
  set "binary_path=%package_root%\vendor\%target_triple%\codex-potter\codex-potter.exe"
  set "path_dir=%package_root%\vendor\%target_triple%\path"
)

if exist "%path_dir%\" set "PATH=%path_dir%;%PATH%"

set "managed_by_bun="
if defined npm_config_user_agent if not "%npm_config_user_agent%"=="%npm_config_user_agent:bun/=%" set "managed_by_bun=1"
if defined npm_execpath if not "%npm_execpath%"=="%npm_execpath:bun=%" set "managed_by_bun=1"
if not "%launcher_dir%"=="%launcher_dir:.bun\bin\=%" set "managed_by_bun=1"
if not "%launcher_dir%"=="%launcher_dir:.bun\install\global\=%" set "managed_by_bun=1"

set "CODEX_POTTER_MANAGED_BY_NPM="
set "CODEX_POTTER_MANAGED_BY_BUN="
if defined managed_by_bun (
  set "CODEX_POTTER_MANAGED_BY_BUN=1"
) else (
  set "CODEX_POTTER_MANAGED_BY_NPM=1"
)

if not exist "%binary_path%" (
  >&2 echo Missing packaged binary: %binary_path%
  exit /b 1
)

"%binary_path%" %*
exit /b %ERRORLEVEL%
