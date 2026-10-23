//! Typed address -> a point on the road graph, from the bundle's OSM address points and TIGER ranges.

use crate::Bundle;
use regex::Regex;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Location {
    pub label: String,
    pub edge: u32,
    pub t: f32,
    #[serde(default)]
    pub lat: f64,
    #[serde(default)]
    pub lon: f64,
    /// "exact" (an OSM address point), "interpolated" (inside a TIGER range), or "approximate" (the
    /// number wasn't found, so the nearest range on that street, or the street itself with no number).
    #[serde(default)]
    pub quality: String,
}

/// The same rules as pipeline/build_bundle.py's normalize(), from the same file.
pub struct Normalizer {
    patterns: Vec<(Regex, String)>,
    tokens: HashMap<String, String>,
}

impl Default for Normalizer {
    fn default() -> Self {
        #[derive(Deserialize)]
        struct N {
            patterns: Vec<(String, String)>,
            tokens: HashMap<String, String>,
        }
        let n: N = serde_json::from_str(include_str!("../../pipeline/normalize.json")).expect("normalize.json");
        Normalizer {
            patterns: n.patterns.into_iter().map(|(p, r)| (Regex::new(&p).expect("pattern"), r)).collect(),
            tokens: n.tokens,
        }
    }
}

impl Normalizer {
    pub fn normalize(&self, s: &str) -> String {
        let up: String = s
            .to_uppercase()
            .chars()
            .filter(|c| *c != '\'' && *c != '\u{2019}')
            .map(|c| if c.is_ascii_uppercase() || c.is_ascii_digit() { c } else { ' ' })
            .collect();
        let mut s = up.split_whitespace().collect::<Vec<_>>().join(" ");
        for (p, r) in &self.patterns {
            s = p.replace_all(&s, r.as_str()).into_owned();
        }
        s.split_whitespace().map(|t| self.tokens.get(t).map_or(t, String::as_str)).collect::<Vec<_>>().join(" ")
    }
}

struct Parsed {
    number: Option<u32>,
    street: String,
    city: Option<String>,
    zip: Option<u32>,
}

pub struct Geocoder {
    pub norm: Normalizer,
    keys: Vec<String>,
    by_key: HashMap<String, usize>,
    key_streets: Vec<Vec<u32>>,
    zip_place: HashMap<u32, String>,
}

fn digits(s: &str, n: usize) -> bool {
    s.len() == n && s.bytes().all(|c| c.is_ascii_digit())
}

