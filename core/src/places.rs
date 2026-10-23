//! Businesses and other places by kind, near somewhere: "gas stations near Hattiesburg", "barbecue in
//! Clarksdale", "Walmart near Oxford". The kinds and the words for them are pipeline/categories.json.

use crate::geocode::town_name;
use crate::{Bundle, Geocoder, Location};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Deserialize)]
struct Kind {
    label: String,
    words: Vec<String>,
}

#[derive(Deserialize)]
struct Categories {
    kinds: HashMap<String, Kind>,
}

/// The bundle's kinds (by its index) with their labels and words.
pub(crate) struct Kinds {
    labels: Vec<String>,
    words: Vec<Vec<String>>,
}

impl Kinds {
    pub(crate) fn new(b: &Bundle) -> Kinds {
        let mut c: Categories = serde_json::from_str(include_str!("../../pipeline/categories.json")).expect("categories.json");
        let (mut labels, mut words) = (Vec::new(), Vec::new());
        for id in &b.kinds {
            let k = c.kinds.remove(id).unwrap_or(Kind { label: id.replace('_', " "), words: vec![] });
            words.push(k.words.iter().chain([&k.label]).map(|w| singular_phrase(w)).collect());
            labels.push(k.label);
        }
        Kinds { labels, words }
    }

    /// What place i is: its kind's label, else its own category.
    pub(crate) fn of(&self, b: &Bundle, i: usize) -> String {
        match b.pl_kind[i] {
            255 => b.place_cats[b.pl_cat[i] as usize].clone(),
            k => self.labels.get(k as usize).cloned().unwrap_or_default(),
        }
    }
}

fn singular(w: &str) -> String {
    let n = w.len();
    if n > 4 && w.ends_with("ies") {
        format!("{}y", &w[..n - 3])
    } else if n > 4 && ["ses", "xes", "ches", "shes"].iter().any(|e| w.ends_with(e)) {
        w[..n - 2].to_string()
    } else if n > 3 && w.ends_with('s') && !w.ends_with("ss") {
        w[..n - 1].to_string()
    } else {
        w.to_string()
    }
}

/// Lowercase words, punctuation dropped, each made singular: "Gas Stations" -> "gas station".
fn singular_phrase(s: &str) -> String {
    let clean: String = s.to_lowercase().chars().map(|c| if c.is_alphanumeric() { c } else { ' ' }).collect();
    clean.split_whitespace().map(singular).collect::<Vec<_>>().join(" ")
}

/// Does place i's category ("thai restaurant") or name ("Thai Zapp") carry the words of `q`?
fn qualified(b: &Bundle, q: &str, i: usize) -> bool {
    has_words(&singular_phrase(&b.place_cats[b.pl_cat[i] as usize]), q)
        || has_words(&singular_phrase(&b.place_names[b.pl_name[i] as usize]), q)
}

/// Is `part` a whole-word piece of `whole`?
fn has_words(whole: &str, part: &str) -> bool {
    format!(" {whole} ").contains(&format!(" {part} "))
}

#[derive(Serialize)]
pub struct Near {
    pub label: String,
    pub lat: f64,
    pub lon: f64,
}

#[derive(Serialize)]
pub struct PlacesOut {
    /// What was searched for, as understood: a kind ("gas station"), a category, or a name.
    pub what: String,
    pub near: Near,
    pub results: Vec<Location>,
}

const CONNECTORS: [&str; 6] = [" near ", " in ", " around ", " close to ", " by ", " at "];
// ponytail: how far past its equal-area radius a town reaches, and the most results for one; tune by eye.
const TOWN_REACH: f64 = 1.6;
const TOWN_MAX: usize = 100;

fn haversine((la1, lo1): (f64, f64), (la2, lo2): (f64, f64)) -> f64 {
    let (p1, p2) = (la1.to_radians(), la2.to_radians());
    let h = ((p2 - p1) / 2.0).sin().powi(2) + p1.cos() * p2.cos() * ((lo2 - lo1).to_radians() / 2.0).sin().powi(2);
    2.0 * 6371008.8 * h.sqrt().asin()
}

