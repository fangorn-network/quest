// Loads the region bundle and quest-core (Rust, compiled to wasm) off the main thread, then answers
// {fn: "geocode" | "route"} requests. Everything stays in this tab: the only fetches are this site's own
// files, made the same way whatever the person types.

// ponytail: the manifest comes from this site; Phase 4 reads the sha256 from the app's record on chain.
const MANIFEST = "bundle/us-ms.json";
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
  const res = await fetch(MANIFEST);
  if (!res.ok) throw new Error(`Could not load the map manifest (HTTP ${res.status}).`);
  const manifest = await res.json();
  const [gz, code] = await Promise.all([
    download(`bundle/${manifest.file}`, manifest.bytes),
    fetch("quest.wasm").then((r) => r.arrayBuffer()),
  ]);
  const digest = [...new Uint8Array(await crypto.subtle.digest("SHA-256", gz))]
    .map((b) => b.toString(16).padStart(2, "0")).join("");
  if (digest !== manifest.sha256) throw new Error("The map data failed its integrity check, so it was not used.");
  const raw = new Uint8Array(await new Response(new Blob([gz]).stream().pipeThrough(new DecompressionStream("gzip"))).arrayBuffer());
  wasm = (await WebAssembly.instantiate(code, {})).instance.exports;
  const out = reply(wasm.quest_load(copyIn(raw), raw.length)); // quest_load takes ownership of the buffer
  if (out.error) throw new Error(out.error);
  postMessage({ type: "ready", meta: out.meta, sha256: digest });
}

const ready = load().catch((e) => { wasm = null; postMessage({ type: "error", message: e.message }); });

onmessage = async ({ data }) => {
  await ready;
  postMessage({ id: data.id, result: wasm ? call(data) : { error: "The map data is not loaded." } });
};
