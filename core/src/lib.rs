//! quest-core: geocoding and routing over a quest bundle, entirely on the device.
//!
//! The same code runs in the browser (wasm32) and, later, in the phone apps. The C ABI at the bottom
//! serves both: JSON in, JSON out. Decompressing the bundle and checking its sha256 are the platform's
//! job (every platform has both built in); this crate takes the raw bytes `pipeline/build_bundle.py` wrote.

pub mod geocode;
mod map;
mod route;

pub use geocode::{Geocoder, Location};
pub use map::{Map, MapRequest};
pub use route::{Graph, Route, Step};

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

pub const ONEWAY: u8 = 1;
pub const ROUNDABOUT: u8 = 2;

#[derive(Deserialize)]
struct ArrayInfo {
    name: String,
    #[serde(rename = "type")]
    ty: String,
    length: usize,
    offset: usize,
}

#[derive(Deserialize)]
struct Header {
    format: u32,
    region: String,
    osm_date: Option<String>,
    tiger_year: u32,
    attribution: String,
    classes: Vec<String>,
    names: Vec<String>,
    streets: Vec<(String, String)>,
    places: HashMap<String, Vec<u32>>,
    #[serde(default)]
    zip_places: HashMap<String, String>,
    arrays: Vec<ArrayInfo>,
}

#[derive(Serialize, Clone)]
pub struct Meta {
    pub region: String,
    pub osm_date: Option<String>,
    pub tiger_year: u32,
    pub attribution: String,
    pub junctions: usize,
    pub edges: usize,
}

/// The bundle's arrays, as written by the builder. Field names match the array names in the header.
pub struct Bundle {
    pub meta: Meta,
    pub classes: Vec<String>,
    pub names: Vec<String>,
    pub streets: Vec<(String, String)>,
    pub places: HashMap<String, Vec<u32>>,
    /// Each ZIP's town for labels: the one sharing the most land with it.
    pub zip_places: HashMap<u32, String>,
    pub node_lat: Vec<i32>,
    pub node_lon: Vec<i32>,
    pub edge_u: Vec<u32>,
    pub edge_v: Vec<u32>,
    pub edge_len: Vec<f32>,
    pub edge_speed: Vec<u8>,
    pub edge_flags: Vec<u8>,
    pub edge_class: Vec<u8>,
    pub edge_name: Vec<u32>,
    pub edge_ref: Vec<u32>,
    pub edge_b0: Vec<u8>,
    pub edge_b1: Vec<u8>,
    pub geo_first: Vec<u32>,
    pub geo_lat: Vec<i32>,
    pub geo_lon: Vec<i32>,
    pub res_from: Vec<u32>,
    pub res_via: Vec<u32>,
    pub res_to: Vec<u32>,
    pub res_kind: Vec<u8>,
    pub rng_street: Vec<u32>,
    pub rng_zip: Vec<u32>,
    pub rng_from: Vec<u32>,
    pub rng_to: Vec<u32>,
    pub rng_parity: Vec<u8>,
    pub rng_piece: Vec<u32>,
    pub rng_npieces: Vec<u8>,
    pub pc_edge: Vec<u32>,
    pub pc_f0: Vec<f32>,
    pub pc_f1: Vec<f32>,
    pub pc_t0: Vec<f32>,
    pub pc_t1: Vec<f32>,
    pub pt_street: Vec<u32>,
    pub pt_zip: Vec<u32>,
    pub pt_number: Vec<u32>,
    pub pt_edge: Vec<u32>,
    pub pt_t: Vec<f32>,
}

trait Le: Sized {
    const TYPE: &'static str;
    const SIZE: usize;
    fn from_le(b: &[u8]) -> Self;
}

