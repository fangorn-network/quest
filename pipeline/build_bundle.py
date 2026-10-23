"""Build one region's bundle: OSM roads + TIGER house-number ranges + OSM address points, in one gzip file.

    .venv/bin/python pipeline/build_bundle.py            # .ship/raw -> .ship/bundle/us-ms.bin.gz + manifest

The bundle holds no road geometry: TIGER ranges are mapped onto OSM edges here, so the page only needs
the graph, the edge bearings (for turn instructions) and (edge, fraction) for every address. Format:
"QST1", uint32 header length, header JSON, then little-endian typed arrays at the offsets the header lists.
The output is deterministic, so the same inputs give the same sha256.
"""
import argparse, array, glob, gzip, hashlib, json, math, re, struct, sys
from collections import defaultdict
from pathlib import Path

HERE = Path(__file__).parent
NORM = json.loads((HERE / "normalize.json").read_text())
PATTERNS = [(re.compile(p), re.sub(r"\$(\d)", r"\\g<\1>", r)) for p, r in NORM["patterns"]]


def normalize(name):
    s = re.sub(r"['\u2019]", "", name.upper())
    s = " ".join(re.sub(r"[^A-Z0-9 ]", " ", s).split())
    for p, r in PATTERNS:
        s = p.sub(r, s)
    return " ".join(NORM["tokens"].get(t, t) for t in s.split())


# ponytail: rough US speeds in km/h where OSM has no maxspeed; this table is the calibration knob.
SPEED = {"motorway": 105, "motorway_link": 64, "trunk": 88, "trunk_link": 56, "primary": 80,
         "primary_link": 48, "secondary": 72, "secondary_link": 48, "tertiary": 56, "tertiary_link": 40,
         "unclassified": 56, "residential": 40, "living_street": 16, "service": 24}
CLASSES = list(SPEED)
SKIP_SERVICE = {"parking_aisle", "driveway", "drive-through", "emergency_access"}
ONEWAY, ROUNDABOUT = 1, 2  # edge flags


def drivable(t):
    hw = t.get("highway")
    if hw not in SPEED or t.get("area") == "yes":
        return False
    if hw == "service" and t.get("service") in SKIP_SERVICE:
        return False
    mv = t.get("motor_vehicle") or t.get("motorcar")
    # ponytail: private roads are dropped; addresses on them snap to the nearest public road
    return mv not in ("no", "private") if mv else t.get("access") not in ("no", "private")


def speed(t):
    m = re.match(r"\s*(\d+)\s*(mph)?", t.get("maxspeed", ""))
    return min(255, round(int(m[1]) * (1.609 if m[2] else 1))) if m else SPEED[t["highway"]]


def oneway(t):
    o = t.get("oneway", "")
    if o in ("yes", "1", "true"):
        return 1
    if o == "-1":
        return -1
    if o == "no":
        return 0
    return 1 if t.get("junction") in ("roundabout", "circular") or t["highway"] == "motorway" else 0


R = 6371008.8


def haversine(a, b):
    (la1, lo1), (la2, lo2) = a, b
    p1, p2 = math.radians(la1), math.radians(la2)
    h = math.sin((p2 - p1) / 2) ** 2 + math.cos(p1) * math.cos(p2) * math.sin(math.radians(lo2 - lo1) / 2) ** 2
    return 2 * R * math.asin(math.sqrt(h))


def bearing(a, b):
    p1, p2, dl = math.radians(a[0]), math.radians(b[0]), math.radians(b[1] - a[1])
    y = math.sin(dl) * math.cos(p2)
    x = math.cos(p1) * math.sin(p2) - math.sin(p1) * math.cos(p2) * math.cos(dl)
    return math.degrees(math.atan2(y, x)) % 360


def end_bearings(pts):
    """Bearing leaving the first point and arriving at the last, each measured over >= 10 m so a
    stub of geometry at the junction doesn't decide the turn."""
    def far(seq):
        d = 0
        for k in range(1, len(seq)):
            d += haversine(seq[k - 1], seq[k])
            if d >= 10:
                return seq[k]
        return seq[-1]
    return bearing(pts[0], far(pts)), bearing(far(pts[::-1]), pts[-1])


