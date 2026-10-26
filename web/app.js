// The page: two addresses in, a printable list of turns out. All lookups go to worker.js in this tab.
import { actions, orderFrom, registerAgent } from "./agent.js";

const $ = (id) => document.getElementById(id);
const worker = new Worker("worker.js");
const pending = new Map();
let nextId = 0, meta = null;
const chosen = { from: null, to: null }; // the Location picked for each field, until its text changes

const ask = (req) => new Promise((resolve) => {
  const id = ++nextId;
  pending.set(id, resolve);
  worker.postMessage({ ...req, id });
});

function status(text, error = false) {
  $("status").textContent = text;
  $("status").classList.toggle("error", error);
}

worker.onmessage = ({ data }) => {
  if (data.type === "progress") {
    status(`Downloading Mississippi map data: ${(data.loaded / 1e6).toFixed(1)} of ${(data.total / 1e6).toFixed(1)} MB`);
  } else if (data.type === "ready") {
    meta = data.meta;
    status(data.checked === "record"
      ? "Map data ready, matching quest's on-chain record. Nothing you type leaves this device."
      : "Map data ready. Nothing you type leaves this device.");
    $("go").disabled = false;
    drawMap();
    $("find-go").disabled = false;
    registerAgent(ctx); // after the data loads, so a tool never answers from an empty map
  } else if (data.type === "error") {
    status(data.message, true);
  } else {
    pending.get(data.id)?.(data.result);
    pending.delete(data.id);
  }
};

const miles = (m) => m / 1609.344;
function distance(m) {
  return miles(m) < 0.1 ? `${Math.max(10, Math.round((m * 3.28084) / 10) * 10)} ft` : `${miles(m).toFixed(1)} mi`;
}
function duration(s) {
  const min = Math.max(1, Math.round(s / 60)), h = Math.floor(min / 60);
  return h ? `${h} h ${min % 60} min` : `${min} min`;
}
const NOTES = {
  exact: "",
  interpolated: "",
  approximate: " (house number not found; using the nearest known block)",
  place: "",
};

// Show the matches for one field as radio buttons, the first selected.
function showChoices(side, matches) {
  const box = $(`${side}-choices`);
  box.replaceChildren();
  const fs = document.createElement("fieldset");
  const legend = document.createElement("legend");
  legend.textContent = "Which one?";
  fs.append(legend);
  matches.forEach((m, i) => {
    const label = document.createElement("label");
    const radio = Object.assign(document.createElement("input"), { type: "radio", name: `${side}-pick`, checked: i === 0 });
    radio.addEventListener("change", () => { chosen[side] = m; });
    const note = document.createElement("small");
    note.textContent = m.kind ? ` ${m.kind}` : (NOTES[m.quality] ?? "");
    label.append(radio, " ", m.label, note);
    fs.append(label);
  });
  box.append(fs);
  chosen[side] = matches[0];
}

function fieldError(side, text) {
  const p = Object.assign(document.createElement("p"), { className: "field-error", textContent: text });
  $(`${side}-choices`).replaceChildren(p);
}

// Resolve one field to a Location, or null after showing the person what to fix.
async function resolve(side) {
  if (chosen[side]) return { loc: chosen[side], confirm: false };
  const text = $(side).value.trim();
  if (!text) { fieldError(side, "Enter an address."); return null; }
  const found = await actions.findAddress(ctx, { address: text, limit: 6 });
  const matches = found.error ? [] : found;
  if (!matches.length) { fieldError(side, `No match for “${text}” in Mississippi. Check the street name, or add the town or ZIP code.`); return null; }
  if (matches.length === 1) { $(`${side}-choices`).replaceChildren(); chosen[side] = matches[0]; return { loc: matches[0], confirm: false }; }
  showChoices(side, matches);
  return { loc: matches[0], confirm: true };
}