/// "<what> near <where>". `near` is used when the query names no place (the page passes its map's centre).
pub fn search(b: &Bundle, g: &Geocoder, query: &str, near: Option<[f64; 2]>, limit: usize) -> Result<PlacesOut, String> {
    let q = format!(" {} ", query.trim());
    let lower = q.to_lowercase();
    // Split at the first connector; failing that, a town at the end ("gas stations hattiesburg").
    let (what, place) = match CONNECTORS.iter().filter_map(|c| lower.find(c).map(|i| (i, c.len()))).min() {
        Some((i, n)) => (q[..i].trim().to_string(), Some(q[i + n..].trim().to_string())),
        None => {
            let words: Vec<&str> = q.split_whitespace().collect();
            (1..words.len())
                .find(|&k| b.towns.contains_key(&g.norm.normalize(&words[k..].join(" "))))
                .map(|k| (words[..k].join(" "), Some(words[k..].join(" "))))
                .unwrap_or((query.trim().to_string(), None))
        }
    };
    let phrase = singular_phrase(&what);
    if phrase.is_empty() {
        return Err("Say what to look for, like “gas stations near Hattiesburg”.".into());
    }

    // Where: a town's centre, else an address or place found by the geocoder, else the given point.
    let mut town: Option<String> = None;
    let near = match place.as_deref().filter(|p| !p.is_empty()) {
        Some(p) => {
            let key = g.norm.normalize(p.trim_end_matches(|c: char| c == ',' || c.is_whitespace()).trim_end_matches(" MS"));
            match b.towns.get(&key) {
                Some(&[lat, lon]) => {
                    town = Some(key.clone());
                    Near { label: town_name(b, &key), lat, lon }
                }
                None => match g.search(b, p, 1).into_iter().next() {
                    Some(loc) => Near { label: loc.label, lat: loc.lat, lon: loc.lon },
                    None => return Err(format!("No town, address or place in Mississippi matches “{p}”.")),
                },
            }
        }
        None => match near {
            Some([lat, lon]) => Near { label: "the map's centre".into(), lat, lon },
            None => return Err("Say where, like “gas stations near Hattiesburg”.".into()),
        },
    };

    // What: kinds whose words are the phrase, else contained in it ("cheap gas"), else containing it
    // ("station"); else categories containing it ("barbecue" -> "barbecue restaurant"); else names.
    let kind_hits = |test: &dyn Fn(&str) -> bool| -> Vec<u8> {
        (0..g.kinds.words_len()).filter(|&k| g.kinds.words_of(k).iter().any(|w| test(w))).map(|k| k as u8).collect()
    };
    let mut kinds = kind_hits(&|w| w == phrase);
    // A kind's word inside the phrase, the rest qualifying it: "thai food" is restaurants that are Thai (by
    // their category, "thai restaurant", or their name). A qualifier nothing matches ("cheap gas") is dropped.
    let mut qualifier = String::new();
    if kinds.is_empty() {
        let words = (0..g.kinds.words_len()).flat_map(|k| g.kinds.words_of(k).iter());
        if let Some(w) = words.filter(|w| has_words(&phrase, w)).max_by_key(|w| w.len()).cloned() {
            kinds = kind_hits(&|x| x == w);
            qualifier = format!(" {phrase} ").replacen(&format!(" {w} "), " ", 1).trim().to_string();
        }
    }
    if !qualifier.is_empty() && !(0..b.pl_name.len()).any(|i| kinds.contains(&b.pl_kind[i]) && qualified(b, &qualifier, i)) {
        qualifier.clear();
    }
    if kinds.is_empty() {
        kinds = kind_hits(&|w| has_words(w, &phrase));
    }
    let cats: Vec<u16> = if kinds.is_empty() {
        (0..b.place_cats.len()).filter(|&c| has_words(&singular_phrase(&b.place_cats[c]), &phrase)).map(|c| c as u16).collect()
    } else {
        Vec::new()
    };
    let by_name = kinds.is_empty() && cats.is_empty();
    let (what, matches): (String, Box<dyn Fn(usize) -> bool>) = if !kinds.is_empty() {
        let label = kinds.iter().map(|&k| g.kinds.label(k)).collect::<Vec<_>>().join(", ");
        if qualifier.is_empty() {
            (label, Box::new(move |i| kinds.contains(&b.pl_kind[i])))
        } else {
            let what = format!("{qualifier} {label}");
            (what, Box::new(move |i| kinds.contains(&b.pl_kind[i]) && qualified(b, &qualifier, i)))
        }
    } else if !cats.is_empty() {
        (phrase.clone(), Box::new(move |i| cats.contains(&b.pl_cat[i])))
    } else {
        let key = g.norm.normalize(&what);
        let mut ids: Vec<u32> = g.places.exact(&key).into_iter().chain(g.places.loose(&key)).map(|(i, _)| i).collect();
        ids.sort_unstable();
        if ids.is_empty() {
            return Err(format!("Nothing in Mississippi is called or listed as “{what}”."));
        }
        (what.clone(), Box::new(move |i| ids.binary_search(&(i as u32)).is_ok()))
    };

    // Nearest first: within 25 km, widening to 60 km if that finds fewer than 5, else the nearest anywhere.
    let from = (near.lat, near.lon);
    let mut hits: Vec<(f64, usize)> = (0..b.pl_name.len())
        .filter(|&i| matches(i))
        .map(|i| (haversine(from, (b.pl_lat[i] as f64 * 1e-7, b.pl_lon[i] as f64 * 1e-7)), i))
        .collect();
    // Nearest first; within 50 m of each other the shortest name first, the business rather than a kiosk in it.
    let name_len = |i: usize| b.place_names[b.pl_name[i] as usize].len();
    hits.sort_by(|x, y| ((x.0 / 50.0) as u64, name_len(x.1)).cmp(&((y.0 / 50.0) as u64, name_len(y.1))).then(x.0.total_cmp(&y.0)));
    if by_name {
        // One per address: a store's departments (bakery, photo centre) are listed as places of their own.
        let mut seen = std::collections::HashSet::new();
        hits.retain(|&(_, i)| b.pl_street[i] == 0 || seen.insert((b.pl_street[i], b.pl_zip[i])));
    }
    // In a town: everything in it, not just the few nearest its centre point (Flowood runs 7 km east of its
    // centre). In it means within its reach (its land area's radius, generously: towns are rarely round) or
    // carrying its main ZIP code. Elsewhere, or when that finds too few: the nearest within 25 km, widening
    // to 60 km if that finds fewer than 5, else the nearest anywhere.
    let in_town: Vec<(f64, usize)> = town.as_ref().map_or(Vec::new(), |key| {
        let reach = (b.town_radius.get(key).copied().unwrap_or(0.0) * TOWN_REACH).max(3000.0);
        let main_zip = b.places.get(key).and_then(|zs| zs.first().copied());
        hits.iter().copied().filter(|&(d, i)| d <= reach || (d <= 25_000.0 && Some(b.pl_zip[i]) == main_zip)).collect()
    });
    let within = |km: f64| hits.iter().take_while(|h| h.0 <= km * 1000.0).count();
    let chosen: Vec<(f64, usize)> = if in_town.len() >= 5 {
        in_town.into_iter().take(TOWN_MAX).collect()
    } else {
        let n = if within(25.0) >= 5 { within(25.0) } else if within(60.0) > 0 { within(60.0) } else { hits.len() };
        hits.into_iter().take(n.min(limit)).collect()
    };
    let results = chosen
        .into_iter()
        .map(|(d, i)| Location { distance_m: Some(d.round()), ..g.place_location(b, i, None) })
        .collect();
    Ok(PlacesOut { what, near, results })
}

impl Kinds {
    fn words_len(&self) -> usize {
        self.words.len()
    }
    fn words_of(&self, k: usize) -> &[String] {
        &self.words[k]
    }
    fn label(&self, k: u8) -> String {
        self.labels[k as usize].clone()
    }
}