def pieces_for(samples):
    """samples: [(f, edge, t)] along one TIGER line, f ascending. Returns pieces (edge, f0, f1, t0, t1)
    covering f in [0, 1]: runs of samples on the same edge, split halfway between runs, with t
    extrapolated linearly to each piece's ends. t0/t1 may fall outside [0, 1]; locate clamps."""
    runs = []
    for s in samples:
        if runs and runs[-1][-1][1] == s[1]:
            runs[-1].append(s)
        else:
            runs.append([s])
    out = []
    for k, run in enumerate(runs):
        f0 = 0.0 if k == 0 else (runs[k - 1][-1][0] + run[0][0]) / 2
        f1 = 1.0 if k == len(runs) - 1 else (run[-1][0] + runs[k + 1][0][0]) / 2
        (fa, e, ta), (fb, _, tb) = run[0], run[-1]
        slope = (tb - ta) / (fb - fa) if fb > fa else 0.0
        out.append((e, f0, f1, ta + (f0 - fa) * slope, ta + (f1 - fa) * slope))
    return out


def locate(pieces, f):
    """House-number fraction f along a TIGER line -> (edge, t) on the OSM graph."""
    e, f0, f1, t0, t1 = next((p for p in pieces if f <= p[2]), pieces[-1])
    return e, min(1.0, max(0.0, t0 + ((f - f0) / (f1 - f0) if f1 > f0 else 0) * (t1 - t0)))


def simplify(pts, tol_m=5.0):
    """Douglas-Peucker: the fewest points of a polyline that stay within tol_m metres of it."""
    if len(pts) < 3:
        return pts
    k = math.cos(math.radians(pts[0][0]))
    xy = [(lo * 111320 * k, la * 110540) for la, lo in pts]
    keep, stack = {0, len(pts) - 1}, [(0, len(pts) - 1)]
    while stack:
        a, b = stack.pop()
        (ax, ay), (bx, by) = xy[a], xy[b]
        dx, dy = bx - ax, by - ay
        norm = math.hypot(dx, dy)
        far, far_d = None, tol_m
        for i in range(a + 1, b):
            px, py = xy[i]
            d = abs(dx * (ay - py) - dy * (ax - px)) / norm if norm else math.hypot(px - ax, py - ay)
            if d > far_d:
                far, far_d = i, d
        if far is not None:
            keep.add(far)
            stack += [(a, far), (far, b)]
    return [pts[i] for i in sorted(keep)]


def house_fraction(h, h_from, h_to):
    return 0.5 if h_from == h_to else min(1.0, max(0.0, (h - h_from) / (h_to - h_from)))


def morton(x, y):
    z = 0
    for i in range(16):
        z |= ((x >> i) & 1) << (2 * i) | ((y >> i) & 1) << (2 * i + 1)
    return z


class Grid:
    """Edge segments bucketed by ~50 m cells, for snapping points onto the graph at build time."""
    CELL = 0.0005

    def __init__(self, geoms):
        self.geoms, self.cells = geoms, defaultdict(list)
        self.cum = []
        for e, pts in enumerate(geoms):
            c = [0.0]
            for k in range(1, len(pts)):
                c.append(c[-1] + haversine(pts[k - 1], pts[k]))
                (a, b), (p, q) = pts[k - 1], pts[k]
                for cx in range(self.key(min(a, p)), self.key(max(a, p)) + 1):
                    for cy in range(self.key(min(b, q)), self.key(max(b, q)) + 1):
                        self.cells[cx, cy].append((e, k))
            self.cum.append(c)

    def key(self, deg):
        return math.floor(deg / self.CELL)

    def snap(self, lat, lon, names, keys, maxd):
        """Nearest edge to (lat, lon): one whose name is in `names` if any lies within maxd,
        else the nearest within min(maxd, 40 m). Returns (edge, t) or None."""
        kx, ky = math.cos(math.radians(lat)) * 111320, 110540
        r = math.ceil(maxd / 45)
        cx, cy = self.key(lat), self.key(lon)
        best, best_named, seen = None, None, set()
        for i in range(cx - r, cx + r + 1):
            for j in range(cy - r, cy + r + 1):
                for e, k in self.cells.get((i, j), ()):
                    if (e, k) in seen:
                        continue
                    seen.add((e, k))
                    (a, b), (p, q) = self.geoms[e][k - 1], self.geoms[e][k]
                    ax, ay, px, py = (b - lon) * kx, (a - lat) * ky, (q - lon) * kx, (p - lat) * ky
                    dx, dy = px - ax, py - ay
                    u = 0.0 if dx == dy == 0 else min(1.0, max(0.0, -(ax * dx + ay * dy) / (dx * dx + dy * dy)))
                    d = math.hypot(ax + u * dx, ay + u * dy)
                    c = self.cum[e]
                    hit = (d, e, (c[k - 1] + u * (c[k] - c[k - 1])) / c[-1] if c[-1] else 0.0)
                    if d <= maxd and keys[e] & names and (best_named is None or hit < best_named):
                        best_named = hit
                    if d <= min(maxd, 40) and (best is None or hit < best):
                        best = hit
        hit = best_named or best
        return (hit[1], hit[2]) if hit else None


