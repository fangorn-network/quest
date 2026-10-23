//! The map, drawn as SVG from the bundle's own roads: no tile server, so nothing about where anyone is
//! looking leaves the device. The page styles the classes (m-major, m-route, ...) for light, dark and print.

use crate::Bundle;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fmt::Write;

#[derive(Deserialize)]
pub struct MapRequest {
    /// [south, west, north, east] in degrees; omitted, the map fits the route, or else the whole region.
    pub bbox: Option<[f64; 4]>,
    pub width: f64,
    pub height: f64,
    #[serde(default)]
    pub route: Vec<[f64; 2]>,
    pub from: Option<[f64; 2]>,
    pub to: Option<[f64; 2]>,
    /// Places search results, drawn as numbered pins in this order, and where they were searched near.
    #[serde(default)]
    pub pins: Vec<[f64; 2]>,
    pub anchor: Option<[f64; 2]>,
}

#[derive(Serialize)]
pub struct MapOut {
    pub svg: String,
    /// The box actually drawn: the request's, widened to the map's shape.
    pub bbox: [f64; 4],
}

/// Per edge [min lat, min lon, max lat, max lon] in 1e-7 degrees, so a view only decodes the roads it shows.
pub struct Map {
    boxes: Vec<[i32; 4]>,
    region: [f64; 4],
}

// ponytail: which roads to draw at which zoom, by the view's width in km; a knob, tune by eye.
fn max_view_km(class: &str) -> f64 {
    match class {
        "motorway" | "trunk" | "primary" => f64::INFINITY,
        "secondary" => 150.0,
        "tertiary" => 60.0,
        c if c.ends_with("_link") => 40.0,
        "unclassified" | "residential" => 12.0,
        _ => 3.0,
    }
}

fn group(class: &str) -> usize {
    match class {
        "motorway" | "motorway_link" | "trunk" | "trunk_link" => 3,
        "primary" | "primary_link" => 2,
        c if c.starts_with("secondary") || c.starts_with("tertiary") => 1,
        _ => 0,
    }
}
const GROUPS: [&str; 4] = ["m-minor", "m-secondary", "m-primary", "m-major"];

// ponytail: which roads carry labels at which zoom, and how far apart; knobs like max_view_km, tuned by eye.
// The rank orders placement: a lower rank is placed first and wins where labels would overlap.
fn shield_rank(class: &str, view_km: f64) -> Option<u8> {
    match class {
        "motorway" => Some(0),
        "trunk" => Some(1),
        "primary" if view_km <= 150.0 => Some(2),
        _ => None,
    }
}
fn name_rank(class: &str, view_km: f64) -> Option<u8> {
    match class {
        "motorway" | "trunk" | "primary" if view_km <= 25.0 => Some(3),
        "secondary" | "tertiary" if view_km <= 25.0 => Some(4),
        "unclassified" | "residential" | "living_street" if view_km <= 6.0 => Some(5),
        "service" if view_km <= 2.0 => Some(6),
        _ => None,
    }
}
const SHIELD_EVERY_PX: f64 = 240.0;
const NAME_EVERY_PX: f64 = 280.0;
const MAX_LABELS: usize = 400;

/// OSM names are anyone's text, and the SVG goes into the page: escape them.
fn xml(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;").replace('\'', "&apos;")
}

/// A route number as its shield shows it, and the shield's class.
fn shield(r: &str) -> (String, &'static str) {
    match r.strip_prefix("I ") {
        Some(n) => (format!("I-{n}"), "m-i"),
        None if r.starts_with("US ") => (r.to_string(), "m-us"),
        None => (r.to_string(), "m-ms"),
    }
}

/// The edges carrying one label, joined end to end into polylines (lat, lon): a road's name runs across
/// many short edges, too short alone to hold it.
fn chains(b: &Bundle, edges: &[u32]) -> Vec<Vec<(f64, f64)>> {
    let mut at: HashMap<u32, Vec<usize>> = HashMap::new();
    for (i, &e) in edges.iter().enumerate() {
        at.entry(b.edge_u[e as usize]).or_default().push(i);
        at.entry(b.edge_v[e as usize]).or_default().push(i);
    }
    let mut used = vec![false; edges.len()];
    let mut out = Vec::new();
    for start in 0..edges.len() {
        if used[start] {
            continue;
        }
        used[start] = true;
        let e0 = edges[start] as usize;
        let mut pts = b.shape(e0 as u32);
        for forward in [true, false] {
            let mut node = if forward { b.edge_v[e0] } else { b.edge_u[e0] };
            while let Some(&i) = at.get(&node).and_then(|v| v.iter().find(|&&i| !used[i])) {
                used[i] = true;
                let e = edges[i] as usize;
                let mut more = b.shape(e as u32); // oriented to start at `node`
                if b.edge_u[e] != node {
                    more.reverse();
                }
                node = if b.edge_u[e] == node { b.edge_v[e] } else { b.edge_u[e] };
                if forward {
                    pts.extend(more.into_iter().skip(1));
                } else {
                    more.reverse();
                    let n = more.len();
                    pts.splice(0..0, more.into_iter().take(n - 1));
                }
            }
        }
        out.push(pts);
    }
    out
}

