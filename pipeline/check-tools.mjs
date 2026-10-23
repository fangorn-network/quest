// Every WebMCP tool, called the way an agent would: node pipeline/check-tools.mjs
// Loads web/quest.wasm and the staged bundle (npm run build:web first), captures registerAgent's tools with a
// stub document.modelContext, runs one sample call per tool and one that must miss. Exit 1 on any failure.
import { readFileSync } from "node:fs";
import { gunzipSync } from "node:zlib";
import { actions, registerAgent } from "../web/agent.js";

const web = new URL("../web/", import.meta.url);
const manifest = JSON.parse(readFileSync(new URL("bundle/us-ms.json", web)));
const { instance } = await WebAssembly.instantiate(readFileSync(new URL("quest.wasm", web)), {});
const w = instance.exports, enc = new TextEncoder(), dec = new TextDecoder();
const reply = (p) => {
  const n = new DataView(w.memory.buffer).getUint32(p, true);
  const s = dec.decode(new Uint8Array(w.memory.buffer, p + 4, n));
  w.quest_free(p, n + 4);
  return JSON.parse(s);
};
const copyIn = (bytes) => {
  const p = w.quest_alloc(bytes.length);
  new Uint8Array(w.memory.buffer, p, bytes.length).set(bytes);
  return p;
};
const raw = gunzipSync(readFileSync(new URL(`bundle/${manifest.file}`, web)));
const loaded = reply(w.quest_load(copyIn(raw), raw.length));
if (loaded.error) throw new Error(loaded.error);

const shown = [];
const ctx = {
  ask: async (req) => {
    const b = enc.encode(JSON.stringify(req)), p = copyIn(b);
    const out = reply(w.quest_call(p, b.length));
    w.quest_free(p, b.length);
    return out;
  },
  show: (route, from, to) => shown.push([from.label, to.label]),
};
const tools = {};
globalThis.document = { modelContext: { registerTool: (t) => { tools[t.name] = t; } } };
registerAgent(ctx);

const call = async (name, args) => JSON.parse((await tools[name].execute(args)).content[0].text);
const checks = {
  "find-quest-address": [
    [{ address: "400 High St, Jackson MS" }, (o) => Array.isArray(o) && o[0]?.label.startsWith("400 High St")],
    [{ address: "zzzz qqqq" }, (o) => Array.isArray(o) && o.length === 0],
  ],
  "get-quest-directions": [
    [{ from: "400 High St, Jackson MS 39201", to: "306 Elvis Presley Dr, Tupelo" },
     (o) => o.miles > 170 && o.miles < 220 && o.steps.at(-1).text === "Arrive at your destination"],
    [{ from: "100 Main St", to: "400 High St, Jackson MS" }, (o) => o.error && o.choices?.length > 1], // ambiguous
    [{ from: "zzzz qqqq", to: "400 High St, Jackson MS" }, (o) => o.error && !o.choices],               // no match
    [{ from: { edge: 1.5, t: 0 }, to: "400 High St, Jackson MS" }, (o) => !!o.error],                   // not a match
  ],
};
let bad = 0;
for (const name of Object.keys(tools)) {
  if (!checks[name]) { console.log(`✗ ${name}: no sample call`); bad++; continue; }
  for (const [args, pass] of checks[name]) {
    const out = await call(name, args);
    const good = pass(out);
    console.log(`${good ? "✓" : "✗"} ${name} ${JSON.stringify(args)} → ${JSON.stringify(out).slice(0, 110)}`);
    bad += !good;
  }
}
// A choice from find-quest-address goes straight back in as from/to.
const [jackson] = await call("find-quest-address", { address: "400 High St, Jackson MS" });
const viaMatch = await call("get-quest-directions", { from: jackson, to: "306 Elvis Presley Dr, Tupelo" });
console.log(`${viaMatch.miles ? "✓" : "✗"} a find-quest-address match as from → ${viaMatch.miles} mi`);
bad += !viaMatch.miles;
// The page shows what an agent asked for, and the form uses the same actions.
console.log(`${shown.length === 2 ? "✓" : "✗"} page painted ${shown.length} agent routes`);
bad += shown.length !== 2;
bad += !(typeof actions.findAddress === "function" && typeof actions.getDirections === "function");
process.exit(bad ? 1 : 0);
