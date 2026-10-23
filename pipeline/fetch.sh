#!/usr/bin/env bash
# Download the raw Mississippi data into .ship/raw (ignored by git). Rerunning skips what is there.
set -euo pipefail
cd "$(dirname "$0")/.."
RAW=.ship/raw
TIGER_YEAR=2026
STATE_FIPS=28
mkdir -p "$RAW/tiger"

# OSM roads and address points. -L follows Geofabrik's redirect to the dated file.
[ -s "$RAW/mississippi.osm.pbf" ] || curl -fL -o "$RAW/mississippi.osm.pbf" \
  https://download.geofabrik.de/north-america/us/mississippi-latest.osm.pbf

# TIGER house-number ranges, one zip per county.
base=https://www2.census.gov/geo/tiger/TIGER$TIGER_YEAR/ADDRFEAT
for f in $(curl -fs --http1.1 --retry 3 "$base/" | grep -o "tl_${TIGER_YEAR}_${STATE_FIPS}[0-9]*_addrfeat.zip" | sort -u); do
  [ -s "$RAW/tiger/$f" ] || { curl -fs --http1.1 --retry 3 -o "$RAW/tiger/$f.part" "$base/$f" && unzip -tq "$RAW/tiger/$f.part" >/dev/null && mv "$RAW/tiger/$f.part" "$RAW/tiger/$f" || { rm -f "$RAW/tiger/$f.part"; echo "failed: $f (the Census site may be rate limiting; rerun later)" >&2; }; }
done

# ZIP code -> Census place names, so "Jackson, MS" narrows to Jackson's ZIPs.
[ -s "$RAW/zcta_place.txt" ] || curl -fs --http1.1 --retry 3 -o "$RAW/zcta_place.txt" \
  https://www2.census.gov/geo/docs/maps-data/data/rel2020/zcta520/tab20_zcta520_place20_natl.txt

ls "$RAW/tiger" | wc -l | xargs echo "tiger counties:"
du -sh "$RAW"