def read_osm(pbf):
    import osmium
    ways, points, restrictions = [], [], []
    fp = osmium.FileProcessor(str(pbf)).with_locations().with_filter(
        osmium.filter.KeyFilter("highway", "addr:housenumber", "type"))
    for o in fp:
        t = o.tags
        if o.is_way() and drivable(t):
            nodes = [(n.ref, n.lat, n.lon) for n in o.nodes if n.location.valid()]
            if len(nodes) >= 2:
                ways.append((o.id, nodes, {k: t.get(k) for k in ("highway", "name", "ref") if k in t},
                             oneway(t), speed(t), t.get("junction") in ("roundabout", "circular")))
        if "addr:housenumber" in t and "addr:street" in t and (o.is_node() or o.is_way()):
            if o.is_node():
                lat, lon = o.lat, o.lon
            else:
                ll = [(n.lat, n.lon) for n in o.nodes if n.location.valid()]
                if not ll:
                    continue
                lat, lon = sum(p[0] for p in ll) / len(ll), sum(p[1] for p in ll) / len(ll)
            points.append((t["addr:housenumber"], t["addr:street"], t.get("addr:postcode", ""),
                           t.get("addr:city", ""), lat, lon))
        if o.is_relation() and t.get("type") == "restriction":
            kind = t.get("restriction") or t.get("restriction:motorcar") or ""
            m = {role: [(x.type, x.ref) for x in o.members if x.role == role] for role in ("from", "via", "to")}
            ok = kind.startswith(("no_", "only_")) and len(m["from"]) == len(m["via"]) == len(m["to"]) == 1 \
                and m["from"][0][0] == m["to"][0][0] == "w" and m["via"][0][0] == "n"
            restrictions.append((m["from"][0][1], m["via"][0][1], m["to"][0][1], int(kind.startswith("only_")))
                                if ok else None)
    return ways, points, restrictions


def build_graph(ways):
    use = defaultdict(int)
    for _, nodes, *_ in ways:
        for ref, *_ in nodes:
            use[ref] += 1
        use[nodes[0][0]] += 1
        use[nodes[-1][0]] += 1
    edges = []  # (u_osm, v_osm, pts, way_id, tags, flags, speed)
    for wid, nodes, tags, ow, spd, rb in ways:
        start = 0
        for k in range(1, len(nodes)):
            if use[nodes[k][0]] >= 2 or k == len(nodes) - 1:
                seg = nodes[start:k + 1]
                if ow == -1:
                    seg = seg[::-1]
                flags = (ONEWAY if ow else 0) | (ROUNDABOUT if rb else 0)
                edges.append((seg[0][0], seg[-1][0], [(la, lo) for _, la, lo in seg], wid, tags, flags, spd))
                start = k
    # Junctions in Morton order and edges sorted by their ends, so nearby things sit together and compress.
    coords = {}
    for u, v, pts, *_ in edges:
        coords[u], coords[v] = pts[0], pts[-1]
    lats, lons = [c[0] for c in coords.values()], [c[1] for c in coords.values()]
    la0, lo0 = min(lats), min(lons)
    sx, sy = 65535 / (max(lats) - la0), 65535 / (max(lons) - lo0)
    order = sorted(coords, key=lambda n: (morton(int((coords[n][0] - la0) * sx), int((coords[n][1] - lo0) * sy)), n))
    index = {n: i for i, n in enumerate(order)}
    edges.sort(key=lambda e: (index[e[0]], index[e[1]], e[3]))
    return order, coords, index, edges


