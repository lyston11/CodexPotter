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

platform_tag=
case "$target_triple" in
  x86_64-unknown-linux-musl) platform_tag=linux-x64 ;;
  aarch64-unknown-linux-musl) platform_tag=linux-arm64 ;;
  x86_64-apple-darwin) platform_tag=darwin-x64 ;;
  aarch64-apple-darwin) platform_tag=darwin-arm64 ;;
  *) ;;
esac

local_vendor_root=$basedir/../vendor
local_binary_path=$local_vendor_root/$target_triple/codex-potter/codex-potter

optional_vendor_root=
optional_binary_path=
if [ -n "$platform_tag" ]; then
  package_root=$basedir/..
  node_modules_root=$package_root/..
  optional_package=$node_modules_root/codex-potter-$platform_tag
  optional_vendor_root=$optional_package/vendor
  optional_binary_path=$optional_vendor_root/$target_triple/codex-potter/codex-potter
fi

if [ -n "$optional_binary_path" ] && [ -x "$optional_binary_path" ]; then
  vendor_root=$optional_vendor_root
  binary_path=$optional_binary_path
elif [ -x "$local_binary_path" ]; then
  vendor_root=$local_vendor_root
  binary_path=$local_binary_path
else
  update_cmd='npm install -g codex-potter@latest'
  if [ -n "${npm_config_user_agent-}" ] && printf '%s' "${npm_config_user_agent-}" | grep -q 'bun/'; then
    update_cmd='bun install -g codex-potter@latest'
  elif [ -n "${npm_execpath-}" ] && printf '%s' "${npm_execpath-}" | grep -q 'bun'; then
    update_cmd='bun install -g codex-potter@latest'
  fi

  if [ -n "$platform_tag" ]; then
    printf 'Missing optional dependency codex-potter-%s. Reinstall: %s\n' "$platform_tag" "$update_cmd" >&2
  else
    printf 'Missing packaged binary for target %s. Reinstall: %s\n' "$target_triple" "$update_cmd" >&2
  fi
  exit 1
fi

path_dir=$vendor_root/$target_triple/path

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
set "platform_tag="
if /I "%arch%"=="AMD64" set "target_triple=x86_64-pc-windows-msvc" & set "platform_tag=win32-x64"
if /I "%arch%"=="ARM64" set "target_triple=aarch64-pc-windows-msvc" & set "platform_tag=win32-arm64"

if not defined target_triple (
  >&2 echo Unsupported platform: Windows %arch%
  exit /b 1
)

set "optional_package=codex-potter-%platform_tag%"
set "binary_path="
set "path_dir="
set "launcher_dir=%~dp0"

call :resolve_binary_paths "%~dp0.."
if not defined binary_path call :resolve_binary_paths "%~dp0..\codex-potter"
if not defined binary_path call :resolve_binary_paths "%~dp0..\install\global\node_modules\codex-potter"

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

if not defined binary_path (
  >&2 echo Missing optional dependency %optional_package%. Reinstall: npm install -g codex-potter@latest
  exit /b 1
)

"%binary_path%" %*
exit /b %ERRORLEVEL%

:resolve_binary_paths
set "package_root=%~1"
set "optional_vendor_root=%package_root%\..\%optional_package%\vendor"
set "optional_binary=%optional_vendor_root%\%target_triple%\codex-potter\codex-potter.exe"
if exist "%optional_binary%" (
  set "binary_path=%optional_binary%"
  set "path_dir=%optional_vendor_root%\%target_triple%\path"
  exit /b 0
)

set "local_vendor_root=%package_root%\vendor"
set "local_binary=%local_vendor_root%\%target_triple%\codex-potter\codex-potter.exe"
if exist "%local_binary%" (
  set "binary_path=%local_binary%"
  set "path_dir=%local_vendor_root%\%target_triple%\path"
)
exit /b 0
