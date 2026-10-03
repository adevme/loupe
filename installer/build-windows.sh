#!/usr/bin/env bash
# Makes the Windows installer. Releases are built by GitHub Actions
# (.github/workflows/release.yml); this is for trying one out by hand.
#
# It needs Inno Setup's ISCC.exe, which is a Windows program. It will use,
# in order: a local ISCC (through wine if it is not a native one), or a
# Windows machine named by LOUPE_BUILD_PC.
set -euo pipefail

here="$(cd "$(dirname "$0")" && pwd)"
root="$(cd "$here/.." && pwd)"
version="$(grep -m1 '^version' "$root/Cargo.toml" | cut -d'"' -f2)"
target=x86_64-pc-windows-gnu
built="$root/target/$target/release"

cargo build --release --target "$target" -p loupe -p loupe-host -p loupe-launcher --manifest-path "$root/Cargo.toml"

stage="$here/build"
rm -rf "$stage"
mkdir -p "$stage" "$here/output"
cp "$built/loupe.exe" "$stage/loupe.exe"
cp "$built/loupe-host.exe" "$stage/loupe-host.exe"
cp "$built/loupe-launcher.exe" "$stage/Loupe.exe"

if command -v iscc >/dev/null 2>&1; then
  iscc /Q "/DAppVersion=$version" "$here/loupe.iss"
elif command -v wine >/dev/null 2>&1 && [ -n "${INNO_SETUP:-}" ]; then
  wine "$INNO_SETUP" /Q "/DAppVersion=$version" "$(winepath -w "$here/loupe.iss")"
elif [ -n "${LOUPE_BUILD_PC:-}" ]; then
  remote='C:/Users/ash/AppData/Local/Temp/loupe-installer'
  iscc='%LOCALAPPDATA%\Programs\Inno Setup 6\ISCC.exe'
  pc="$LOUPE_BUILD_PC"
  ssh "$pc" "if exist \"%TEMP%\\loupe-installer\" rmdir /s /q \"%TEMP%\\loupe-installer\"" >/dev/null 2>&1 || true
  ssh "$pc" "mkdir \"%TEMP%\\loupe-installer\\installer\\build\" & mkdir \"%TEMP%\\loupe-installer\\crates\\app\\assets\"" >/dev/null 2>&1
  scp -q "$here/loupe.iss" "$pc:$remote/installer/loupe.iss"
  scp -q "$stage/"*.exe "$pc:$remote/installer/build/"
  scp -q "$root/crates/app/assets/icon.ico" "$pc:$remote/crates/app/assets/icon.ico"
  ssh "$pc" "cd /d \"%TEMP%\\loupe-installer\\installer\" && \"$iscc\" /Q /DAppVersion=$version loupe.iss"
  scp -q "$pc:$remote/installer/output/loupe-setup-$version.exe" "$here/output/"
  ssh "$pc" "rmdir /s /q \"%TEMP%\\loupe-installer\"" >/dev/null 2>&1 || true
else
  echo "No way to run Inno Setup here." >&2
  echo "Install it and put ISCC on the path, or set INNO_SETUP to its ISCC.exe and have wine," >&2
  echo "or set LOUPE_BUILD_PC to a Windows machine you can ssh into." >&2
  echo "Releases do not need any of this: push a v tag and GitHub Actions builds it." >&2
  exit 1
fi

echo "$here/output/loupe-setup-$version.exe"
