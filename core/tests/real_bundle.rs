//! cargo test --release   (needs ../.ship/bundle/us-ms.bin.gz from pipeline/build_bundle.py; skips without it)
use quest_core::{geocode::Normalizer, Quest};
use std::io::Read;

fn quest() -> Option<Quest> {
    let f = std::fs::File::open("../.ship/bundle/us-ms.bin.gz").ok()?;
    let mut raw = Vec::new();
    flate2::read::GzDecoder::new(f).read_to_end(&mut raw).unwrap();
    Some(Quest::load(&raw).unwrap())
}

#[test]
fn normalizes_like_the_builder() {
    let n = Normalizer::default();
    // The same pairs as pipeline/test_build_bundle.py.
    for (a, b) in [("North State Street", "N State St"), ("I-55", "I 55"), ("Interstate 55", "I- 55"),
                   ("US Hwy 49", "US 49"), ("U.S. Highway 49", "US 49"), ("State Hwy 25", "MS 25"),
                   ("Mississippi Highway 25", "MS 25"), ("County Road 112", "Co Rd 112"), ("CR 112", "Co Rd 112"),
                   ("O'Neal Rd", "ONeal Road")] {
        assert_eq!(n.normalize(a), n.normalize(b), "{a} / {b}");
    }
    assert_eq!(n.normalize("North State Street"), "N STATE ST");
}

#[test]
fn finds_addresses() {
    let Some(q) = quest() else { return eprintln!("no bundle, skipped") };
    let top = |s: &str| q.geocode(s, 5).into_iter().next().map(|l| l.label).unwrap_or_default();
    assert_eq!(top("400 High St, Jackson MS 39201"), "400 High St, Jackson, MS 39201");
    assert_eq!(top("400 high street jackson"), "400 High St, Jackson, MS 39201");
    assert!(top("100 Main St, Hattiesburg MS").contains("Hattiesburg"), "city narrows, even to N Main St");
    assert!(top("400 Hihg St, Jackson").starts_with("400 High St"), "one typo");
    assert!(q.geocode("306 Elvis Presley Dr, Tupelo", 5)[0].quality == "exact");
    assert!(q.geocode("zzzz qqqq", 5).is_empty());
}

#[test]
fn routes_jackson_to_tupelo() {
    let Some(q) = quest() else { return eprintln!("no bundle, skipped") };
    let a = &q.geocode("400 High St, Jackson MS 39201", 1)[0];
    let b = &q.geocode("306 Elvis Presley Dr, Tupelo, MS 38801", 1)[0];
    let r = q.route(a, b).expect("route");
    let miles = r.distance_m / 1609.344;
    assert!((170.0..220.0).contains(&miles), "{miles} mi");
    assert!(r.steps.iter().any(|s| s.text.contains("I 55")));
    assert_eq!(r.steps.last().unwrap().text, "Arrive at your destination");
    let sum: f64 = r.steps.iter().map(|s| s.distance_m).sum();
    assert!((sum - r.distance_m).abs() < 1.0, "step distances add up to the total");
    // Same point: a route of nothing, not a failure.
    assert!(q.route(a, a).expect("same point").distance_m < 1.0);
}

#[test]
fn json_abi() {
    let Some(q) = quest() else { return eprintln!("no bundle, skipped") };
    let out: serde_json::Value = serde_json::from_str(&q.call(r#"{"fn":"geocode","q":"400 High St, Jackson"}"#)).unwrap();
    let loc = &out["matches"][0];
    let req = serde_json::json!({ "fn": "route", "from": loc, "to": loc }).to_string();
    assert!(serde_json::from_str::<serde_json::Value>(&q.call(&req)).unwrap()["route"]["steps"].is_array());
    assert!(q.call("nonsense").contains("error"));
}

#[test]
fn draws_the_route_map() {
    let Some(q) = quest() else { return eprintln!("no bundle, skipped") };
    let a = &q.geocode("400 High St, Jackson MS 39201", 1)[0];
    let b = &q.geocode("306 Elvis Presley Dr, Tupelo, MS 38801", 1)[0];
    let r = q.route(a, b).unwrap();
    // The path starts and ends at the two addresses.
    let near = |p: [f64; 2], la: f64, lo: f64| (p[0] - la).abs() < 1e-4 && (p[1] - lo).abs() < 1e-4;
    assert!(near(r.path[0], a.lat, a.lon) && near(*r.path.last().unwrap(), b.lat, b.lon));
    let req = serde_json::json!({ "fn": "map", "width": 800, "height": 500, "route": r.path,
                                  "from": [a.lat, a.lon], "to": [b.lat, b.lon] });
    let out: serde_json::Value = serde_json::from_str(&q.call(&req.to_string())).unwrap();
    let svg = out["svg"].as_str().unwrap();
    assert!(svg.starts_with("<svg") && svg.ends_with("</svg>"));
    for class in ["m-major", "m-route", "m-start", "m-end", "m-scale"] {
        assert!(svg.contains(class), "{class} missing");
    }
    // Zoomed out over the whole state, no residential streets; zoomed into downtown Jackson, they appear.
    let state: serde_json::Value = serde_json::from_str(&q.call(r#"{"fn":"map","width":800,"height":600}"#)).unwrap();
    assert!(!state["svg"].as_str().unwrap().contains("m-minor"));
    let town = serde_json::json!({ "fn": "map", "width": 800, "height": 600, "bbox": [32.29, -90.19, 32.31, -90.17] });
    let town: serde_json::Value = serde_json::from_str(&q.call(&town.to_string())).unwrap();
    assert!(town["svg"].as_str().unwrap().contains("m-minor"));
}

#[test]
fn one_answer_per_place() {
    let Some(q) = quest() else { return eprintln!("no bundle, skipped") };
    // OSM has "306 Elvis Presley Drive, 38801", TIGER "306 Elvis Presley Dr, 38804": one place.
    let found = q.geocode("306 Elvis Presley Dr, Tupelo", 5);
    assert_eq!(found.len(), 1, "{:?}", found.iter().map(|l| &l.label).collect::<Vec<_>>());
    assert_eq!(found[0].quality, "exact");
}
