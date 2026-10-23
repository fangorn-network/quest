//! Fastest route between two points on the graph (A* over directed edges, honouring turn restrictions),
//! then turn-by-turn steps from the edge bearings. math.md §8 has the formulas.

use crate::{Bundle, Location, ONEWAY, ROUNDABOUT};
use serde::Serialize;
use std::cmp::Reverse;
use std::collections::{BinaryHeap, HashMap};

#[derive(Serialize, Debug)]
pub struct Step {
    pub text: String,
    pub distance_m: f64,
    pub lat: f64,
    pub lon: f64,
}

#[derive(Serialize, Debug)]
pub struct Route {
    pub from: String,
    pub to: String,
    pub distance_m: f64,
    pub duration_s: f64,
    pub steps: Vec<Step>,
    /// The road the route follows, as [lat, lon] points, for the map.
    pub path: Vec<[f64; 2]>,
}

/// An arc is one direction of an edge: 2e for u->v, 2e+1 for v->u.
pub struct Graph {
    first: Vec<u32>,
    arcs: Vec<u32>,
    secs: Vec<f32>,
    vmax: f64,
    restrict: HashMap<(u32, u32), Vec<(u32, u8)>>,
}

const NONE: u32 = u32::MAX;

fn haversine((la1, lo1): (f64, f64), (la2, lo2): (f64, f64)) -> f64 {
    let (p1, p2) = (la1.to_radians(), la2.to_radians());
    let h = ((p2 - p1) / 2.0).sin().powi(2) + p1.cos() * p2.cos() * ((lo2 - lo1).to_radians() / 2.0).sin().powi(2);
    2.0 * 6371008.8 * h.sqrt().asin()
}

fn deg(b: u8) -> f64 {
    b as f64 * 360.0 / 256.0
}

/// Signed turn from heading `a` to heading `c`, in [-180, 180); positive is right.
fn turn(a: f64, c: f64) -> f64 {
    (c - a + 540.0).rem_euclid(360.0) - 180.0
}

// ponytail: fixed turn costs in seconds; a calibration knob, not a model of real intersections.
fn penalty(angle: f64) -> f64 {
    match angle.abs() {
        a if a < 45.0 => 0.0,
        a if a > 150.0 => 30.0,
        _ if angle > 0.0 => 5.0,
        _ => 12.0,
    }
}

impl Graph {
    pub fn new(b: &Bundle) -> Graph {
        let (n, m) = (b.node_lat.len(), b.edge_u.len());
        let mut first = vec![0u32; n + 1];
        for e in 0..m {
            first[b.edge_u[e] as usize + 1] += 1;
            if b.edge_flags[e] & ONEWAY == 0 {
                first[b.edge_v[e] as usize + 1] += 1;
            }
        }
        for i in 0..n {
            first[i + 1] += first[i];
        }
        let mut fill = first.clone();
        let mut arcs = vec![0u32; first[n] as usize];
        for e in 0..m {
            let u = b.edge_u[e] as usize;
            arcs[fill[u] as usize] = 2 * e as u32;
            fill[u] += 1;
            if b.edge_flags[e] & ONEWAY == 0 {
                let v = b.edge_v[e] as usize;
                arcs[fill[v] as usize] = 2 * e as u32 + 1;
                fill[v] += 1;
            }
        }
        let secs: Vec<f32> = (0..m).map(|e| b.edge_len[e] / (b.edge_speed[e].max(5) as f32 / 3.6)).collect();
        let vmax = b.edge_speed.iter().copied().max().unwrap_or(100).max(5) as f64 / 3.6;
        let mut restrict: HashMap<(u32, u32), Vec<(u32, u8)>> = HashMap::new();
        for i in 0..b.res_from.len() {
            restrict.entry((b.res_from[i], b.res_via[i])).or_default().push((b.res_to[i], b.res_kind[i]));
        }
        Graph { first, arcs, secs, vmax, restrict }
    }

    fn out(&self, n: u32) -> &[u32] {
        &self.arcs[self.first[n as usize] as usize..self.first[n as usize + 1] as usize]
    }

