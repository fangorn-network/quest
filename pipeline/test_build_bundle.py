"""Checks for the builder's pure parts: .venv/bin/python pipeline/test_build_bundle.py"""
from build_bundle import dedupe_places, haversine, house_fraction, locate, normalize, osm_place, pieces_for, simplify

# Typed and stored forms of a name meet in one form.
for a, b in [("North State Street", "N State St"), ("I-55", "I 55"), ("Interstate 55", "I- 55"),
             ("US Hwy 49", "US 49"), ("U.S. Highway 49", "US 49"), ("State Hwy 25", "MS 25"),
             ("Mississippi Highway 25", "MS 25"), ("County Road 112", "Co Rd 112"), ("CR 112", "Co Rd 112"),
             ("O'Neal Rd", "ONeal Road")]:
    assert normalize(a) == normalize(b), (a, normalize(a), b, normalize(b))
assert normalize("North State Street") == "N STATE ST"

# One degree of latitude is about 111.2 km.
assert abs(haversine((32.0, -90.0), (33.0, -90.0)) - 111195) < 10

# House numbers run either way along a line.
assert house_fraction(1850, 1801, 1899) == 0.5
assert house_fraction(1850, 1899, 1801) == 0.5
assert house_fraction(1801, 1801, 1801) == 0.5

# A TIGER line whose first half follows edge 7 forwards and second half edge 9 backwards.
ps = pieces_for([(0.1, 7, 0.2), (0.3, 7, 0.6), (0.7, 9, 0.8), (0.9, 9, 0.4)])
assert [p[0] for p in ps] == [7, 9] and ps[0][1] == 0 and ps[-1][2] == 1 and ps[0][2] == ps[1][1] == 0.5
e, t = locate(ps, 0.2)
assert e == 7 and abs(t - 0.4) < 1e-9
e, t = locate(ps, 0.8)
assert e == 9 and abs(t - 0.6) < 1e-9
assert locate(ps, 0.0)[1] == 0.0 and abs(locate(ps, 1.0)[1] - 0.2) < 1e-9
assert locate(pieces_for([(0.4, 5, 0.9), (0.6, 5, 1.0)]), 1.0) == (5, 1.0)  # extrapolation clamps to the edge
assert locate(pieces_for([(0.5, 3, 0.25)]), 0.9) == (3, 0.25)  # one sample: a fixed point

# A wiggle of under a metre goes; a 50 m bend stays; the ends always stay.
line = [(32.0, -90.0), (32.0, -89.9995), (32.000005, -89.999), (32.0, -89.998)]
assert simplify(line) == [line[0], line[-1]]
bend = [(32.0, -90.0), (32.0005, -89.9995), (32.0, -89.999)]
assert simplify(bend) == bend

# OSM tags become kinds; furniture and nameless things are not places.
assert osm_place({"amenity": "fuel", "name": "Shell"}) == ("amenity=fuel", "gas_station")
assert osm_place({"shop": "bakery", "name": "Sweet Spot"}) == ("shop=bakery", None)  # no kind: found by name
assert osm_place({"amenity": "bench", "name": "Memorial Bench"}) is None
assert osm_place({"amenity": "fuel"}) is None

# One place listed twice is kept once (Flowood's real pairs); different places nearby are not merged.
def place(name, lat, lon, kind="gas_station", street="", src=2, cat="gas station"):
    return {"name": name, "lat": lat, "lon": lon, "kind": kind, "street": street, "src": src, "cat": cat}
def kept(*ps):
    out, _ = dedupe_places([dict(p) for p in ps])
    return sorted(p["name"] for p in out)
m = 1 / 111000  # about a metre of latitude
assert kept(place("Sunoco", 32.3563, -90.0248, street="1012 Hugh Ward Blvd"), place("SUNCO", 32.3563 + 20 * m, -90.0248, src=1)) == ["Sunoco"]
assert kept(place("Texaco", 32.3518, -90.0273, src=1), place("Texaco Flowood", 32.3518 + 30 * m, -90.0273, street="110 Lakeland Heights Blvd")) == ["Texaco Flowood"]
assert kept(place("Marathon", 32.35478, -90.06302, street="105 Jacks Place"), place("Xtra Mile", 32.35484, -90.06216, street="105 Jacks Place")) == ["Marathon"]
assert len(kept(place("Shell", 32.32358, -90.10716, street="2530 Flowood Dr"), place("Sunoco", 32.32347, -90.10716, street="1509 W Northside Dr"))) == 1
assert kept(place("Taco Bell", 32.3, -90.1, "fast_food"), place("Arby's", 32.3 + 10 * m, -90.1, "fast_food")) == ["Arby's", "Taco Bell"]
assert len(kept(place("Dollar General", 32.3, -90.1, "store"), place("Dollar Tree", 32.3 + 20 * m, -90.1, "store"))) == 2
assert len(kept(place("Walmart", 32.3, -90.1, "store", "2530 Jackson Ave W"), place("Walmart Pharmacy", 32.3, -90.1, "pharmacy", "2530 Jackson Ave W"))) == 2
# A stale OSM brand with no address, on top of the station as it is now: one. Opposite corners: two.
assert kept(place("BP", 32.35862, -90.01258, src=1), place("Grants Ferry Marathon", 32.35873, -90.01225, street="691 Grants Ferry Rd")) == ["Grants Ferry Marathon"]
assert len(kept(place("Texaco", 32.35844, -90.01318, street="690 Grants Ferry Rd"), place("Grants Ferry Marathon", 32.35873, -90.01225, street="691 Grants Ferry Rd"))) == 2
# Generic words aside, a typo apart: one place. Different names that only share a generic word: two.
assert kept(place("Thai Zap", 32.33702, -90.06670, "restaurant", src=1), place("Thai Zapp Restaurant", 32.33709, -90.06672, "restaurant", "145 Market St")) == ["Thai Zapp Restaurant"]
assert len(kept(place("Joe's Grill", 32.3, -90.1, "restaurant"), place("Mama's Grill", 32.3 + 30 * m, -90.1, "restaurant"))) == 2
# Spelled two ways, one place: Burgers Blues at 108 Market St, Flowood.
assert kept(place("Burgers Blues & Barbeque", 32.33826, -90.06738, "restaurant", "108 Market Street"),
            place("Burgers Blues Barbecue - Flowood", 32.33828, -90.06737, "restaurant", "108 Market St b", src=1)) == ["Burgers Blues & Barbeque"]
assert len(kept(place("Joe's BBQ", 32.3, -90.1, "restaurant"), place("Bob's Bar-B-Q", 32.3 + 30 * m, -90.1, "restaurant"))) == 2
# A shopping centre's tenants share its address, and stay separate places.
assert kept(place("Five Guys", 32.43389, -90.14833, "restaurant", "1000 Highland Colony Pkwy"),
            place("Local 463", 32.43450, -90.14838, "restaurant", "1000 Highland Colony Pkwy")) == ["Five Guys", "Local 463"]
assert len(kept(place("Waffle House", 34.2576, -88.7034, "restaurant"), place("Waffle House", 34.2760, -88.7034, "restaurant"))) == 2
merged, n = dedupe_places([place("Sunoco", 32.3563, -90.0248, street="1012 Hugh Ward Blvd"), place("SUNCO", 32.3563, -90.0248, src=1)])
assert n == 1 and merged[0]["src"] == 3  # found in both sources

print("ok")