/// Where a label `w` px long fits along a polyline (px), about every `spacing` px: (x, y, upright angle).
/// Skipped where the road bends too much over the label's length to read straight across.
fn spots(pts: &[(f64, f64)], w: f64, spacing: f64) -> Vec<(f64, f64, f64)> {
    let mut cum = vec![0.0];
    for p in pts.windows(2) {
        cum.push(cum.last().unwrap() + (p[1].0 - p[0].0).hypot(p[1].1 - p[0].1));
    }
    let total = *cum.last().unwrap();
    if pts.len() < 2 || total < w + 16.0 {
        return Vec::new();
    }
    let at = |d: f64| {
        let k = cum.partition_point(|&c| c < d).clamp(1, pts.len() - 1);
        let seg = cum[k] - cum[k - 1];
        let u = if seg > 0.0 { (d - cum[k - 1]) / seg } else { 0.0 };
        (pts[k - 1].0 + u * (pts[k].0 - pts[k - 1].0), pts[k - 1].1 + u * (pts[k].1 - pts[k - 1].1))
    };
    let n = (total / spacing).floor().max(1.0) as usize;
    (0..n)
        .filter_map(|i| {
            let d = total * (i as f64 + 0.5) / n as f64;
            let (a, c) = (at(d - w / 2.0), at(d + w / 2.0));
            if (c.0 - a.0).hypot(c.1 - a.1) < 0.9 * w {
                return None;
            }
            let mut angle = (c.1 - a.1).atan2(c.0 - a.0).to_degrees();
            if angle > 90.0 {
                angle -= 180.0;
            } else if angle <= -90.0 {
                angle += 180.0;
            }
            let (x, y) = at(d);
            Some((x, y, angle))
        })
        .collect()
}

impl Map {
    pub fn new(b: &Bundle) -> Map {
        let mut region = [f64::MAX, f64::MAX, f64::MIN, f64::MIN];
        let boxes = (0..b.edge_u.len() as u32)
            .map(|e| {
                let mut bx = [i32::MAX, i32::MAX, i32::MIN, i32::MIN];
                let (u, v) = (b.edge_u[e as usize] as usize, b.edge_v[e as usize] as usize);
                let (mut la, mut lo) = (b.node_lat[u], b.node_lon[u]);
                let mut add = |la: i32, lo: i32| {
                    bx = [bx[0].min(la), bx[1].min(lo), bx[2].max(la), bx[3].max(lo)];
                };
                add(la, lo);
                for k in b.geo_first[e as usize] as usize..b.geo_first[e as usize + 1] as usize {
                    la += b.geo_lat[k];
                    lo += b.geo_lon[k];
                    add(la, lo);
                }
                add(b.node_lat[v], b.node_lon[v]);
                region = [
                    region[0].min(bx[0] as f64 * 1e-7),
                    region[1].min(bx[1] as f64 * 1e-7),
                    region[2].max(bx[2] as f64 * 1e-7),
                    region[3].max(bx[3] as f64 * 1e-7),
                ];
                bx
            })
            .collect();
        Map { boxes, region }
    }