function render(route) {
  $("r-from").textContent = route.from + (NOTES[chosen.from.quality] ?? "");
  $("r-to").textContent = route.to + (NOTES[chosen.to.quality] ?? "");
  $("r-total").textContent = `${miles(route.distance_m).toFixed(1)} mi, about ${duration(route.duration_s)}`;
  $("r-steps").replaceChildren(...route.steps.map((s, i) => {
    const tr = document.createElement("tr");
    for (const [text, cls] of [[`${i + 1}.`], [s.text], [s.distance_m > 0 ? distance(s.distance_m) : "", "num"]]) {
      const td = Object.assign(document.createElement("td"), { textContent: text });
      if (cls) td.className = cls;
      tr.append(td);
    }
    return tr;
  }));
  $("r-date").textContent = new Date().toLocaleDateString();
  $("r-data").textContent = `${meta.osm_date?.slice(0, 10) ?? "unknown"} (OpenStreetMap) and ${meta.tiger_year} (TIGER)`;
  $("result").hidden = false;
  Object.assign(view, { bbox: null, moves: view.moves + 1, pins: [], anchor: null, route: route.path, from: [chosen.from.lat, chosen.from.lon], to: [chosen.to.lat, chosen.to.lon] });
  drawMap();
}

// ---- The map: drawn by quest-core as SVG from the bundle's roads, redrawn for every move. ----
const view = { bbox: null, route: [], from: null, to: null, pins: [], anchor: null, busy: false, again: false, moves: 0 };

async function drawMap() {
  if (!meta) return;
  if (view.busy) { view.again = true; return; }
  view.busy = true;
  const el = $("map"), moves = view.moves;
  const out = await ask({ fn: "map", bbox: view.bbox, width: el.clientWidth, height: el.clientHeight,
                          route: view.route, from: view.from, to: view.to, pins: view.pins, anchor: view.anchor });
  if (view.moves === moves) view.bbox = out.bbox; // else the person moved the map meanwhile: keep their box
  el.innerHTML = out.svg; // markup from quest-core; it holds no text anyone typed
  view.busy = false;
  if (view.again) { view.again = false; drawMap(); }
}

// Zoom by factor f about a point given as fractions of the map's width and height.
function zoom(f, fx = 0.5, fy = 0.5) {
  if (!view.bbox) return;
  const [s, w, n, e] = view.bbox;
  if ((e - w) / f < 0.004 || (e - w) / f > 20) return; // from a few blocks to a few states
  const lat = n - fy * (n - s), lon = w + fx * (e - w);
  view.bbox = [lat - (lat - s) / f, lon - (lon - w) / f, lat + (n - lat) / f, lon + (e - lon) / f];
  view.moves++;
  drawMap();
}

function pan(dx, dy) {
  if (!view.bbox) return;
  const el = $("map"), [s, w, n, e] = view.bbox;
  const dlon = (dx * (e - w)) / el.clientWidth, dlat = (dy * (n - s)) / el.clientHeight;
  view.bbox = [s + dlat, w - dlon, n + dlat, e - dlon];
  view.moves++;
  drawMap();
}

