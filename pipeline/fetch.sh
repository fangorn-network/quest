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

# TIGER house-number ranges, one zip per county. A zip is kept only if it tests whole: the Census site
# answers some requests with a 200 "Request Rejected" page.
get() { curl -fs --http1.1 --retry 3 -o "$2.part" "$1" && unzip -tq "$2.part" >/dev/null && mv "$2.part" "$2" || { rm -f "$2.part"; false; }; }
tiger=https://www2.census.gov/geo/tiger
for f in $(curl -fs --http1.1 --retry 3 "$tiger/TIGER$TIGER_YEAR/ADDRFEAT/" | grep -o "tl_${TIGER_YEAR}_${STATE_FIPS}[0-9]*_addrfeat.zip" | sort -u); do
  [ -s "$RAW/tiger/$f" ] || get "$tiger/TIGER$TIGER_YEAR/ADDRFEAT/$f" "$RAW/tiger/$f" && continue
  # Refused (its firewall has rejected one county's current file for days): the year before stands in until
  # a rerun gets the current one. The build uses the newest year on disk per county and lists stand-ins.
  prev="${f/_${TIGER_YEAR}_/_$((TIGER_YEAR - 1))_}"
  if [ -s "$RAW/tiger/$prev" ] || get "$tiger/TIGER$((TIGER_YEAR - 1))/ADDRFEAT/$prev" "$RAW/tiger/$prev"; then
    echo "refused: $f; using $prev until a rerun gets it" >&2
  else
    echo "failed: $f and $prev (rerun later)" >&2
  fi
done

# ZIP code -> Census place names, so "Jackson, MS" narrows to Jackson's ZIPs.
[ -s "$RAW/zcta_place.txt" ] || curl -fs --http1.1 --retry 3 -o "$RAW/zcta_place.txt" \
  https://www2.census.gov/geo/docs/maps-data/data/rel2020/zcta520/tab20_zcta520_place20_natl.txt

# Town centre points (Census Gazetteer), so "gas stations near Hattiesburg" has somewhere to measure from.
[ -s "$RAW/gaz_place.txt" ] || curl -fs --http1.1 --retry 3 -o "$RAW/gaz_place.txt" \
  "https://www2.census.gov/geo/docs/maps-data/data/gazetteer/2020_Gazetteer/2020_gaz_place_${STATE_FIPS}.txt"

# Businesses and other places from Overture Maps (CDLA-Permissive-2.0), pinned to one release.
OVERTURE_RELEASE=2026-10-21.0
[ -s "$RAW/overture_places_$OVERTURE_RELEASE.parquet" ] || .venv/bin/python pipeline/fetch_overture.py \
  "$OVERTURE_RELEASE" "$RAW/overture_places_$OVERTURE_RELEASE.parquet"

ls "$RAW/tiger" | wc -l | xargs echo "tiger counties:"
du -sh "$RAW"
