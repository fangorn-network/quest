# Goals

One section per kind of person this app serves. Each check in golden.jsonl names one with `goal`.

## directions: a traveler in Mississippi prints a route
- **They ask:** "from 123 Main St, Jackson MS to 400 Hwy 51, Ridgeland MS"; two addresses, step-by-step turns, a page to print.
- **They get:** numbered turns with street names and distances, total miles and time, printable on a page or two, with the OpenStreetMap credit.
- **Coverage:** every drivable road and every addressable street in Mississippi (OSM data).
- **Privacy:** neither address leaves the browser. No geocoding or routing service, and no request that depends on what was typed.
- **Failure looks like:** the wrong address is matched, a turn onto a road that isn't there, or any network request that reveals where someone is going.
