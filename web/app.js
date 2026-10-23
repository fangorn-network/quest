// The page: two addresses in, a printable list of turns out. All lookups go to worker.js in this tab.
import { actions, registerAgent } from "./agent.js";

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
    status("Map data ready. Nothing you type leaves this device.");
    $("go").disabled = false;
    drawMap();
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
    note.textContent = NOTES[m.quality];
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
  $("r-from").textContent = route.from + NOTES[chosen.from.quality];
  $("r-to").textContent = route.to + NOTES[chosen.to.quality];
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
  Object.assign(view, { bbox: null, moves: view.moves + 1, route: route.path, from: [chosen.from.lat, chosen.from.lon], to: [chosen.to.lat, chosen.to.lon] });
  drawMap();
}

// ---- The map: drawn by quest-core as SVG from the bundle's roads, redrawn for every move. ----
const view = { bbox: null, route: [], from: null, to: null, busy: false, again: false, moves: 0 };

async function drawMap() {
  if (!meta) return;
  if (view.busy) { view.again = true; return; }
  view.busy = true;
  const el = $("map"), moves = view.moves;
  const out = await ask({ fn: "map", bbox: view.bbox, width: el.clientWidth, height: el.clientHeight,
                          route: view.route, from: view.from, to: view.to });
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
  let drag = null;
  el.addEventListener("pointerdown", (e) => { drag = { x: e.clientX, y: e.clientY }; el.setPointerCapture(e.pointerId); });
  el.addEventListener("pointermove", (e) => {
    const svg = drag && el.querySelector("svg");
    if (svg) svg.style.transform = `translate(${e.clientX - drag.x}px, ${e.clientY - drag.y}px)`;
  });
  el.addEventListener("pointerup", (e) => {
    if (!drag) return;
    const [dx, dy] = [e.clientX - drag.x, e.clientY - drag.y];
    drag = null;
    if (Math.abs(dx) + Math.abs(dy) > 2) pan(dx, dy);
  });
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
  let resize;
  new ResizeObserver(() => { clearTimeout(resize); resize = setTimeout(drawMap, 150); }).observe(el);
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
  const pick = (loc) => { chosen[side] = loc; input.value = loc.label; $(`${side}-choices`).replaceChildren(); close(); };
  const open = (matches) => {
    items = matches;
    list.replaceChildren(...matches.map((m, i) => {
      const li = Object.assign(document.createElement("li"), { id: `${side}-opt-${i}`, textContent: m.label });
      li.setAttribute("role", "option");
      li.addEventListener("pointerdown", (e) => { e.preventDefault(); pick(m); }); // before the input loses focus
      return li;
    }));
    list.hidden = !matches.length;
    input.setAttribute("aria-expanded", String(matches.length > 0));
    highlight(-1);
  };
  input.addEventListener("input", () => {
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
  const [a, b] = [$("from-choices"), $("to-choices")];
  const kids = [[...a.childNodes], [...b.childNodes]];
  a.replaceChildren(...kids[1]);
  b.replaceChildren(...kids[0]);
});

$("print").addEventListener("click", () => window.print());

// What agent.js's actions use: quest-core, and a way to paint a route on the page (for the form and for
// agents alike, so the person sees directions an agent asked for).
const ctx = {
  ask,
  show(route, from, to) {
    for (const [side, loc] of [["from", from], ["to", to]]) {
      chosen[side] = loc;
      $(side).value = loc.label;
      $(`${side}-choices`).replaceChildren();
    }
    render(route);
  },
};

if ("serviceWorker" in navigator) navigator.serviceWorker.register("sw.js").catch(() => {});