def read_tiger(paths):
    import shapefile
    lines, ranges = {}, []  # tlid -> (pts, names); ranges: (tlid, display, zip, from, to, parity)
    dropped = 0
    for path in paths:
        for sr in shapefile.Reader(path).iterShapeRecords():
            r = sr.record.as_dict()
            tlid = r["TLID"]
            if tlid not in lines:
                lines[tlid] = ([(lat, lon) for lon, lat in sr.shape.points], set())
            lines[tlid][1].add(normalize(r["FULLNAME"]))
            for side in "LR":
                hf, ht, z, par = r[side + "FROMHN"], r[side + "TOHN"], r["ZIP" + side], r["PARITY" + side]
                if not hf or not ht:
                    continue
                if not (hf.isdigit() and ht.isdigit() and z.isdigit()):
                    dropped += 1
                    continue
                ranges.append((tlid, r["FULLNAME"], int(z), int(hf), int(ht), " OEB".index(par or "B")))
    return lines, ranges, dropped


def read_places(path, points):
    places = defaultdict(set)
    with open(path, encoding="utf-8-sig") as f:
        next(f)
        for line in f:
            c = line.rstrip("\n").split("|")
            if c[1] and c[9].startswith("28"):
                places[normalize(re.sub(r" (city|town|village|CDP)$", "", c[10]))].add(int(c[1]))
    for _, _, z, city, *_ in points:
        if city and z[:5].isdigit():
            places[normalize(city)].add(int(z[:5]))
    return {k: sorted(v) for k, v in sorted(places.items())}


DTYPE = {"i": "int32", "I": "uint32", "f": "float32", "B": "uint8", "H": "uint16"}