    pub fn render(&self, b: &Bundle, req: &MapRequest) -> MapOut {
        let (w, h) = (req.width.clamp(100.0, 4000.0), req.height.clamp(100.0, 4000.0));
        // The box to show: asked for, else the route plus a margin, else the region.
        let [mut s, mut west, mut n, mut e] = req.bbox.unwrap_or_else(|| {
            // Fit the places found, where they were searched near and the trip's start (their distances are
            // from it); else the route; else the region.
            let fit: Vec<[f64; 2]> = if req.pins.is_empty() {
                req.route.clone()
            } else {
                req.pins.iter().chain(&req.anchor).chain(&req.from).copied().collect()
            };
            if fit.is_empty() {
                return self.region;
            }
            let mut bx = [f64::MAX, f64::MAX, f64::MIN, f64::MIN];
            for p in &fit {
                bx = [bx[0].min(p[0]), bx[1].min(p[1]), bx[2].max(p[0]), bx[3].max(p[1])];
            }
            let pad = ((bx[2] - bx[0]).max(bx[3] - bx[1]) * 0.08).max(0.002);
            [bx[0] - pad, bx[1] - pad, bx[2] + pad, bx[3] + pad]
        });
        // Widen it to the map's shape, keeping the centre.
        let k = ((s + n) / 2.0).to_radians().cos();
        let scale = (w / ((e - west) * k)).min(h / (n - s)); // pixels per degree of latitude
        let (cx, cy) = ((west + e) / 2.0, (s + n) / 2.0);
        let (half_lon, half_lat) = (w / scale / k / 2.0, h / scale / 2.0);
        (s, west, n, e) = (cy - half_lat, cx - half_lon, cy + half_lat, cx + half_lon);
        let px = |la: f64, lo: f64| ((lo - west) * k * scale, (n - la) * scale);

        let view_km = (e - west) * k * 111.32;
        let allowed: Vec<bool> = b.classes.iter().map(|c| view_km <= max_view_km(c)).collect();
        let q = |d: f64| (d * 1e7) as i32;
        let (qs, qw, qn, qe) = (q(s), q(west), q(n), q(e));
        let mut paths = [String::new(), String::new(), String::new(), String::new()];
        // Label text -> (rank, edges): (true, "US 49") a shield, (false, "N State St") a name.
        let mut labelled: HashMap<(bool, String), (u8, Vec<u32>)> = HashMap::new();
        for (i, bx) in self.boxes.iter().enumerate() {
            let class = b.edge_class[i] as usize;
            if !allowed[class] || bx[2] < qs || bx[0] > qn || bx[3] < qw || bx[1] > qe {
                continue;
            }
            line(&mut paths[group(&b.classes[class])], b.shape(i as u32).iter().map(|&(la, lo)| px(la, lo)));
            let mut add = |shield: bool, text: &str, rank: u8| {
                let entry = labelled.entry((shield, text.to_string())).or_insert((rank, Vec::new()));
                entry.0 = entry.0.min(rank);
                entry.1.push(i as u32);
            };
            if let Some(rank) = shield_rank(&b.classes[class], view_km) {
                for r in b.names[b.edge_ref[i] as usize].split(';').map(str::trim).filter(|r| !r.is_empty()) {
                    add(true, r, rank);
                }
            }
            let name = &b.names[b.edge_name[i] as usize];
            if let (Some(rank), false) = (name_rank(&b.classes[class], view_km), name.is_empty()) {
                add(false, name, rank);
            }
        }
        let mut route = String::new();
        line(&mut route, req.route.iter().map(|p| px(p[0], p[1])));

        let mut svg = String::new();
        let _ = write!(
            svg,
            r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 {w:.0} {h:.0}" role="img" aria-label="Map{}"><rect class="m-bg" width="100%" height="100%"/>"#,
            if req.route.is_empty() { "" } else { " of the route" }
        );
        for (g, d) in paths.iter().enumerate() {
            if !d.is_empty() {
                let _ = write!(svg, r#"<path class="{}" d="{d}"/>"#, GROUPS[g]);
            }
        }
        if !route.is_empty() {
            let _ = write!(svg, r#"<path class="m-route-casing" d="{route}"/><path class="m-route" d="{route}"/>"#);
        }
        // Labels: shields first, then names by road, each dropped where it would overlap one already placed
        // (or the A/B markers), and only where it fits inside the map.
        let mut taken: Vec<[f64; 4]> = [req.from, req.to]
            .into_iter()
            .flatten()
            .chain(req.pins.iter().copied())
            .map(|[la, lo]| {
                let (x, y) = px(la, lo);
                [x - 12.0, y - 12.0, x + 12.0, y + 12.0]
            })
            .collect();
        let mut order: Vec<_> = labelled.into_iter().collect();
        order.sort_by(|a, b| (a.1 .0, &a.0).cmp(&(b.1 .0, &b.0)));
        let mut placed = 0;
        for ((is_shield, text), (_, edges)) in order {
            let (shown, class) = if is_shield { shield(&text) } else { (text.clone(), "") };
            let chars = shown.chars().count() as f64;
            let (lw, lh) = if is_shield { (chars * 6.6 + 10.0, 16.0) } else { (chars * 6.1 + 4.0, 12.0) };
            for chain in chains(b, &edges) {
                let pts: Vec<(f64, f64)> = chain.iter().map(|&(la, lo)| px(la, lo)).collect();
                for (x, y, angle) in spots(&pts, lw, if is_shield { SHIELD_EVERY_PX } else { NAME_EVERY_PX }) {
                    let a = if is_shield { 0.0 } else { angle.to_radians() };
                    let hx = a.cos().abs() * lw / 2.0 + a.sin().abs() * lh / 2.0 + 3.0;
                    let hy = a.sin().abs() * lw / 2.0 + a.cos().abs() * lh / 2.0 + 3.0;
                    let r = [x - hx, y - hy, x + hx, y + hy];
                    let inside = r[0] > 2.0 && r[1] > 2.0 && r[2] < w - 2.0 && r[3] < h - 2.0;
                    if placed >= MAX_LABELS || !inside || taken.iter().any(|t| r[0] < t[2] && t[0] < r[2] && r[1] < t[3] && t[1] < r[3]) {
                        continue;
                    }
                    taken.push(r);
                    placed += 1;
                    let _ = if is_shield {
                        write!(svg, r#"<g class="m-shield {class}" transform="translate({x:.1} {y:.1})"><rect x="{:.1}" y="-8" width="{lw:.1}" height="16" rx="3"/><text dy="0.35em">{}</text></g>"#,
                               -lw / 2.0, xml(&shown))
                    } else {
                        write!(svg, r#"<text class="m-label" transform="translate({x:.1} {y:.1}) rotate({angle:.1})" dy="0.35em">{}</text>"#, xml(&shown))
                    };
                }
            }
        }
        if let Some([la, lo]) = req.anchor {
            let (x, y) = px(la, lo);
            let _ = write!(svg, r#"<circle class="m-anchor" cx="{x:.1}" cy="{y:.1}" r="6"/>"#);
        }
        // Pins last-numbered first, so number 1 is drawn on top where they overlap.
        for (n, [la, lo]) in req.pins.iter().enumerate().rev() {
            let (x, y) = px(*la, *lo);
            let _ = write!(
                svg,
                r#"<g class="m-pin"><circle cx="{x:.1}" cy="{y:.1}" r="9"/><text x="{x:.1}" y="{:.1}">{}</text></g>"#,
                y + 3.5,
                n + 1
            );
        }
        for (pt, class, label) in [(req.from, "m-start", "A"), (req.to, "m-end", "B")] {
            if let Some([la, lo]) = pt {
                let (x, y) = px(la, lo);
                let _ = write!(
                    svg,
                    r#"<g class="{class}"><circle cx="{x:.1}" cy="{y:.1}" r="9"/><text x="{x:.1}" y="{:.1}">{label}</text></g>"#,
                    y + 4.0
                );
            }
        }
        // Scale bar: the roundest number of miles near a fifth of the width.
        let miles_per_px = 69.0 / scale; // a degree of latitude is about 69 miles
        let target = w / 5.0 * miles_per_px;
        let mag = 10f64.powf(target.log10().floor());
        let nice = [1.0, 2.0, 5.0, 10.0].into_iter().map(|m| m * mag).filter(|&m| m <= target).last().unwrap_or(mag);
        let bar = nice / miles_per_px;
        let label = if nice < 0.1 { format!("{:.0} ft", nice * 5280.0) } else { format!("{nice} mi") };
        let _ = write!(
            svg,
            r#"<g class="m-scale"><path d="M12 {y0:.0}v6h{bar:.1}v-6"/><text x="{:.1}" y="{:.0}">{label}</text></g></svg>"#,
            12.0 + bar + 6.0,
            h - 12.0,
            y0 = h - 18.0
        );
        MapOut { svg, bbox: [s, west, n, e] }
    }
}

/// Append one polyline to an SVG path, dropping points under a pixel from the last one drawn.
fn line(d: &mut String, pts: impl Iterator<Item = (f64, f64)>) {
    let mut last: Option<(f64, f64)> = None;
    let mut first = true;
    let mut tail = None;
    for (x, y) in pts {
        tail = Some((x, y));
        if last.is_some_and(|(lx, ly)| (x - lx).abs() < 0.7 && (y - ly).abs() < 0.7) {
            continue;
        }
        let _ = write!(d, "{}{x:.1} {y:.1}", if first { "M" } else { "L" });
        first = false;
        last = Some((x, y));
    }
    // Keep the end point, so roads still meet at junctions.
    if let (Some(t), Some(l)) = (tail, last) {
        if t != l {
            let _ = write!(d, "L{:.1} {:.1}", t.0, t.1);
        }
    }
}