{
  const el = $("map");
  // One finger (or a mouse) drags; two pinch. The drawn map follows the fingers as a CSS transform, and is
  // redrawn once when the gesture ends, so a gesture costs one render however long it lasts.
  const pointers = new Map(); // pointerId -> { x, y }
  let gesture = null;         // where the fingers started: centre x, y and spread d
  const now = () => {
    const ps = [...pointers.values()];
    if (ps.length < 2) return { x: ps[0].x, y: ps[0].y, d: 0 };
    return { x: (ps[0].x + ps[1].x) / 2, y: (ps[0].y + ps[1].y) / 2, d: Math.hypot(ps[0].x - ps[1].x, ps[0].y - ps[1].y) };
  };
  const scaleOf = (g, n) => (g.d && n.d ? n.d / g.d : 1);
  // Turn the gesture so far into a new view: zoom about where the pinch started, then pan.
  const commit = () => {
    if (!gesture || !pointers.size) return;
    const n = now(), s = scaleOf(gesture, n), r = el.getBoundingClientRect();
    const [dx, dy] = [n.x - gesture.x, n.y - gesture.y];
    if (Math.abs(s - 1) > 0.02) zoom(s, (gesture.x - r.left) / r.width, (gesture.y - r.top) / r.height);
    if (Math.abs(dx) + Math.abs(dy) > 2) pan(dx, dy);
    gesture = null;
  };
  const begin = () => { gesture = pointers.size ? now() : null; };
  el.addEventListener("pointerdown", (e) => {
    commit(); // a second finger arriving ends the drag so far, and starts a pinch
    pointers.set(e.pointerId, { x: e.clientX, y: e.clientY });
    el.setPointerCapture(e.pointerId);
    begin();
  });
  el.addEventListener("pointermove", (e) => {
    if (!pointers.has(e.pointerId) || !gesture) return;
    pointers.set(e.pointerId, { x: e.clientX, y: e.clientY });
    // Move the map's world only: the scale bar stays put until the redraw gives it its new length.
    const world = el.querySelector("svg .m-world"), n = now(), r = el.getBoundingClientRect();
    if (!world) return;
    world.style.transformOrigin = `${gesture.x - r.left}px ${gesture.y - r.top}px`;
    world.style.transform = `translate(${n.x - gesture.x}px, ${n.y - gesture.y}px) scale(${scaleOf(gesture, n)})`;
  });
  const end = (e) => {
    if (!pointers.has(e.pointerId)) return;
    pointers.set(e.pointerId, { x: e.clientX, y: e.clientY });
    commit();
    pointers.delete(e.pointerId);
    begin(); // a finger left: carry on dragging with the one still down
  };
  el.addEventListener("pointerup", end);
  el.addEventListener("pointercancel", end);
  el.addEventListener("wheel", (e) => {
    e.preventDefault();
    const r = el.getBoundingClientRect();
    const dy = e.deltaY * (e.deltaMode === 1 ? 33 : e.deltaMode === 2 ? 800 : 1); // lines or pages to pixels
    zoom(Math.exp(-dy * 0.002), (e.clientX - r.left) / r.width, (e.clientY - r.top) / r.height); // ~1.2x per notch
  }, { passive: false });
  el.addEventListener("keydown", (e) => {
    const step = { ArrowLeft: [1, 0], ArrowRight: [-1, 0], ArrowUp: [0, 1], ArrowDown: [0, -1] }[e.key];
    if (step) pan(step[0] * el.clientWidth / 4, step[1] * el.clientHeight / 4);
    else if (e.key === "+" || e.key === "=") zoom(1.6);
    else if (e.key === "-") zoom(1 / 1.6);
    else return;
    e.preventDefault();
  });
  $("zoom-in").addEventListener("click", () => zoom(1.6));
  $("zoom-out").addEventListener("click", () => zoom(1 / 1.6));
  $("zoom-fit").addEventListener("click", () => { view.bbox = null; view.moves++; drawMap(); });
  // Redraw when the map's size really changes (a rotated phone), not on every scroll: mobile browsers resize
  // the viewport as their address bar shows and hides.
  let resize, drawnSize = "";
  new ResizeObserver(() => {
    const size = `${el.clientWidth}x${el.clientHeight}`;
    if (size === drawnSize) return;
    clearTimeout(resize);
    resize = setTimeout(() => { drawnSize = size; drawMap(); }, 150);
  }).observe(el);
}

$("trip").addEventListener("submit", async (ev) => {
  ev.preventDefault();
  $("go").disabled = true;
  try {
    const [from, to] = await Promise.all([resolve("from"), resolve("to")]);
    if (!from || !to) return status("Fix the address marked above.", true);
    if (from.confirm || to.confirm) return status("More than one place matches. Pick the right one, then press Get directions.");
    status("Finding the route…");
    const out = await actions.getDirections(ctx, { from: from.loc, to: to.loc });
    if (out.error) return status(out.error, true);
    status("Directions ready. Nothing you typed left this device.");
    $("result-title").focus?.();
  } finally {
    $("go").disabled = !meta;
  }
});

