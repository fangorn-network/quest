#!/usr/bin/env bash
# Build quest-core to wasm and stage the bundle for the page, into web/ (both ignored by git).
# The bundle is copied under a name holding its sha256, so a cached copy can never disagree with its manifest.
set -euo pipefail
cd "$(dirname "$0")/.."
(cd core && cargo build --release --target wasm32-unknown-unknown --lib)
wasm-opt -O3 --enable-bulk-memory --enable-nontrapping-float-to-int --enable-sign-ext \
  --enable-mutable-globals --enable-multivalue --enable-reference-types \
  core/target/wasm32-unknown-unknown/release/quest_core.wasm -o web/quest.wasm
rm -rf web/bundle && mkdir -p web/bundle
m=.ship/bundle/us-ms.json
file="us-ms-$(jq -r '.sha256[:12]' $m).bin.gz"
cp .ship/bundle/us-ms.bin.gz "web/bundle/$file"
jq --arg f "$file" '.file = $f' $m > web/bundle/us-ms.json
ls -la web/quest.wasm web/bundle
