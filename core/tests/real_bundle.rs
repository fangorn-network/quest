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

#[test]
fn a_town_prefers_its_own_zip() {
    let Some(q) = quest() else { return eprintln!("no bundle, skipped") };
    // Pontotoc is ZIP 38863; stray OSM addresses tagged "Pontotoc" with 38663 (Ripley) must not come first.
    let top = &q.geocode("100 N Main St, Pontotoc MS", 5)[0];
    assert!(top.label.ends_with("38863"), "{}", top.label);
}

#[test]
fn suggests_while_typing() {
    let Some(q) = quest() else { return eprintln!("no bundle, skipped") };
    let labels = |s: &str| q.suggest(s, 6).into_iter().map(|l| l.label).collect::<Vec<_>>();
    // A half-typed town, or a whole one, narrows to 400 High St, Jackson.
    for typed in ["400 High St, Jack", "400 high st jackson", "400 High S, Jackson"] {
        assert!(labels(typed).iter().any(|l| l == "400 High St, Jackson, MS 39201"), "{typed}: {:?}", labels(typed));
    }
    // A half-typed suffix ("Str" before it becomes "St") still means High St, in whichever towns have one.
    let high = labels("400 High Str");
    assert!(!high.is_empty() && high.iter().all(|l| l.starts_with("400 High St,")), "{high:?}");
    // No number yet: the street, once per town.
    let mains = labels("Main St");
    assert!(mains.len() > 3 && mains.iter().all(|l| l.starts_with("Main St")), "{mains:?}");
    // Too little to go on, or nothing like it: no suggestions, not an error.
    assert!(labels("4").is_empty() && labels("zzzz qqqq").is_empty());
    // With no town typed, a ZIP is named for its main town: 38801 is Tupelo, though Plantersville shares it.
    assert_eq!(labels("306 Elvis Pres")[0], "306 Elvis Presley Drive, Tupelo, MS 38801");
    // A whole address searched normally is unchanged by suggest mode.
    assert_eq!(q.geocode("400 High St, Jackson MS 39201", 1)[0].label, "400 High St, Jackson, MS 39201");
}

#[test]
fn labels_roads() {
    let Some(q) = quest() else { return eprintln!("no bundle, skipped") };
    let svg = |req: serde_json::Value| -> String {
        let out: serde_json::Value = serde_json::from_str(&q.call(&req.to_string())).unwrap();
        out["svg"].as_str().unwrap().to_string()
    };
    // The whole state: interstate shields, no street names.
    let state = svg(serde_json::json!({ "fn": "map", "width": 800, "height": 600 }));
    assert!(state.contains("m-shield m-i") && state.contains(">I-55<"), "no I-55 shield");
    assert!(!state.contains("m-label"), "street names at state scale");
    // Downtown Jackson: street names, and the capitol's High Street among them.
    let town = svg(serde_json::json!({ "fn": "map", "width": 800, "height": 600, "bbox": [32.295, -90.195, 32.31, -90.17] }));
    assert!(town.contains("m-label") && town.contains(">High Street<"), "no street names downtown");
    // Every label is well-formed text: nothing from OSM's names escapes into the markup.
    assert_eq!(town.matches("<text").count(), town.matches("</text>").count());
}

#[test]
fn finds_businesses_by_name() {
    let Some(q) = quest() else { return eprintln!("no bundle, skipped") };
    let top = |s: &str| q.geocode(s, 5).into_iter().next().expect(s);
    let wh = top("Waffle House, Tupelo");
    assert!(wh.label.starts_with("Waffle House") && wh.label.contains("Tupelo") && wh.kind.is_some(), "{wh:?}");
    let abes = top("Abe's Bar-B-Q");
    assert!(abes.label.contains("Clarksdale"), "{}", abes.label);
    assert!(top("Waffle House Tupelo").label.contains("Tupelo"), "the town may end the text, no comma");
    // As typed, a business comes up alongside streets.
    assert!(q.suggest("Elvis Presley Birth", 6).iter().any(|l| l.label.starts_with("Elvis Presley Birthplace")));
    // Addresses are unchanged.
    assert_eq!(top("400 High St, Jackson MS 39201").label, "400 High St, Jackson, MS 39201");
    // A business found by name routes like an address.
    assert!(q.route(&top("400 High St, Jackson MS 39201"), &abes).is_some());
}