fn title(s: &str) -> String {
    s.split(' ')
        .map(|w| {
            let mut c = w.chars();
            c.next().map_or(String::new(), |f| f.to_string() + &c.as_str().to_lowercase())
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// Edit distance where swapping two neighbouring letters ("HIHG") counts as one edit.
fn edit_distance(a: &str, b: &str) -> usize {
    let (a, b) = (a.as_bytes(), b.as_bytes());
    let mut rows = vec![(0..=b.len()).collect::<Vec<usize>>()];
    for i in 1..=a.len() {
        let mut cur = vec![i; b.len() + 1];
        for j in 1..=b.len() {
            let prev = &rows[i - 1];
            cur[j] = (prev[j - 1] + (a[i - 1] != b[j - 1]) as usize).min(prev[j] + 1).min(cur[j - 1] + 1);
            if i > 1 && j > 1 && a[i - 1] == b[j - 2] && a[i - 2] == b[j - 1] {
                cur[j] = cur[j].min(rows[i - 2][j - 2] + 1);
            }
        }
        rows.push(cur);
    }
    rows[a.len()][b.len()]
}

/// The edge and position for house-number fraction f along a TIGER line's pieces (build_bundle.locate).
fn locate(b: &Bundle, first: u32, n: u8, f: f32) -> (u32, f32) {
    let ps = first as usize..first as usize + n as usize;
    let k = ps.clone().find(|&k| f <= b.pc_f1[k]).unwrap_or(ps.end - 1);
    let (f0, f1, t0, t1) = (b.pc_f0[k], b.pc_f1[k], b.pc_t0[k], b.pc_t1[k]);
    let u = if f1 > f0 { (f - f0) / (f1 - f0) } else { 0.0 };
    (b.pc_edge[k], (t0 + u * (t1 - t0)).clamp(0.0, 1.0))
}

fn house_fraction(h: u32, from: u32, to: u32) -> f32 {
    if from == to {
        0.5
    } else {
        ((h as f32 - from as f32) / (to as f32 - from as f32)).clamp(0.0, 1.0)
    }
}

/// Indices i in a sorted array where arr[i] == key.
fn equal_range(arr: &[u32], key: u32) -> std::ops::Range<usize> {
    arr.partition_point(|&x| x < key)..arr.partition_point(|&x| x <= key)
}

impl Geocoder {
    pub fn new(b: &Bundle) -> Geocoder {
        let (mut keys, mut by_key, mut key_streets) = (Vec::new(), HashMap::new(), Vec::<Vec<u32>>::new());
        for (sid, (_, key)) in b.streets.iter().enumerate() {
            let k = *by_key.entry(key.clone()).or_insert_with(|| {
                keys.push(key.clone());
                key_streets.push(Vec::new());
                keys.len() - 1
            });
            key_streets[k].push(sid as u32);
        }
        // Each ZIP's town for labels: the bundle's (most shared land); else the first town listing it.
        let mut names: Vec<_> = b.places.iter().collect();
        names.sort();
        let mut zip_place: HashMap<u32, String> = b.zip_places.iter().map(|(z, n)| (*z, title(n))).collect();
        for (name, zips) in names {
            for z in zips {
                zip_place.entry(*z).or_insert_with(|| title(name));
            }
        }
        Geocoder { norm: Normalizer::default(), keys, by_key, key_streets, zip_place }
    }

    fn parse(&self, b: &Bundle, q: &str) -> Parsed {
        let mut parts: Vec<Vec<String>> = q
            .split(',')
            .map(|s| self.norm.normalize(s).split(' ').filter(|t| !t.is_empty()).map(String::from).collect())
            .filter(|v: &Vec<String>| !v.is_empty())
            .collect();
        let mut zip = None;
        let n_parts = parts.len();
        if let Some(last) = parts.last_mut() {
            let n = last.len();
            if n >= 2 && digits(&last[n - 1], 4) && digits(&last[n - 2], 5) {
                last.pop(); // ZIP+4
            }
            if last.last().is_some_and(|t| digits(t, 5)) {
                zip = last.pop().and_then(|t| t.parse().ok());
            }
            if last.last().is_some_and(|t| t == "MS" || t == "MISSISSIPPI") && (last.len() > 1 || n_parts > 1) {
                last.pop();
            }
        }
        parts.retain(|p| !p.is_empty());
        let Some(first) = parts.first_mut() else {
            return Parsed { number: None, street: String::new(), city: None, zip };
        };
        let lead: String = first[0].chars().take_while(|c| c.is_ascii_digit()).collect();
        let number = if !lead.is_empty() && first.len() > 1 {
            first.remove(0);
            lead.parse().ok()
        } else {
            None
        };
        if parts.len() > 1 {
            let city = parts[1..].concat().join(" ");
            return Parsed { number, street: parts[0].join(" "), city: Some(city), zip };
        }
        // One part: "123 N State St Jackson". The longest known street name first, the rest is the city.
        let tokens = &parts[0];
        for k in (1..=tokens.len()).rev() {
            let (street, city) = (tokens[..k].join(" "), tokens[k..].join(" "));
            if self.by_key.contains_key(&street) && (city.is_empty() || b.places.contains_key(&city)) {
                return Parsed { number, street, city: (!city.is_empty()).then_some(city), zip };
            }
        }
        Parsed { number, street: tokens.join(" "), city: None, zip }
    }

    /// Street ids with exactly this normalized name (score 0).
    fn match_exact(&self, s: &str) -> Vec<(u32, u8)> {
        self.by_key.get(s).map_or(Vec::new(), |&k| self.key_streets[k].iter().map(|&sid| (sid, 0)).collect())
    }

    /// Street ids whose name contains the typed one (score 1), or failing that is a typo or two away (2).
    fn match_loose(&self, s: &str) -> Vec<(u32, u8)> {
        let ids = |k: usize, score: u8| self.key_streets[k].iter().map(move |&sid| (sid, score));
        if s.len() < 3 {
            return Vec::new();
        }
        // ponytail: linear scans over ~40k names, a few ms; an n-gram index if a country's names get slow.
        let (pre, suf, mid) = (format!("{s} "), format!(" {s}"), format!(" {s} "));
        let mut hits: Vec<usize> = (0..self.keys.len())
            .filter(|&k| {
                let key = &self.keys[k];
                key.starts_with(&pre) || key.ends_with(&suf) || key.contains(&mid)
            })
            .take(60)
            .collect();
        let score = if hits.is_empty() {
            let max = if s.len() < 8 { 1 } else { 2 };
            hits = (0..self.keys.len()).filter(|&k| edit_distance(&self.keys[k], s) <= max).take(60).collect();
            2
        } else {
            1
        };
        hits.into_iter().flat_map(|k| ids(k, score)).collect()
    }

    /// Street ids for text still being typed (score 1): names starting with it, or with a word starting
    /// with it ("STATE" finds "N STATE ST"). Failing that, the last word is likely half a word the
    /// normalizer would have shortened ("HIGH STR" before "STREET" becomes "ST"), so it is dropped.
    fn match_prefix(&self, s: &str) -> Vec<(u32, u8)> {
        if s.len() < 2 {
            return Vec::new();
        }
        let word = format!(" {s}");
        let hits: Vec<usize> =
            (0..self.keys.len()).filter(|&k| self.keys[k].starts_with(s) || self.keys[k].contains(&word)).take(60).collect();
        let ids = |hits: Vec<usize>| hits.into_iter().flat_map(|k| self.key_streets[k].iter().map(|&sid| (sid, 1))).collect();
        let Some((head, last)) = s.rsplit_once(' ').filter(|_| hits.is_empty()) else { return ids(hits) };
        // "STR" could be STREET (ST), "BLV" BOULEVARD (BLVD): try each word it could finish as, shortened.
        let mut finished: Vec<(u32, u8)> = Vec::new();
        for (full, short) in &self.norm.tokens {
            if full.starts_with(last) {
                for m in self.match_prefix(&format!("{head} {short}")) {
                    if !finished.contains(&m) {
                        finished.push(m);
                    }
                }
            }
        }
        if finished.is_empty() { self.match_prefix(head) } else { finished }
    }

    /// Matches for a whole address, best first.
    pub fn search(&self, b: &Bundle, query: &str, limit: usize) -> Vec<Location> {
        self.search_with(b, query, limit, false)
    }

    /// Matches for an address still being typed: the last word and the town may be unfinished.
    pub fn suggest(&self, b: &Bundle, query: &str, limit: usize) -> Vec<Location> {
        self.search_with(b, query, limit, true)
    }

    fn search_with(&self, b: &Bundle, query: &str, limit: usize, partial: bool) -> Vec<Location> {
        let p = self.parse(b, query);
        if p.street.is_empty() {
            return Vec::new();
        }
        let town = |c: &String| {
            b.places.get(c).cloned().or_else(|| {
                // A town still being typed ("JACK"): every town it could become, in name order.
                let mut names: Vec<_> = b.places.keys().filter(|k| partial && c.len() >= 2 && k.starts_with(c.as_str())).collect();
                names.sort();
                let zs: Vec<u32> = names.into_iter().flat_map(|k| b.places[k].iter().copied()).collect();
                (!zs.is_empty()).then_some(zs)
            })
        };
        let zips = p.zip.map(|z| vec![z]).or_else(|| p.city.as_ref().and_then(town));
        // The exact name in the city, then a looser name in the city ("Main St" finds "N Main St"), then
        // the exact name anywhere, then a looser one anywhere.
        let exact = self.match_exact(&p.street);
        let loose = if partial { self.match_prefix(&p.street) } else { self.match_loose(&p.street) };
        let mut found = Vec::new();
        for (streets, z) in [(&exact, zips.as_deref()), (&loose, zips.as_deref()), (&exact, None), (&loose, None)] {
            if found.is_empty() {
                found = self.find(b, &p, streets, z);
            }
        }
        found.sort_by(|a, b| (a.0, a.1, a.2).cmp(&(b.0, b.1, b.2)).then_with(|| a.4.label.cmp(&b.4.label)));
        // One answer per place: OSM and TIGER often hold the same address under "Drive"/"Dr" or two ZIPs.
        // Keep the best of any that share a street name and lie within ~400 m of each other.
        let mut kept: Vec<(String, f64, f64)> = Vec::new();
        found.retain(|(_, _, _, key, loc)| {
            let dup = kept.iter().any(|(k, la, lo)| k == key && (la - loc.lat).abs() < 0.004 && (lo - loc.lon).abs() < 0.004);
            if !dup {
                kept.push((key.clone(), loc.lat, loc.lon));
            }
            !dup
        });
        found.into_iter().take(limit).map(|(_, _, _, _, loc)| loc).collect()
    }

    /// (street score, ZIP preference, quality rank, street key, location) for every street and ZIP that holds
    /// the address. The preference is the ZIP's place in the typed town's list, strongest first (0 with no town).
    fn find(&self, b: &Bundle, p: &Parsed, streets: &[(u32, u8)], zips: Option<&[u32]>) -> Vec<(u8, usize, u8, String, Location)> {
        let zip_ok = |z: u32| zips.is_none_or(|zs| zs.contains(&z));
        let mut out = Vec::new();
        let mut push = |score: u8, rank: u8, sid: u32, zip: u32, (edge, t): (u32, f32)| {
            let (lat, lon) = b.point(edge, t);
            let place = p
                .city
                .as_ref()
                .filter(|c| b.places.get(*c).is_some_and(|zs| zs.contains(&zip)))
                .map(|c| title(c))
                .or_else(|| self.zip_place.get(&zip).cloned())
                .map_or(String::new(), |c| format!("{c}, "));
            let num = p.number.map_or(String::new(), |n| format!("{n} "));
            let label = format!("{num}{}, {place}MS {zip:05}", b.streets[sid as usize].0);
            let quality = ["exact", "interpolated", "approximate"][rank as usize].to_string();
            let pref = zips.and_then(|zs| zs.iter().position(|&z| z == zip)).unwrap_or(0);
            out.push((score, pref, rank, b.streets[sid as usize].1.clone(), Location { label, edge, t, lat, lon, quality }));
        };
        for &(sid, score) in streets {
            let rngs = equal_range(&b.rng_street, sid);
            let Some(n) = p.number else {
                // No number: the street once per town (the middle of its first range in each ZIP), so
                // "Main St" lists the Main Streets to pick from.
                let mut zips_seen: Vec<u32> = Vec::new();
                for r in rngs.clone().filter(|&r| zip_ok(b.rng_zip[r])) {
                    if !zips_seen.contains(&b.rng_zip[r]) && zips_seen.len() < 8 {
                        zips_seen.push(b.rng_zip[r]);
                        push(score, 2, sid, b.rng_zip[r], locate(b, b.rng_piece[r], b.rng_npieces[r], 0.5));
                    }
                }
                continue;
            };
            let mut hit: Vec<u32> = Vec::new(); // ZIPs already answered for this street
            for i in equal_range(&b.pt_street, sid) {
                if b.pt_number[i] == n && zip_ok(b.pt_zip[i]) {
                    push(score, 0, sid, b.pt_zip[i], (b.pt_edge[i], b.pt_t[i]));
                    hit.push(b.pt_zip[i]);
                }
            }
            let mut nearest: Option<(u32, usize)> = None;
            for r in rngs {
                let (from, to, z) = (b.rng_from[r], b.rng_to[r], b.rng_zip[r]);
                if !zip_ok(z) {
                    continue;
                }
                let (lo, hi) = (from.min(to), from.max(to));
                let parity_ok = match b.rng_parity[r] {
                    1 => n % 2 == 1,
                    2 => n % 2 == 0,
                    _ => true,
                };
                if (lo..=hi).contains(&n) && parity_ok {
                    if !hit.contains(&z) {
                        push(score, 1, sid, z, locate(b, b.rng_piece[r], b.rng_npieces[r], house_fraction(n, from, to)));
                        hit.push(z);
                    }
                } else {
                    let d = if n < lo { lo - n } else { n.saturating_sub(hi) };
                    if nearest.is_none_or(|(best, _)| d < best) {
                        nearest = Some((d, r));
                    }
                }
            }
            if let (true, Some((_, r))) = (hit.is_empty(), nearest) {
                let (from, to) = (b.rng_from[r], b.rng_to[r]);
                let f = house_fraction(n.clamp(from.min(to), from.max(to)), from, to);
                push(score, 2, sid, b.rng_zip[r], locate(b, b.rng_piece[r], b.rng_npieces[r], f));
            }
        }
        out
    }
}
