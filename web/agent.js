// quest's tools for agents in a browser. Each is one action shared with the page's own form, so an agent
// and a person get the same answer, and directions an agent asks for show on the page's map and list.
// Everything runs in this tab: addresses go to quest-core (wasm) in worker.js, never over the network.
// `ctx` (set by app.js): ask(request) -> quest-core's reply; show(route, from, to) paints the page.

const ok = (o) => ({ content: [{ type: "text", text: JSON.stringify(o) }] });
const miles = (m) => Math.round(m / 160.9344) / 10;
const LOCATION = {
  type: "object",
  description: "A match returned by find-quest-address, passed back unchanged.",
  properties: { label: { type: "string" }, edge: { type: "integer" }, t: { type: "number" },
                lat: { type: "number" }, lon: { type: "number" }, quality: { type: "string" } },
  required: ["edge", "t"],
};
const PLACE = (which) => ({
  description: `Where the trip ${which}: address text ("400 High St, Jackson MS 39201"), or a match from find-quest-address.`,
  anyOf: [{ type: "string", minLength: 3 }, LOCATION],
});

export const actions = {
  /** Mississippi addresses matching free text, best first. */
  async findAddress(ctx, { address, limit = 5 }) {
    const n = Math.min(20, Math.max(1, Math.trunc(Number(limit) || 5)));
    const { matches, error } = await ctx.ask({ fn: "geocode", q: String(address ?? ""), limit: n });
    return error ? { error } : matches;
  },

  /** Driving directions between two places; each is address text or a match from findAddress. */
  async getDirections(ctx, { from, to }) {
    const pick = async (place, which) => {
      if (place && typeof place === "object") {
        return Number.isInteger(place.edge) && typeof place.t === "number"
          ? place : { error: `${which} is not a match from find-quest-address` };
      }
      const found = await actions.findAddress(ctx, { address: place, limit: 5 });
      if (found.error) return found;
      if (!found.length) return { error: `no Mississippi address matches "${place}" (${which})` };
      if (found.length > 1) return { error: `"${place}" matches ${found.length} places: pass one as ${which}`, choices: found };
      return found[0];
    };
    const a = await pick(from, "from");
    if (a.error) return a;
    const b = await pick(to, "to");
    if (b.error) return b;
    const { route, error } = await ctx.ask({ fn: "route", from: a, to: b });
    if (error) return { error };
    ctx.show?.(route, a, b);
    return {
      from: route.from, to: route.to, from_quality: a.quality, to_quality: b.quality,
      miles: miles(route.distance_m), minutes: Math.round(route.duration_s / 60),
      steps: route.steps.map((s) => ({ text: s.text, miles: miles(s.distance_m) })),
    };
  },
};

export function registerAgent(ctx) {
  const mc = document.modelContext;
  if (!mc?.registerTool) return; // most browsers have no WebMCP yet
  mc.registerTool({
    name: "find-quest-address",
    description: "Find Mississippi street addresses matching text, best first. Each match has a label and a quality: "
      + "exact (a mapped address), interpolated (placed along its block) or approximate (number not found; nearest block). "
      + "Pass a match as from/to in get-quest-directions when text is ambiguous. Runs on this device; nothing is sent anywhere.",
    inputSchema: {
      type: "object",
      properties: {
        address: { type: "string", minLength: 3, description: "Street address, with a town or ZIP code if known." },
        limit: { type: "integer", minimum: 1, maximum: 20, default: 5 },
      },
      required: ["address"],
    },
    execute: async (args) => ok(await actions.findAddress(ctx, args)),
  });
  mc.registerTool({
    name: "get-quest-directions",
    description: "Turn-by-turn driving directions between two places in Mississippi: total miles and minutes, and each step "
      + "with its distance. The route also appears on the page's map and printable list. If address text matches several "
      + "places, returns {error, choices}: pick one (from find-quest-address's shape) and call again. Runs on this device.",
    inputSchema: { type: "object", properties: { from: PLACE("starts"), to: PLACE("ends") }, required: ["from", "to"] },
    execute: async (args) => ok(await actions.getDirections(ctx, args)),
  });
}