    fn head(b: &Bundle, a: u32) -> u32 {
        let e = (a >> 1) as usize;
        if a & 1 == 0 { b.edge_v[e] } else { b.edge_u[e] }
    }

    fn two_way(b: &Bundle, e: u32) -> bool {
        b.edge_flags[e as usize] & ONEWAY == 0
    }

    /// Heading leaving the arc's tail, and arriving at its head.
    fn leave(b: &Bundle, a: u32) -> f64 {
        let e = (a >> 1) as usize;
        if a & 1 == 0 { deg(b.edge_b0[e]) } else { (deg(b.edge_b1[e]) + 180.0) % 360.0 }
    }

    fn arrive(b: &Bundle, a: u32) -> f64 {
        let e = (a >> 1) as usize;
        if a & 1 == 0 { deg(b.edge_b1[e]) } else { (deg(b.edge_b0[e]) + 180.0) % 360.0 }
    }

    fn angle(b: &Bundle, a: u32, c: u32) -> f64 {
        turn(Self::arrive(b, a), Self::leave(b, c))
    }

    /// May a route arriving on arc `a` leave on arc `c`? No U-turns except at dead ends, and no turn a
    /// restriction forbids.
    fn allowed(&self, b: &Bundle, a: u32, c: u32) -> bool {
        let (ea, ec, via) = (a >> 1, c >> 1, Self::head(b, a));
        if ea == ec && a != c && self.out(via).len() > 1 {
            return false;
        }
        match self.restrict.get(&(ea, via)) {
            None => true,
            Some(rs) => {
                !rs.iter().any(|&(to, k)| k == 0 && to == ec)
                    && (!rs.iter().any(|&(_, k)| k == 1) || rs.iter().any(|&(to, k)| k == 1 && to == ec))
            }
        }
    }

    /// Ways onward from the head of `a`, other than turning back.
    fn exits(&self, b: &Bundle, a: u32) -> usize {
        self.out(Self::head(b, a)).iter().filter(|&&c| c >> 1 != a >> 1 && self.allowed(b, a, c)).count()
    }