macro_rules! le {
    ($t:ty, $name:literal) => {
        impl Le for $t {
            const TYPE: &'static str = $name;
            const SIZE: usize = std::mem::size_of::<$t>();
            fn from_le(b: &[u8]) -> Self {
                <$t>::from_le_bytes(b.try_into().unwrap())
            }
        }
    };
}
le!(i32, "int32");
le!(u32, "uint32");
le!(f32, "float32");
le!(u8, "uint8");

impl Bundle {
    pub fn parse(raw: &[u8]) -> Result<Bundle, String> {
        if raw.len() < 8 || &raw[..4] != b"QST1" {
            return Err("not a quest bundle".into());
        }
        let hlen = u32::from_le_bytes(raw[4..8].try_into().unwrap()) as usize;
        let h: Header = serde_json::from_slice(raw.get(8..8 + hlen).ok_or("truncated header")?)
            .map_err(|e| format!("bad header: {e}"))?;
        if h.format != 1 {
            return Err(format!("bundle format {} is newer than this app", h.format));
        }
        let data = &raw[8 + hlen..];
        fn take<T: Le>(data: &[u8], arrays: &[ArrayInfo], name: &str) -> Result<Vec<T>, String> {
            let a = arrays.iter().find(|a| a.name == name).ok_or(format!("missing array {name}"))?;
            if a.ty != T::TYPE {
                return Err(format!("{name} is {}, expected {}", a.ty, T::TYPE));
            }
            let bytes = data.get(a.offset..a.offset + a.length * T::SIZE).ok_or(format!("{name} truncated"))?;
            Ok(bytes.chunks_exact(T::SIZE).map(T::from_le).collect())
        }
        macro_rules! arr {
            ($n:ident) => {
                take(data, &h.arrays, stringify!($n))?
            };
        }
        let node_lat: Vec<i32> = arr!(node_lat);
        let edge_u: Vec<u32> = arr!(edge_u);
        Ok(Bundle {
            meta: Meta {
                region: h.region,
                osm_date: h.osm_date,
                tiger_year: h.tiger_year,
                attribution: h.attribution,
                junctions: node_lat.len(),
                edges: edge_u.len(),
            },
            classes: h.classes,
            names: h.names,
            streets: h.streets,
            places: h.places,
            zip_places: h.zip_places.into_iter().filter_map(|(z, n)| Some((z.parse().ok()?, n))).collect(),
            node_lat,
            node_lon: arr!(node_lon),
            edge_u,
            edge_v: arr!(edge_v),
            edge_len: arr!(edge_len),
            edge_speed: arr!(edge_speed),
            edge_flags: arr!(edge_flags),
            edge_class: arr!(edge_class),
            edge_name: arr!(edge_name),
            edge_ref: arr!(edge_ref),
            edge_b0: arr!(edge_b0),
            edge_b1: arr!(edge_b1),
            geo_first: arr!(geo_first),
            geo_lat: arr!(geo_lat),
            geo_lon: arr!(geo_lon),
            res_from: arr!(res_from),
            res_via: arr!(res_via),
            res_to: arr!(res_to),
            res_kind: arr!(res_kind),
            rng_street: arr!(rng_street),
            rng_zip: arr!(rng_zip),
            rng_from: arr!(rng_from),
            rng_to: arr!(rng_to),
            rng_parity: arr!(rng_parity),
            rng_piece: arr!(rng_piece),
            rng_npieces: arr!(rng_npieces),
            pc_edge: arr!(pc_edge),
            pc_f0: arr!(pc_f0),
            pc_f1: arr!(pc_f1),
            pc_t0: arr!(pc_t0),
            pc_t1: arr!(pc_t1),
            pt_street: arr!(pt_street),
            pt_zip: arr!(pt_zip),
            pt_number: arr!(pt_number),
            pt_edge: arr!(pt_edge),
            pt_t: arr!(pt_t),
        })
    }