// ---- Suggestions under each field as the person types: quest-core in the worker, so nothing leaves the tab. ----
// An ARIA combobox: arrows move, Enter picks, Escape closes; picking locks in that exact place.
function combobox(side) {
  const input = $(side), list = $(`${side}-list`);
  let items = [], active = -1, seq = 0, timer;
  const highlight = (i) => {
    active = i;
    [...list.children].forEach((li, k) => li.setAttribute("aria-selected", String(k === i)));
    if (i < 0) return input.removeAttribute("aria-activedescendant");
    input.setAttribute("aria-activedescendant", list.children[i].id);
    list.children[i].scrollIntoView({ block: "nearest" });
  };
  const close = () => { list.hidden = true; input.setAttribute("aria-expanded", "false"); highlight(-1); };
  const pick = (loc) => {
    chosen[side] = loc;
    input.value = loc.label;
    $(`${side}-choices`).replaceChildren();
    close();
    if (side === "from") fromChanged();
  };
  const open = (matches) => {
    items = matches;
    list.replaceChildren(...matches.map((m, i) => {
      const li = Object.assign(document.createElement("li"), { id: `${side}-opt-${i}`, textContent: m.label });
      if (m.kind) li.append(Object.assign(document.createElement("small"), { textContent: m.kind }));
      li.setAttribute("role", "option");
      li.addEventListener("pointerdown", (e) => { e.preventDefault(); pick(m); }); // before the input loses focus
      return li;
    }));
    list.hidden = !matches.length;
    input.setAttribute("aria-expanded", String(matches.length > 0));
    highlight(-1);
  };
  input.addEventListener("input", () => {
    if (chosen[side] && side === "from") queueMicrotask(fromChanged);
    chosen[side] = null;
    $(`${side}-choices`).replaceChildren();
    clearTimeout(timer);
    const mine = ++seq, text = input.value.trim();
    if (text.length < 3 || !meta) return close();
    timer = setTimeout(async () => {
      const found = await actions.findAddress(ctx, { address: text, limit: 6, partial: true });
      if (mine === seq && document.activeElement === input) open(found.error ? [] : found); // the latest keystroke only
    }, 120);
  });
  input.addEventListener("keydown", (e) => {
    if (list.hidden) {
      if (e.key === "ArrowDown" && items.length && !chosen[side]) { open(items); highlight(0); e.preventDefault(); }
      return;
    }
    if (e.key === "ArrowDown") highlight((active + 1) % items.length);
    else if (e.key === "ArrowUp") highlight(active <= 0 ? items.length - 1 : active - 1);
    else if (e.key === "Enter" && active >= 0) pick(items[active]);
    else if (e.key === "Enter") return close(); // nothing highlighted: close, and the form submits
    else if (e.key === "Escape") close();
    else return; // typing: the input handler refreshes the list
    e.preventDefault();
  });
  input.addEventListener("blur", close);
}
combobox("from");
combobox("to");

$("swap").addEventListener("click", () => {
  [$("from").value, $("to").value] = [$("to").value, $("from").value];
  [chosen.from, chosen.to] = [chosen.to, chosen.from];
  fromChanged();
  const [a, b] = [$("from-choices"), $("to-choices")];
  const kids = [[...a.childNodes], [...b.childNodes]];
  a.replaceChildren(...kids[1]);
  b.replaceChildren(...kids[0]);
});

$("print").addEventListener("click", () => window.print());

// ---- Find places: "gas stations near Hattiesburg" -> numbered pins on the map and a list to pick from. ----
function findStatus(text, error = false) {
  $("find-status").textContent = text;
  $("find-status").classList.toggle("error", error);
}

// Use a found place as one end of the trip; with the other end filled in too, get directions.
function useAs(side, loc) {
  chosen[side] = loc;
  $(side).value = loc.label;
  $(`${side}-choices`).replaceChildren();
  if (side === "from") fromChanged();
  const other = side === "from" ? "to" : "from";
  if ($(other).value.trim()) $("trip").requestSubmit();
  else $(other).focus();
}