    pub fn route(&self, b: &Bundle, from: &Location, to: &Location) -> Option<Route> {
        let (e0, t0, e1, t1) = (from.edge, from.t as f64, to.edge, to.t as f64);
        let (s0, s1) = (self.secs[e0 as usize] as f64, self.secs[e1 as usize] as f64);
        let goal = b.point(e1, to.t);
        let h = |n: u32| haversine(b.node(n), goal) / self.vmax;

        // best: (cost, last arc of the search or NONE for the direct case, final partial arc)
        let mut best: (f64, u32, u32) = (f64::INFINITY, NONE, NONE);
        if e0 == e1 {
            if t1 >= t0 {
                best = ((t1 - t0) * s0, NONE, 2 * e0);
            } else if Self::two_way(b, e0) {
                best = ((t0 - t1) * s0, NONE, 2 * e0 + 1);
            }
        }
        let mut g = vec![f64::INFINITY; self.secs.len() * 2];
        let mut parent = vec![NONE; g.len()];
        let mut heap = BinaryHeap::new();
        let mut seed = |a: u32, cost: f64, heap: &mut BinaryHeap<_>| {
            g[a as usize] = cost;
            heap.push(Reverse(((cost + h(Self::head(b, a))).to_bits(), a)));
        };
        seed(2 * e0, (1.0 - t0) * s0, &mut heap);
        if Self::two_way(b, e0) {
            seed(2 * e0 + 1, t0 * s0, &mut heap);
        }
        while let Some(Reverse((f, a))) = heap.pop() {
            let f = f64::from_bits(f);
            if f >= best.0 {
                break;
            }
            let ga = g[a as usize];
            let via = Self::head(b, a);
            if f > ga + h(via) + 1e-6 {
                continue; // stale entry
            }
            for (c, part) in [(2 * e1, t1), (2 * e1 + 1, 1.0 - t1)] {
                let tail = if c & 1 == 0 { b.edge_u[e1 as usize] } else { b.edge_v[e1 as usize] };
                if tail == via && (c & 1 == 0 || Self::two_way(b, e1)) && self.allowed(b, a, c) {
                    let total = ga + penalty(Self::angle(b, a, c)) + part * s1;
                    if total < best.0 {
                        best = (total, a, c);
                    }
                }
            }
            for &c in self.out(via) {
                if !self.allowed(b, a, c) {
                    continue;
                }
                let ng = ga + penalty(Self::angle(b, a, c)) + self.secs[(c >> 1) as usize] as f64;
                if ng < g[c as usize] {
                    g[c as usize] = ng;
                    parent[c as usize] = a;
                    heap.push(Reverse(((ng + h(Self::head(b, c))).to_bits(), c)));
                }
            }
        }
        if best.0.is_infinite() {
            return None;
        }

        // Spans: (arc, from, to) as fractions along the edge in travel order; the first and last are partial.
        let mut spans: Vec<(u32, f64, f64)> = Vec::new();
        let whole = |a: u32| if a & 1 == 0 { (0.0, 1.0) } else { (1.0, 0.0) };
        if best.1 == NONE {
            spans.push((best.2, t0, t1));
        } else {
            let mut a = best.1;
            while a != NONE {
                let (f, t) = whole(a);
                spans.push((a, f, t));
                a = parent[a as usize];
            }
            spans.reverse();
            spans[0].1 = t0;
            spans.push((best.2, whole(best.2).0, t1));
        }
        let legs: Vec<(u32, f64)> =
            spans.iter().map(|&(a, f, t)| (a, (t - f).abs() * b.edge_len[(a >> 1) as usize] as f64)).collect();
        let mut path: Vec<[f64; 2]> = Vec::new();
        for &(a, f, t) in &spans {
            let pts = b.cut(a >> 1, f, t);
            let skip = (!path.is_empty()) as usize; // each span starts where the last one ended
            path.extend(pts.into_iter().skip(skip).map(|(la, lo)| [la, lo]));
        }
        let steps = self.steps(b, &legs, (from.lat, from.lon), (to.lat, to.lon));
        Some(Route {
            from: from.label.clone(),
            to: to.label.clone(),
            distance_m: legs.iter().map(|l| l.1).sum(),
            duration_s: best.0,
            steps,
            path,
        })
    }

