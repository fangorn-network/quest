//! cargo run --release --example cli -- "<from address>" ["<to address>"]
//! One address: its matches. Two: the route between their best matches. QUEST_BUNDLE overrides the path;
//! QUEST_SVG=<file> also writes the route's map there. QUEST_PLACES=1: one places query instead.
use std::io::Read;

fn main() {
    let path = std::env::var("QUEST_BUNDLE").unwrap_or("../.ship/bundle/us-ms.bin.gz".into());
    let mut raw = Vec::new();
    flate2::read::GzDecoder::new(std::fs::File::open(&path).expect("bundle")).read_to_end(&mut raw).unwrap();
    let t = std::time::Instant::now();
    let q = quest_core::Quest::load(&raw).unwrap();
    eprintln!("loaded {path} in {:?}", t.elapsed());
    let args: Vec<String> = std::env::args().skip(1).collect();
    if std::env::var("QUEST_PLACES").is_ok() {
        // QUEST_PLACES=1 cargo run --release --example cli -- "gas stations near Hattiesburg"
        match q.places(&args[0], None, 20) {
            Ok(out) => {
                println!("{} near {} ({:.4}, {:.4}):", out.what, out.near.label, out.near.lat, out.near.lon);
                for (i, l) in out.results.iter().enumerate() {
                    println!("{:>3}. {} [{}] {:.1} km ({:.5}, {:.5})", i + 1, l.label, l.kind.as_deref().unwrap_or(""), l.distance_m.unwrap_or(0.0) / 1000.0, l.lat, l.lon);
                }
            }
            Err(e) => println!("error: {e}"),
        }
        return;
    }
    let found: Vec<_> = args.iter().map(|a| q.geocode(a, 5)).collect();
    for (a, f) in args.iter().zip(&found) {
        eprintln!("{a:?}:");
        for l in f {
            eprintln!("  {} [{}] edge {} t {:.2} ({:.5}, {:.5})", l.label, l.quality, l.edge, l.t, l.lat, l.lon);
        }
    }
    if let [Some(a), Some(b)] = [found.first().and_then(|f| f.first()), found.get(1).and_then(|f| f.first())] {
        let t = std::time::Instant::now();
        let r = q.route(a, b).expect("route");
        eprintln!("routed in {:?}", t.elapsed());
        println!("{} -> {}: {:.1} mi, {:.0} min", r.from, r.to, r.distance_m / 1609.344, r.duration_s / 60.0);
        if let Ok(file) = std::env::var("QUEST_SVG") {
            let req = serde_json::json!({ "fn": "map", "width": 900, "height": 600, "route": r.path,
                                          "from": [a.lat, a.lon], "to": [b.lat, b.lon] });
            let t = std::time::Instant::now();
            let out: serde_json::Value = serde_json::from_str(&q.call(&req.to_string())).unwrap();
            let svg = out["svg"].as_str().unwrap();
            eprintln!("map in {:?}, {} KB", t.elapsed(), svg.len() / 1000);
            std::fs::write(file, svg).unwrap();
        }
        for (i, s) in r.steps.iter().enumerate() {
            println!("{:>3}. {:<60} {:>6.2} mi", i + 1, s.text, s.distance_m / 1609.344);
        }
    }
}