    /// Edge e's road shape from u to v, in degrees. Interior points are stored as steps from the point
    /// before, starting at u.
    pub fn shape(&self, e: u32) -> Vec<(f64, f64)> {
        let (e, u, v) = (e as usize, self.edge_u[e as usize], self.edge_v[e as usize]);
        let (mut la, mut lo) = (self.node_lat[u as usize], self.node_lon[u as usize]);
        let mut pts = vec![(la as f64 * 1e-7, lo as f64 * 1e-7)];
        for k in self.geo_first[e] as usize..self.geo_first[e + 1] as usize {
            la += self.geo_lat[k];
            lo += self.geo_lon[k];
            pts.push((la as f64 * 1e-7, lo as f64 * 1e-7));
        }
        pts.push(self.node(v));
        pts
    }

    /// The part of edge e's shape between fractions lo and hi of its length, in that direction.
    pub fn cut(&self, e: u32, lo: f64, hi: f64) -> Vec<(f64, f64)> {
        let pts = self.shape(e);
        let d = |a: (f64, f64), b: (f64, f64)| ((a.0 - b.0).powi(2) + ((a.1 - b.1) * a.0.to_radians().cos()).powi(2)).sqrt();
        let mut cum = vec![0.0];
        for w in pts.windows(2) {
            cum.push(cum.last().unwrap() + d(w[0], w[1]));
        }
        let total = *cum.last().unwrap();
        let at = |f: f64| {
            let x = f.clamp(0.0, 1.0) * total;
            let k = cum.partition_point(|&c| c < x).clamp(1, pts.len() - 1);
            let seg = cum[k] - cum[k - 1];
            let u = if seg > 0.0 { (x - cum[k - 1]) / seg } else { 0.0 };
            (pts[k - 1].0 + u * (pts[k].0 - pts[k - 1].0), pts[k - 1].1 + u * (pts[k].1 - pts[k - 1].1))
        };
        let (a, b) = (lo.min(hi), lo.max(hi));
        let mut out = vec![at(a)];
        out.extend((1..pts.len() - 1).filter(|&k| cum[k] > a * total && cum[k] < b * total).map(|k| pts[k]));
        out.push(at(b));
        if lo > hi {
            out.reverse();
        }
        out
    }

    /// A point `t` of the way along edge `e`'s shape.
    pub fn point(&self, e: u32, t: f32) -> (f64, f64) {
        self.cut(e, t as f64, t as f64)[0]
    }

    pub fn node(&self, n: u32) -> (f64, f64) {
        (self.node_lat[n as usize] as f64 * 1e-7, self.node_lon[n as usize] as f64 * 1e-7)
    }
}

/// Everything a session needs: the bundle, the routing graph and the address index.
pub struct Quest {
    pub bundle: Bundle,
    pub graph: Graph,
    pub geocoder: Geocoder,
    pub map: Map,
}

impl Quest {
    pub fn load(raw: &[u8]) -> Result<Quest, String> {
        let bundle = Bundle::parse(raw)?;
        let graph = Graph::new(&bundle);
        let geocoder = Geocoder::new(&bundle);
        let map = Map::new(&bundle);
        Ok(Quest { bundle, graph, geocoder, map })
    }

    pub fn geocode(&self, query: &str, limit: usize) -> Vec<Location> {
        self.geocoder.search(&self.bundle, query, limit)
    }

    /// Matches for an address still being typed, for suggestions under a text box.
    pub fn suggest(&self, query: &str, limit: usize) -> Vec<Location> {
        self.geocoder.suggest(&self.bundle, query, limit)
    }

    pub fn route(&self, from: &Location, to: &Location) -> Option<Route> {
        self.graph.route(&self.bundle, from, to)
    }

