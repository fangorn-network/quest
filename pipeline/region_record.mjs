// The region's record for the chain: what `fangorn commit` takes, from the build's manifest.
//
//   node pipeline/region_record.mjs                      -> .ship/fangorn/regions.json (the commit file)
//   node pipeline/region_record.mjs --view web/view      -> also a local view of it, built by westmarch's own
//                                                           publishView with no chain, to test the page's reader
//
// The record carries the bundle's sha256, so the page (and anyone) can check the map data against what the
// app's owner committed. Its id is fixed: the next build's record replaces this one under `--replace`.
import { createHash } from "node:crypto";
import { mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const root = join(dirname(fileURLToPath(import.meta.url)), "..");
const SITE = "https://quest.fangorn.network";
const m = JSON.parse(readFileSync(join(root, ".ship/bundle/us-ms.json"), "utf8"));
const app = JSON.parse(readFileSync(join(root, "app.json"), "utf8"));
const file = `us-ms-${m.sha256.slice(0, 12)}.bin.gz`; // the name publish-data.sh uploads it under

const payload = {
  id: `quest:region:${m.region}`,
  region: m.region,
  title: "Mississippi",
  text: `Mississippi's map data for private, on-device directions: ${m.counts.edges.toLocaleString("en-US")} road `
    + `segments, ${(m.counts.ranges + m.counts.address_points).toLocaleString("en-US")} address ranges and points, and `
    + `${m.counts.businesses.toLocaleString("en-US")} businesses and places. From OpenStreetMap (${m.osm_date.slice(0, 10)}), `
    + `US Census TIGER/Line ${m.tiger_year} and Overture Maps ${m.overture_release}.`,
  tags: ["mississippi", "directions", "map data"],
  file,
  url: `${SITE}/view/regions/${file}`,
  sha256: m.sha256,
  bytes: m.bytes,
  format: 1,
  osm_date: m.osm_date,
  tiger_year: m.tiger_year,
  overture_release: m.overture_release,
  attribution: "© OpenStreetMap contributors (ODbL); US Census Bureau TIGER/Line; places © Overture Maps Foundation "
    + "(CDLA-Permissive-2.0; some sources Apache-2.0 or CC0-1.0)",
};
const commit = { records: [{ id: payload.id, type: "quest.region.v1", payload }] };
const out = join(root, ".ship/fangorn/regions.json");
mkdirSync(dirname(out), { recursive: true });
writeFileSync(out, JSON.stringify(commit, null, 1) + "\n");
console.log(`${out}: ${payload.id}, sha256 ${payload.sha256.slice(0, 16)}…, ${payload.bytes} bytes`);

const at = process.argv.indexOf("--view");
if (at > 0) {
  const dir = join(process.cwd(), process.argv[at + 1]);
  const view = await import("@fangorn-network/westmarch/view");
  const { env } = await import("@huggingface/transformers");
  env.cacheDir = `${process.env.HOME}/.cache/fangorn-mcp/models`;
  // Not the app's real id or owner: nothing here is compared with the chain (as westmarch-ship --local does).
  const owner = `0x${"10ca1".padStart(40, "0")}`, appId = `0x${createHash("sha256").update(app.app).digest("hex")}`;
  const cid = createHash("sha256").update(JSON.stringify(payload)).digest("hex");
  const chain = { [`${owner}/quest`]: { records: [{ ...commit.records[0], cid }] } };
  await view.publishView({
    fangorn: view.localFangorn(chain, { appId, owner }), app: app.app, repos: ["quest"], out: dir,
    schema: { types: app.types, relations: app.relations ?? [] }, embed: view.cachedEmbed(join(root, ".ship/vectors.ndjson")),
  });
  console.log(`local view of the record: ${dir}/cdn/catalog`);
}