#[test]
fn finds_places_near_somewhere() {
    let Some(q) = quest() else { return eprintln!("no bundle, skipped") };
    let gas = q.places("Gas stations near Hattiesburg", None, 20).unwrap();
    assert!(gas.what.contains("gas station") && gas.near.label == "Hattiesburg", "{} / {}", gas.what, gas.near.label);
    assert!(gas.results.len() >= 5 && gas.results.iter().all(|l| l.kind.as_deref() == Some("gas station")));
    let d: Vec<f64> = gas.results.iter().map(|l| l.distance_m.unwrap()).collect();
    assert!(d.windows(2).all(|w| w[0] <= w[1] + 50.0) && d[0] < 10_000.0, "nearest first (to 50 m): {d:?}");
    // A town means all of it: Flowood's east side (Highway 25, Grants Ferry Rd) is 6-7 km from its centre point.
    let flowood: Vec<String> = q.places("gas station flowood", None, 20).unwrap().results.into_iter().map(|l| l.label).collect();
    for addr in ["5657 Highway 25", "690 Grants Ferry Rd", "691 Grants Ferry Rd"] {
        assert!(flowood.iter().any(|l| l.contains(addr)), "{addr} missing from {} Flowood results", flowood.len());
    }
    // A kind word with a qualifier: "Thai food" is Thai restaurants, not every restaurant; Thai Zap (OSM) and
    // Thai Zapp Restaurant (Overture), 2 m apart, are one place.
    let thai = q.places("Thai food in flowood", None, 20).unwrap();
    assert_eq!(thai.what, "thai restaurant");
    // Only Thai places (by name, or by category like Tuk Tuk Boom), the nearest first: not Flowood's other 90+ restaurants.
    assert!(thai.results.len() >= 3 && thai.results.len() < 20, "{} results", thai.results.len());
    assert!(thai.results[0].label.starts_with("Thai Zapp"), "{}", thai.results[0].label);
    assert_eq!(thai.results.iter().filter(|l| l.label.to_lowercase().starts_with("thai zap")).count(), 1, "Thai Zap listed twice");
    // A qualifier nothing matches is dropped: "cheap gas" is gas stations.
    assert_eq!(q.places("cheap gas near Hattiesburg", None, 5).unwrap().what, "gas station");
    // A category that is no kind of ours, and a name.
    assert!(q.places("barbecue in Clarksdale", None, 20).unwrap().results.iter().any(|l| l.label.starts_with("Abe's")));
    let walmart = q.places("Walmart near Oxford", None, 10).unwrap();
    assert!(!walmart.results.is_empty() && walmart.results.iter().all(|l| l.label.to_lowercase().contains("walmart")));
    assert!(walmart.results.iter().all(|l| l.kind.as_deref() != Some("money transfer service")), "counters inside stores");
    // Towns are named as written, not as the normalized key ("Holly Spgs").
    assert!(walmart.results.iter().all(|l| !l.label.contains("Spgs")), "{:?}", walmart.results.iter().map(|l| &l.label).collect::<Vec<_>>());
    // No town connector: a town at the end still counts; no place at all uses the given point.
    assert_eq!(q.places("pharmacies tupelo", None, 5).unwrap().near.label, "Tupelo");
    assert!(q.places("coffee", Some([32.30, -90.18]), 5).unwrap().results.len() == 5);
    // Nothing to go on is an error, not a crash.
    assert!(q.places("coffee", None, 5).is_err());
    assert!(q.places("zzzz qqqq near Jackson", None, 5).is_err());
    assert!(q.places("gas near Qzxvwq", None, 5).is_err());
    // By name, one result per address: Overture lists a Walmart's bakery and photo centre as places too.
    let mut addrs: Vec<String> = walmart.results.iter().map(|l| l.label.split_once(", ").map_or(String::new(), |(_, a)| a.to_string())).collect();
    let n = addrs.len();
    addrs.sort();
    addrs.dedup();
    assert_eq!(addrs.len(), n, "one Walmart per address");
}