    /// One JSON request -> one JSON response; the whole ABI surface.
    /// `{"fn":"geocode","q":"...","limit":5}`, `{"fn":"suggest","q":"400 High S","limit":6}`,
    /// `{"fn":"route","from":Location,"to":Location}`, or
    /// `{"fn":"map", ...MapRequest}`.
    pub fn call(&self, request: &str) -> String {
        #[derive(Deserialize)]
        #[serde(tag = "fn", rename_all = "lowercase")]
        enum Req {
            Geocode { q: String, limit: Option<usize> },
            Suggest { q: String, limit: Option<usize> },
            Route { from: Location, to: Location },
            Map(MapRequest),
            Meta,
        }
        let out = match serde_json::from_str::<Req>(request) {
            Err(e) => serde_json::json!({ "error": format!("bad request: {e}") }),
            Ok(Req::Geocode { q, limit }) => serde_json::json!({ "matches": self.geocode(&q, limit.unwrap_or(5).min(20)) }),
            Ok(Req::Suggest { q, limit }) => serde_json::json!({ "matches": self.suggest(&q, limit.unwrap_or(6).min(20)) }),
            Ok(Req::Route { from, to }) => match self.route(&from, &to) {
                Some(r) => serde_json::json!({ "route": r }),
                None => serde_json::json!({ "error": "no drivable route between these points" }),
            },
            Ok(Req::Map(req)) => serde_json::json!(self.map.render(&self.bundle, &req)),
            Ok(Req::Meta) => serde_json::json!({ "meta": self.bundle.meta }),
        };
        out.to_string()
    }
}

// ---- C ABI, used by the browser (wasm32) and later by the phone apps. ----
// Strings cross as (pointer, length) in and as a pointer to [u32 length][bytes] out; the caller frees
// what it receives with quest_free(ptr, length + 4).

use std::cell::RefCell;
thread_local!(static QUEST: RefCell<Option<Quest>> = const { RefCell::new(None) });

fn reply(s: String) -> *mut u8 {
    let mut v = Vec::with_capacity(4 + s.len());
    v.extend_from_slice(&(s.len() as u32).to_le_bytes());
    v.extend_from_slice(s.as_bytes());
    let mut v = std::mem::ManuallyDrop::new(v.into_boxed_slice());
    v.as_mut_ptr()
}

#[no_mangle]
pub extern "C" fn quest_alloc(len: usize) -> *mut u8 {
    std::mem::ManuallyDrop::new(vec![0u8; len].into_boxed_slice()).as_mut_ptr()
}

/// # Safety
/// `ptr` and `len` must come from quest_alloc, or from a reply (with len = its length + 4).
#[no_mangle]
pub unsafe extern "C" fn quest_free(ptr: *mut u8, len: usize) {
    drop(Box::from_raw(std::ptr::slice_from_raw_parts_mut(ptr, len)));
}

/// Load a decompressed bundle. Takes ownership of the buffer. Replies `{"meta":…}` or `{"error":…}`.
/// # Safety
/// `ptr`/`len` must be a buffer from quest_alloc.
#[no_mangle]
pub unsafe extern "C" fn quest_load(ptr: *mut u8, len: usize) -> *mut u8 {
    let raw = Box::from_raw(std::ptr::slice_from_raw_parts_mut(ptr, len));
    reply(match Quest::load(&raw) {
        Ok(q) => {
            let meta = serde_json::json!({ "meta": q.bundle.meta }).to_string();
            QUEST.with(|c| *c.borrow_mut() = Some(q));
            meta
        }
        Err(e) => serde_json::json!({ "error": e }).to_string(),
    })
}

/// Run one JSON request (see Quest::call). Does not take ownership of the request buffer.
/// # Safety
/// `ptr`/`len` must be a valid UTF-8 buffer.
#[no_mangle]
pub unsafe extern "C" fn quest_call(ptr: *const u8, len: usize) -> *mut u8 {
    let req = std::str::from_utf8(std::slice::from_raw_parts(ptr, len)).unwrap_or("");
    reply(QUEST.with(|c| match &*c.borrow() {
        Some(q) => q.call(req),
        None => r#"{"error":"no bundle loaded"}"#.into(),
    }))
}
