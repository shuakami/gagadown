#!/usr/bin/env bash
# 用法：tools/set-version.sh 0.1.3 —— 统一改各 crate 的版本号
set -euo pipefail
cd "$(dirname "$0")/.."
V="$1"
for f in crates/*/Cargo.toml; do
  sed -i -E "0,/^version = \"[^\"]*\"/s//version = \"$V\"/" "$f"
  sed -i -E "s/(gagadown-core = \{ version = \")[^\"]*/\1$V/" "$f"
done
grep -H '^version' crates/*/Cargo.toml
