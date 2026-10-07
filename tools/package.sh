#!/usr/bin/env bash
# Builds Windows release binaries and writes dist/ (NSIS setup, portable exe, cli, source zip).
set -euo pipefail
cd "$(dirname "$0")/.."
T=x86_64-pc-windows-gnu
V=$(grep -m1 '^version' crates/gagadown-app/Cargo.toml | cut -d'"' -f2)
cargo build --release --target $T -p gagadown-app -p gagadown-cli
rm -rf dist && mkdir -p dist
makensis -INPUTCHARSET UTF8 -V2 -DVERSION="$V" installer/gagadown.nsi
cp target/$T/release/GagaDown.exe "dist/GagaDown-$V-portable.exe"
cp target/$T/release/gagadown-cli.exe "dist/gagadown-cli-$V.exe"
zip -qr "dist/GagaDown-$V-src.zip" Cargo.toml Cargo.lock README.md .gitignore assets crates extension installer patches tools -x '*/target/*' '*__pycache__*'
ls -la dist
