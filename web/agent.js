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

/** Straight-line metres between two [lat, lon] points. */
export function crowMetres([la1, lo1], [la2, lo2]) {
  const rad = Math.PI / 180, dLa = (la2 - la1) * rad, dLo = (lo2 - lo1) * rad;
  const h = Math.sin(dLa / 2) ** 2 + Math.cos(la1 * rad) * Math.cos(la2 * rad) * Math.sin(dLo / 2) ** 2;
  return 2 * 6371008.8 * Math.asin(Math.sqrt(h));
}

/** Places nearest a trip's start first, each with from_m (straight line); unchanged without a start. The page
 *  and find-quest-places both order with this, so they agree. */
export function orderFrom(results, from) {
  if (!from) return results;
  return results.map((p) => ({ ...p, from_m: Math.round(crowMetres(from, [p.lat, p.lon])) })).sort((a, b) => a.from_m - b.from_m);
}

const isPoint = (p) => Array.isArray(p) && p.length === 2 && p.every(Number.isFinite);

export const actions = {
  /** Mississippi addresses matching free text, best first. `partial`: the text is still being typed, so the
   *  last word and the town match by prefix (the page's suggestions as someone types). */
  async findAddress(ctx, { address, limit = 5, partial = false }) {
    const n = Math.min(20, Math.max(1, Math.trunc(Number(limit) || 5)));
    const { matches, error } = await ctx.ask({ fn: partial ? "suggest" : "geocode", q: String(address ?? ""), limit: n });
    return error ? { error } : matches;
  },

  /** Businesses and other places by kind near somewhere ("gas stations near Hattiesburg"), closest to that
   *  place first, or with `from` [lat, lon] (a trip's start) nearest it first. `near` [lat, lon] is used when the
   *  query names no place. Shown on the page's map and list. */
  async findPlaces(ctx, { query, near, from, limit = 20 }) {
    const n = Math.min(50, Math.max(1, Math.trunc(Number(limit) || 20)));
    const { places, error } = await ctx.ask({ fn: "places", q: String(query ?? ""), near: isPoint(near) ? near : null, limit: n });
    if (error) return { error };
    ctx.showPlaces?.(places);
    const start = isPoint(from) ? from : null;
    return {
      what: places.what, near: places.near.label, ordered_from: start ? "from" : "near",
      results: orderFrom(places.results, start).map((p, i) => ({
        n: i + 1, ...p, miles: miles(p.distance_m), ...(start ? { miles_from_start: miles(p.from_m) } : {}),
      })),
    };
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
    name: "find-quest-places",
    description: "Find businesses and other places in Mississippi by kind or name near somewhere, nearest first: "
      + "\"gas stations near Hattiesburg\", \"barbecue in Clarksdale\", \"Walmart near Oxford\". Pass `from` to order them from a trip's start. Results appear as numbered pins "
      + "on the page's map. Each result works as from/to in get-quest-directions. Runs on this device; nothing is sent anywhere.",
    inputSchema: {
      type: "object",
      properties: {
        query: { type: "string", minLength: 2, description: "What and where: \"<kind or name> near <town, address or place>\"." },
        near: { type: "array", items: { type: "number" }, minItems: 2, maxItems: 2,
                description: "[lat, lon] to search around when the query names no place." },
        from: { type: "array", items: { type: "number" }, minItems: 2, maxItems: 2,
                description: "[lat, lon] of the trip's start (a find-quest-address match's lat/lon): results are then ordered nearest it first, with miles_from_start (straight line)." },
        limit: { type: "integer", minimum: 1, maximum: 50, default: 20 },
      },
      required: ["query"],
    },
    execute: async (args) => ok(await actions.findPlaces(ctx, args)),
  });
  mc.registerTool({
    name: "find-quest-address",
    description: "Find Mississippi street addresses, or businesses and places by name, best first (partial: true for text still being typed). Each match has a label and a quality: "
      + "exact (a mapped address), interpolated (placed along its block), approximate (number not found; nearest block) or place (a business, with its kind). "
      + "Pass a match as from/to in get-quest-directions when text is ambiguous. Runs on this device; nothing is sent anywhere.",
    inputSchema: {
      type: "object",
      properties: {
        address: { type: "string", minLength: 3, description: "Street address, with a town or ZIP code if known." },
        limit: { type: "integer", minimum: 1, maximum: 20, default: 5 },
        partial: { type: "boolean", default: false,
                   description: "The text is still being typed: match its last word and town by prefix, as the page's suggestions do." },
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