def write_bundle(path, header, arrays):
    assert sys.byteorder == "little" and array.array("I").itemsize == 4
    blobs, off, header = [], 0, {**header, "arrays": []}
    for name, code, data in arrays:
        b = array.array(code, data).tobytes()
        header["arrays"].append({"name": name, "type": DTYPE[code], "length": len(b) // array.array(code).itemsize,
                                 "offset": off})
        blobs.append(b + b"\0" * (-len(b) % 4))
        off += len(blobs[-1])
    hb = json.dumps(header, separators=(",", ":"), ensure_ascii=False).encode()
    hb += b" " * (-len(hb) % 4)
    raw = b"QST1" + struct.pack("<I", len(hb)) + hb + b"".join(blobs)
    gz = gzip.compress(raw, 9, mtime=0)
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_bytes(gz)
    return len(raw), gz


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--raw", default=".ship/raw")
    ap.add_argument("--out", default=".ship/bundle/us-ms.bin.gz")
    ap.add_argument("--region", default="us-ms")
    ap.add_argument("--tiger-year", default="2026")
    a = ap.parse_args()
    raw, out = Path(a.raw), Path(a.out)
    import osmium
    osm_date = osmium.io.Reader(str(raw / "mississippi.osm.pbf")).header().get("osmosis_replication_timestamp")

    ways, points, restrictions = read_osm(raw / "mississippi.osm.pbf")
    order, coords, index, edges = build_graph(ways)
    print(f"osm {osm_date}: {len(ways)} drivable ways -> {len(order)} junctions, {len(edges)} edges", flush=True)

    names, name_ix = [""], {"": 0}
    def nid(s):
        if s not in name_ix:
            name_ix[s] = len(names)
            names.append(s)
        return name_ix[s]
    keys = [{normalize(x) for x in [t.get("name", "")] + t.get("ref", "").split(";") if x.strip()}
            for *_, t, _, _ in edges]
    grid = Grid([e[2] for e in edges])

    # Turn restrictions: from-edge arriving at the via node, to-edge leaving it.
    by_way = defaultdict(list)
    for i, (u, v, _, wid, *_) in enumerate(edges):
        by_way[wid].append(i)
    res, res_skipped = [], 0
    for r in restrictions:
        if r is None or r[1] not in index:
            res_skipped += 1
            continue
        frm, via, to, kind = r
        fe = [i for i in by_way.get(frm, ()) if via in (edges[i][0], edges[i][1])]
        te = [i for i in by_way.get(to, ()) if via in (edges[i][0], edges[i][1])]
        if len(fe) == 1 and len(te) == 1:
            res.append((fe[0], index[via], te[0], kind))
        else:
            res_skipped += 1

    # TIGER ranges onto OSM edges: sample each line every ~50 m and snap the samples.
    tiger_paths = sorted(glob.glob(str(raw / "tiger" / f"tl_{a.tiger_year}_*_addrfeat.zip")))
    lines, ranges, rng_dropped = read_tiger(tiger_paths)
    used = {r[0] for r in ranges}
    pieces, piece_of = [], {}
    for n, tlid in enumerate(sorted(used)):
        pts, tnames = lines[tlid]
        cum = [0.0]
        for k in range(1, len(pts)):
            cum.append(cum[-1] + haversine(pts[k - 1], pts[k]))
        count = max(3, math.ceil(cum[-1] / 50))
        samples, k = [], 1
        for s in range(count):
            f = (s + 0.5) / count
            d = f * cum[-1]
            while k < len(pts) - 1 and cum[k] < d:
                k += 1
            seg = cum[k] - cum[k - 1]
            u = (d - cum[k - 1]) / seg if seg else 0
            lat = pts[k - 1][0] + u * (pts[k][0] - pts[k - 1][0])
            lon = pts[k - 1][1] + u * (pts[k][1] - pts[k - 1][1])
            hit = grid.snap(lat, lon, tnames, keys, 40)
            if hit:
                samples.append((f, *hit))
        if samples:
            ps = pieces_for(samples)
            piece_of[tlid] = (len(pieces), len(ps))
            pieces.extend(ps)
        if n % 20000 == 0:
            print(f"  tiger {n}/{len(used)} lines", flush=True)
    streets, street_ix = [], {}
    def sid(display):
        if display not in street_ix:
            street_ix[display] = len(streets)
            streets.append([display, normalize(display)])
        return street_ix[display]
    rng = sorted({(sid(d), z, hf, ht, par, *piece_of[t]) for t, d, z, hf, ht, par in ranges if t in piece_of})
    rng_unmatched = sum(1 for r in ranges if r[0] not in piece_of)

    # OSM address points: exact positions, preferred over interpolation when both exist. A third of them
    # carry no postcode; those take the ZIP of a TIGER range on the edge they snap to.
    edge_zip = {}
    for r in rng:
        for e, *_ in pieces[r[5]:r[5] + r[6]]:
            edge_zip.setdefault(e, r[1])
    pts_out, pt_dropped = [], 0
    for hn, street, z, city, lat, lon in points:
        hit = grid.snap(lat, lon, {normalize(street)}, keys, 150) if hn.isdigit() else None
        zip_ = int(z[:5]) if z[:5].isdigit() else edge_zip.get(hit[0]) if hit else None
        if hit and zip_:
            pts_out.append((sid(street), zip_, int(hn), *hit))
        else:
            pt_dropped += 1
    pts_out.sort()

    places = read_places(raw / "zcta_place.txt", points)
    e_b = [end_bearings(e[2]) for e in edges]
    # Road shapes for the map: each edge's interior points, simplified to within 5 m, each stored as the
    # step from the point before it (the first from the edge's u), which compresses a quarter smaller.
    geo_first, geo_lat, geo_lon = [0], [], []
    for e in edges:
        pla, plo = round(e[2][0][0] * 1e7), round(e[2][0][1] * 1e7)
        for la, lo in simplify(e[2])[1:-1]:
            la, lo = round(la * 1e7), round(lo * 1e7)
            geo_lat.append(la - pla)
            geo_lon.append(lo - plo)
            pla, plo = la, lo
        geo_first.append(len(geo_lat))
    bdeg = lambda d: round(d * 256 / 360) % 256
    header = {
        "format": 1, "region": a.region, "osm_date": osm_date, "tiger_year": int(a.tiger_year),
        "attribution": "© OpenStreetMap contributors (ODbL); US Census Bureau TIGER/Line",
        "classes": CLASSES, "flags": {"oneway": ONEWAY, "roundabout": ROUNDABOUT},
        "parity": ["", "odd", "even", "both"], "names": names, "streets": streets, "places": places,
    }
    arrays = [
        ("node_lat", "i", [round(coords[n][0] * 1e7) for n in order]),
        ("node_lon", "i", [round(coords[n][1] * 1e7) for n in order]),
        ("edge_u", "I", [index[e[0]] for e in edges]),
        ("edge_v", "I", [index[e[1]] for e in edges]),
        ("edge_len", "f", [c[-1] for c in grid.cum]),
        ("edge_speed", "B", [e[6] for e in edges]),
        ("edge_flags", "B", [e[5] for e in edges]),
        ("edge_class", "B", [CLASSES.index(e[4]["highway"]) for e in edges]),
        ("edge_name", "I", [nid(e[4].get("name", "")) for e in edges]),
        ("edge_ref", "I", [nid(e[4].get("ref", "")) for e in edges]),
        ("edge_b0", "B", [bdeg(b[0]) for b in e_b]),
        ("edge_b1", "B", [bdeg(b[1]) for b in e_b]),
        ("geo_first", "I", geo_first), ("geo_lat", "i", geo_lat), ("geo_lon", "i", geo_lon),
        ("res_from", "I", [r[0] for r in res]), ("res_via", "I", [r[1] for r in res]),
        ("res_to", "I", [r[2] for r in res]), ("res_kind", "B", [r[3] for r in res]),
        ("rng_street", "I", [r[0] for r in rng]), ("rng_zip", "I", [r[1] for r in rng]),
        ("rng_from", "I", [r[2] for r in rng]), ("rng_to", "I", [r[3] for r in rng]),
        ("rng_parity", "B", [r[4] for r in rng]),
        ("rng_piece", "I", [r[5] for r in rng]), ("rng_npieces", "B", [min(255, r[6]) for r in rng]),
        ("pc_edge", "I", [p[0] for p in pieces]),
        ("pc_f0", "f", [p[1] for p in pieces]), ("pc_f1", "f", [p[2] for p in pieces]),
        ("pc_t0", "f", [p[3] for p in pieces]), ("pc_t1", "f", [p[4] for p in pieces]),
        ("pt_street", "I", [p[0] for p in pts_out]), ("pt_zip", "I", [p[1] for p in pts_out]),
        ("pt_number", "I", [p[2] for p in pts_out]), ("pt_edge", "I", [p[3] for p in pts_out]),
        ("pt_t", "f", [p[4] for p in pts_out]),
    ]
    raw_len, gz = write_bundle(out, header, arrays)
    manifest = {
        "region": a.region, "file": out.name, "bytes": len(gz), "sha256": hashlib.sha256(gz).hexdigest(),
        "osm_date": osm_date, "tiger_year": int(a.tiger_year),
        "counts": {"junctions": len(order), "edges": len(edges), "restrictions": len(res),
                   "restrictions_skipped": res_skipped, "tiger_lines": len(used), "tiger_lines_unmatched":
                   len(used) - len(piece_of), "ranges": len(rng), "ranges_unmatched": rng_unmatched,
                   "ranges_unparsed": rng_dropped, "pieces": len(pieces), "address_points": len(pts_out),
                   "address_points_dropped": pt_dropped, "streets": len(streets), "places": len(places)},
    }
    out.with_suffix("").with_suffix(".json").write_text(json.dumps(manifest, indent=1) + "\n")
    print(json.dumps(manifest["counts"], indent=1))
    print(f"{out}: {raw_len / 1e6:.1f} MB raw, {len(gz) / 1e6:.1f} MB gzip, sha256 {manifest['sha256']}")


if __name__ == "__main__":
    main()