// Found places, nearest the person's From address first, with straight-line distances from it; without a From,
// in the order found (closest to the place searched near) and with no distance: where a search was centred (a
// town's middle, the map's centre) is not where they are. The map's pin numbers follow the list.
let lastPlaces = null;

function renderPlaceList() {
  if (!lastPlaces) return;
  const { what, near } = lastPlaces;
  const origin = chosen.from && [chosen.from.lat, chosen.from.lon];
  const results = orderFrom(lastPlaces.results, origin);
  const list = $("place-list");
  list.replaceChildren(...results.map((p, i) => {
    const li = document.createElement("li");
    const n = Object.assign(document.createElement("span"), { className: "n", textContent: String(i + 1) });
    const text = document.createElement("div");
    const [name, ...rest] = p.label.split(", ");
    const parts = [p.kind, rest.join(", ")];
    if (origin) parts.push(`${distance(p.from_m)} away`);
    text.append(Object.assign(document.createElement("strong"), { textContent: name }), document.createElement("br"),
      Object.assign(document.createElement("span"), { className: "what", textContent: parts.join(" · ") }));
    const go = Object.assign(document.createElement("div"), { className: "go" });
    for (const [side, label] of [["to", "To here"], ["from", "From here"]]) {
      const b = Object.assign(document.createElement("button"), { type: "button", textContent: label });
      b.setAttribute("aria-label", `${label}: ${name}`);
      b.addEventListener("click", () => useAs(side, p));
      go.append(b);
    }
    li.append(n, text, go);
    return li;
  }));
  list.hidden = !results.length;
  const What = `${what[0].toUpperCase()}${what.slice(1)} near ${near.label}`;
  const order = origin
    ? `nearest ${chosen.from.label.split(", ")[0]} first (straight-line distances)`
    : `closest to ${near.label} first. Add a From address to see how far each is from you`;
  findStatus(results.length
    ? `${What}: ${results.length} found, ${order}. Nothing you typed left this device.`
    : `Nothing found for ${what} near ${near.label}.`);
  view.pins = results.map((p) => [p.lat, p.lon]);
  drawMap();
}

// From changed (picked, typed over, swapped, used from a result, confirmed): show where it is on the map (the
// A marker) once it is one known place, and re-order the places list from it.
function fromChanged() {
  view.from = chosen.from ? [chosen.from.lat, chosen.from.lon] : null;
  if (lastPlaces) renderPlaceList(); // draws the map too
  else drawMap();
}

function showPlaces(places) {
  lastPlaces = places;
  Object.assign(view, { bbox: null, moves: view.moves + 1, anchor: [places.near.lat, places.near.lon] });
  renderPlaceList();
}

$("find").addEventListener("submit", async (ev) => {
  ev.preventDefault();
  const query = $("q").value.trim();
  if (!query) return findStatus("Type what to find, like “gas stations near Hattiesburg”.", true);
  const [s, w, n, e] = view.bbox ?? [];
  const near = view.bbox ? [(s + n) / 2, (w + e) / 2] : null; // the map's centre, when the query names no place
  const fromText = $("from").value.trim();
  if (!chosen.from && fromText) {
    const found = await actions.findAddress(ctx, { address: fromText, limit: 2 });
    if (Array.isArray(found) && found.length === 1) chosen.from = found[0]; // ambiguous: no distances until picked
    if (chosen.from) view.from = [chosen.from.lat, chosen.from.lon];
  }
  const out = await actions.findPlaces(ctx, { query, near, limit: 20 });
  if (out.error) findStatus(out.error, true);
});

// What agent.js's actions use: quest-core, and ways to paint a route or found places on the page (for the
// page's own forms and for agents alike, so the person sees what an agent asked for).
const ctx = {
  ask,
  showPlaces,
  show(route, from, to) {
    for (const [side, loc] of [["from", from], ["to", to]]) {
      chosen[side] = loc;
      $(side).value = loc.label;
      $(`${side}-choices`).replaceChildren();
    }
    render(route);
    fromChanged();
  },
};

if ("serviceWorker" in navigator) navigator.serviceWorker.register("sw.js").catch(() => {});
