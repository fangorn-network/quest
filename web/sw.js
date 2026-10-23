// Offline copy of the page and its map data. A bundle's file name holds its sha256, so a cached bundle
// never goes stale: it is served from the cache. Everything else is fetched fresh when online and falls
// back to the cache when not.
const CACHE = "quest-v1";

self.addEventListener("install", () => self.skipWaiting());
self.addEventListener("activate", (e) => e.waitUntil(self.clients.claim()));

self.addEventListener("fetch", (e) => {
  const url = new URL(e.request.url);
  if (e.request.method !== "GET" || url.origin !== location.origin) return;
  e.respondWith((async () => {
    const cache = await caches.open(CACHE);
    if (url.pathname.endsWith(".bin.gz")) {
      const hit = await cache.match(e.request);
      if (hit) return hit;
    }
    try {
      // Revalidate with the server (a 304 when unchanged), so the page, worker and wasm never mix versions.
      const res = await fetch(e.request.mode === "navigate" ? e.request : new Request(e.request, { cache: "no-cache" }));
      if (res.ok) cache.put(e.request, res.clone());
      return res;
    } catch (err) {
      return (await cache.match(e.request)) ?? Promise.reject(err);
    }
  })());
});
