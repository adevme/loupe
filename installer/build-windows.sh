#!/usr/bin/env bash
set -euo pipefail

here="$(cd "$(dirname "$0")" && pwd)"
root="$(cd "$here/.." && pwd)"
pc="${LOUPE_BUILD_PC:-flstudio}"
version="$(grep -m1 '^version' "$root/Cargo.toml" | cut -d'"' -f2)"
target=x86_64-pc-windows-gnu
built="$root/target/$target/release"
remote='C:/Users/ash/AppData/Local/Temp/loupe-installer'
iscc='%LOCALAPPDATA%\Programs\Inno Setup 6\ISCC.exe'

cargo build --release --target "$target" -p loupe -p loupe-host -p loupe-launcher --manifest-path "$root/Cargo.toml"

stage="$here/build"
rm -rf "$stage"
mkdir -p "$stage"
cp "$built/loupe.exe" "$stage/loupe.exe"
cp "$built/loupe-host.exe" "$stage/loupe-host.exe"
cp "$built/loupe-launcher.exe" "$stage/Loupe.exe"

ssh "$pc" "if exist \"%TEMP%\\loupe-installer\" rmdir /s /q \"%TEMP%\\loupe-installer\"" >/dev/null 2>&1 || true
ssh "$pc" "mkdir \"%TEMP%\\loupe-installer\\installer\\build\" & mkdir \"%TEMP%\\loupe-installer\\crates\\app\\assets\"" >/dev/null 2>&1
scp -q "$here/loupe.iss" "$pc:$remote/installer/loupe.iss"
scp -q "$stage/"*.exe "$pc:$remote/installer/build/"
scp -q "$root/crates/app/assets/icon.ico" "$pc:$remote/crates/app/assets/icon.ico"
ssh "$pc" "cd /d \"%TEMP%\\loupe-installer\\installer\" && \"$iscc\" /Q /DAppVersion=$version loupe.iss"

mkdir -p "$here/output"
scp -q "$pc:$remote/installer/output/loupe-setup-$version.exe" "$here/output/"
ssh "$pc" "rmdir /s /q \"%TEMP%\\loupe-installer\"" >/dev/null 2>&1 || true
echo "$here/output/loupe-setup-$version.exe"
