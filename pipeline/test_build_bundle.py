"""Checks for the builder's pure parts: .venv/bin/python pipeline/test_build_bundle.py"""
from build_bundle import haversine, house_fraction, locate, normalize, pieces_for, simplify

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

print("ok")
