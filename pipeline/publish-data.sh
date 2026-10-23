#!/usr/bin/env bash
# Upload the built bundle to the app's R2 bucket, where the Worker serves it as /view/regions/... The bundle
# goes first under its sha256 name (immutable, so it never clobbers what visitors are loading), then the
# manifest that points at it, so the manifest never names a file that isn't there.
set -euo pipefail
cd "$(dirname "$0")/.."
BUCKET=quest-view
m=.ship/bundle/us-ms.json
file="us-ms-$(jq -r '.sha256[:12]' $m).bin.gz"
jq --arg f "$file" '.file = $f' $m > .ship/bundle/us-ms.published.json
npx wrangler r2 object put "$BUCKET/view/regions/$file" --file .ship/bundle/us-ms.bin.gz \
  --content-type application/gzip --remote
npx wrangler r2 object put "$BUCKET/view/regions/us-ms.json" --file .ship/bundle/us-ms.published.json \
  --content-type application/json --remote
echo "published $file ($(jq -r .bytes $m) bytes, sha256 $(jq -r .sha256 $m))"