    fn steps(&self, b: &Bundle, legs: &[(u32, f64)], start: (f64, f64), end: (f64, f64)) -> Vec<Step> {
        let edge = |a: u32| (a >> 1) as usize;
        let name = |a: u32| b.names[b.edge_name[edge(a)] as usize].as_str();
        let refs = |a: u32| -> Vec<&str> {
            b.names[b.edge_ref[edge(a)] as usize].split(';').map(str::trim).filter(|s| !s.is_empty()).collect()
        };
        let display = |a: u32| -> String {
            let (n, r) = (name(a), refs(a).join("/"));
            match (n.is_empty(), r.is_empty()) {
                (false, false) => format!("{n} ({r})"),
                (false, true) => n.to_string(),
                (true, false) => r,
                (true, true) => "the unnamed road".to_string(),
            }
        };
        let same_road = |a: u32, c: u32| {
            let (na, nc) = (name(a), name(c));
            (!na.is_empty() && na == nc)
                || refs(a).iter().any(|r| refs(c).contains(r))
                || (na.is_empty() && nc.is_empty() && refs(a).is_empty() && refs(c).is_empty())
        };
        let link = |a: u32| b.classes[b.edge_class[edge(a)] as usize].ends_with("_link");
        let round = |a: u32| b.edge_flags[edge(a)] & ROUNDABOUT != 0;
        let side = |angle: f64| if angle > 0.0 { "right" } else { "left" };
        let at = |a: u32| {
            let e = edge(a);
            b.node(if a & 1 == 0 { b.edge_u[e] } else { b.edge_v[e] })
        };

        const COMPASS: [&str; 8] = ["north", "northeast", "east", "southeast", "south", "southwest", "west", "northwest"];
        let heading = COMPASS[((Self::leave(b, legs[0].0) / 45.0).round() as usize) % 8];
        let mut steps = vec![Step {
            text: format!("Head {heading} on {}", display(legs[0].0)),
            distance_m: legs[0].1,
            lat: start.0,
            lon: start.1,
        }];
        // Metres of the same road starting at leg i, so a few metres of another name don't become a step.
        let run = |i: usize| -> f64 {
            legs[i..].iter().take_while(|l| same_road(legs[i].0, l.0)).map(|l| l.1).sum()
        };
        // ponytail: 30 m is a guess at "a name blip, not a road"; tune it against the route checks.
        const BLIP_M: f64 = 30.0;
        let ramp_to = |i: usize| -> String {
            let roads = || (i..legs.len()).filter(|&k| !link(legs[k].0));
            roads().find(|&k| run(k) >= BLIP_M).or_else(|| roads().next()).map_or("the ramp".to_string(), |k| display(legs[k].0))
        };
        let mut quiet_until_road = false; // a ramp or roundabout step already named the road ahead
        let mut i = 1;
        while i < legs.len() {
            let (p, c, d) = (legs[i - 1].0, legs[i].0, legs[i].1);
            let angle = Self::angle(b, p, c);
            let text = if round(c) && !round(p) {
                // Count the exits passed until the route leaves the roundabout.
                let mut j = i;
                let mut n = 0;
                while j < legs.len() && round(legs[j].0) {
                    let a = legs[j].0;
                    if j + 1 < legs.len() && !round(legs[j + 1].0) {
                        n += 1;
                    } else if self.out(Self::head(b, a)).iter().any(|&x| !round(x) && self.allowed(b, a, x)) {
                        n += 1;
                    }
                    j += 1;
                }
                let onto = legs.get(j).map_or("your destination".to_string(), |l| if link(l.0) { ramp_to(j) } else { display(l.0) });
                let nth = match n {
                    1 => "1st".to_string(),
                    2 => "2nd".to_string(),
                    3 => "3rd".to_string(),
                    n => format!("{n}th"),
                };
                quiet_until_road = true;
                Some(format!("At the roundabout, take the {nth} exit onto {onto}"))
            } else if round(c) {
                None
            } else if link(c) && !link(p) && quiet_until_road {
                None // the roundabout step already named where this ramp goes
            } else if link(c) && !link(p) {
                let onto = ramp_to(i);
                quiet_until_road = true;
                Some(if angle.abs() < 10.0 {
                    format!("Take the ramp onto {onto}")
                } else {
                    format!("Take the ramp on the {} onto {onto}", side(angle))
                })
            } else if link(c) {
                (self.exits(b, p) >= 2).then(|| format!("Keep {} toward {}", side(angle), ramp_to(i)))
            } else if quiet_until_road {
                quiet_until_road = run(i) < BLIP_M;
                None
            } else if !same_road(p, c) && run(i) >= BLIP_M {
                let a = angle.abs();
                let verb = if a < 20.0 {
                    "Continue".to_string()
                } else if a < 60.0 {
                    format!("Bear {}", side(angle))
                } else if a < 150.0 {
                    format!("Turn {}", side(angle))
                } else {
                    "Make a U-turn".to_string()
                };
                Some(format!("{verb} onto {}", display(c)))
            } else if same_road(p, c) && angle.abs() >= 60.0 && self.exits(b, p) >= 2 {
                Some(format!("Turn {} to stay on {}", side(angle), display(c)))
            } else {
                None
            };
            match text {
                Some(text) if !(text.starts_with("Keep") && text == steps.last().unwrap().text) => {
                    let (lat, lon) = at(c);
                    steps.push(Step { text, distance_m: d, lat, lon });
                }
                _ => steps.last_mut().unwrap().distance_m += d,
            }
            i += 1;
        }
        steps.push(Step { text: "Arrive at your destination".into(), distance_m: 0.0, lat: end.0, lon: end.1 });
        steps
    }
}
