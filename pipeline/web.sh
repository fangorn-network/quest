#!/usr/bin/env bash
# Build quest-core to wasm (web/quest.wasm, committed, so a deploy from git has it) and stage the bundle at
# web/view/regions/ for local viewing (ignored by git and by deploys: live, the Worker serves it from R2, put
# there by npm run publish:data). The bundle's name holds its sha256, so a cached copy never disagrees with
# its manifest.
set -euo pipefail
cd "$(dirname "$0")/.."
(cd core && cargo build --release --target wasm32-unknown-unknown --lib)
wasm-opt -O3 --enable-bulk-memory --enable-nontrapping-float-to-int --enable-sign-ext \
  --enable-mutable-globals --enable-multivalue --enable-reference-types \
  core/target/wasm32-unknown-unknown/release/quest_core.wasm -o web/quest.wasm
rm -rf web/bundle web/view && mkdir -p web/view/regions
m=.ship/bundle/us-ms.json
file="us-ms-$(jq -r '.sha256[:12]' $m).bin.gz"
cp .ship/bundle/us-ms.bin.gz "web/view/regions/$file"
jq --arg f "$file" '.file = $f' $m > web/view/regions/us-ms.json
ls -la web/quest.wasm web/view/regions
