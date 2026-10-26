// Loads the region bundle and quest-core (Rust, compiled to wasm) off the main thread, then answers
// {fn: "geocode" | "route"} requests. Everything stays in this tab: the only fetches are this site's own
// files, made the same way whatever the person types.

// The region's data lives in the app's R2 bucket (npm run publish:data), served by the Worker under /view/, so
// a deploy from git (which holds no data) still finds it. Locally, pipeline/web.sh stages a copy at the same path.
// Which file, and its sha256, come from the region's record committed on chain (pipeline/region_record.mjs),
// which the Worker indexes into the app's view; until there is one, from this site's own manifest.
const DATA = "view/regions/";
const MANIFEST = `${DATA}us-ms.json`;
const REGION = "quest:region:us-ms";
const enc = new TextEncoder(), dec = new TextDecoder();
let wasm;

// Replies from quest-core are a pointer to [u32 length][UTF-8 JSON]; the caller frees them.
function reply(ptr) {
  const len = new DataView(wasm.memory.buffer).getUint32(ptr, true);
  const text = dec.decode(new Uint8Array(wasm.memory.buffer, ptr + 4, len));
  wasm.quest_free(ptr, len + 4);
  return JSON.parse(text);
}

function copyIn(bytes) {
  const ptr = wasm.quest_alloc(bytes.length);
  new Uint8Array(wasm.memory.buffer, ptr, bytes.length).set(bytes); // view made after alloc: memory may grow
  return ptr;
}

function call(req) {
  const bytes = enc.encode(JSON.stringify(req));
  const ptr = copyIn(bytes);
  const out = reply(wasm.quest_call(ptr, bytes.length));
  wasm.quest_free(ptr, bytes.length);
  return out;
}

const sha256 = async (bytes) => [...new Uint8Array(await crypto.subtle.digest("SHA-256", bytes))]
  .map((b) => b.toString(16).padStart(2, "0")).join("");
const gunzip = (bytes) => new Response(new Blob([bytes]).stream().pipeThrough(new DecompressionStream("gzip")));

// The namespace's on-chain tip, which the view keeps as its sha256 digest, as the commit's CID (CIDv1,
// dag-cbor, base32): what `fangorn status` prints. Null if there is none.
function commitCid(head) {
  if (!/^0x[0-9a-f]{64}$/i.test(head ?? "")) return null;
  const bytes = [0x01, 0x71, 0x12, 0x20, ...head.slice(2).match(/../g).map((h) => parseInt(h, 16))];
  let out = "b", bits = 0, v = 0;
  for (const x of bytes) {
    v = (v << 8) | x; bits += 8;
    while (bits >= 5) out += "abcdefghijklmnopqrstuvwxyz234567"[(v >> (bits -= 5)) & 31];
  }
  return bits ? out + "abcdefghijklmnopqrstuvwxyz234567"[(v << (5 - bits)) & 31] : out;
}

// The region's record in the app's view (westmarch's layout: catalog -> domain manifest -> gzipped ndjson
// shards of {track_id, fields}), each shard checked against its manifest's sha256. Null if there is none.
async function regionRecord(id) {
  const get = async (path) => { const r = await fetch(`view/${path}`); return r.ok ? r : null; };
  const catalog = await (await get("cdn/catalog"))?.json();
  for (const d of catalog?.domains ?? []) {
    if (!d.entity_types?.includes("quest.region.v1")) continue;
    const manifest = await (await get(`cdn/domains/${d.name}/manifest`))?.json();
    const gone = new Set(manifest?.tombstones ?? []);
    for (const shard of manifest?.shards ?? []) {
      const gz = new Uint8Array(await (await get(`cdn/domains/${d.name}/shards/${shard.file}`))?.arrayBuffer() ?? new ArrayBuffer(0));
      if (await sha256(gz) !== shard.sha256) continue;
      for (const line of (await gunzip(gz).text()).split("\n")) {
        const row = line.trim() && JSON.parse(line);
        if (row && !gone.has(row.track_id) && row.fields?.id === id) return { ...row.fields, commit: commitCid(manifest.head) };
      }
    }
  }
  return null;
}

async function download(url, total) {
  const res = await fetch(url);
  if (!res.ok) throw new Error(`Could not download the map data (HTTP ${res.status}).`);
  const reader = res.body.getReader(), chunks = [];
  let loaded = 0;
  for (;;) {
    const { done, value } = await reader.read();
    if (done) break;
    chunks.push(value);
    loaded += value.length;
    postMessage({ type: "progress", loaded, total });
  }
  const out = new Uint8Array(loaded);
  let at = 0;
  for (const c of chunks) { out.set(c, at); at += c.length; }
  return out;
}

async function load() {
  const record = await regionRecord(REGION).catch(() => null);
  let manifest = record && { file: record.file, sha256: record.sha256, bytes: record.bytes };
  if (!manifest) {
    const res = await fetch(MANIFEST);
    if (!res.ok) throw new Error(`Could not load the map manifest (HTTP ${res.status}).`);
    manifest = await res.json();
  }
  const [gz, code] = await Promise.all([
    download(`${DATA}${manifest.file}`, manifest.bytes),
    fetch("quest.wasm").then((r) => r.arrayBuffer()),
  ]);
  const digest = await sha256(gz);
  if (digest !== manifest.sha256) throw new Error("The map data failed its integrity check, so it was not used.");
  const raw = new Uint8Array(await gunzip(gz).arrayBuffer());
  wasm = (await WebAssembly.instantiate(code, {})).instance.exports;
  const out = reply(wasm.quest_load(copyIn(raw), raw.length)); // quest_load takes ownership of the buffer
  if (out.error) throw new Error(out.error);
  postMessage({ type: "ready", meta: out.meta, sha256: digest, checked: record ? "record" : "site", commit: record?.commit });
}

const ready = load().catch((e) => { wasm = null; postMessage({ type: "error", message: e.message }); });

onmessage = async ({ data }) => {
  await ready;
  postMessage({ id: data.id, result: wasm ? call(data) : { error: "The map data is not loaded." } });
};
