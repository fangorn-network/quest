//! The map, drawn as SVG from the bundle's own roads: no tile server, so nothing about where anyone is
//! looking leaves the device. The page styles the classes (m-major, m-route, ...) for light, dark and print.

use crate::Bundle;
use serde::{Deserialize, Serialize};
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
            if req.route.is_empty() {
                return self.region;
            }
            let mut bx = [f64::MAX, f64::MAX, f64::MIN, f64::MIN];
            for p in &req.route {
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
        for (i, bx) in self.boxes.iter().enumerate() {
            let class = b.edge_class[i] as usize;
            if !allowed[class] || bx[2] < qs || bx[0] > qn || bx[3] < qw || bx[1] > qe {
                continue;
            }
            line(&mut paths[group(&b.classes[class])], b.shape(i as u32).iter().map(|&(la, lo)| px(la, lo)));
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
        let label = if nice < 1.0 { format!("{:.0} ft", nice * 5280.0) } else { format!("{nice} mi") };
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
