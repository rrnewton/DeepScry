// Server-config: WebSocket URL of the DeepScry lobby.
//
// In the unified-axum deploy (mtg server-web) the lobby is served on
// the same origin as the static page, at path /lobby. This file just
// derives the right ws:// / wss:// URL from window.location so it works
// equally well under HTTP (local dev / direct IP) and HTTPS (deployed
// behind Cloudflare or with direct TLS).
//
// To override at runtime: set `window.MTG_WS_URL = "ws://example:1234/lobby"`
// BEFORE this script tag, or add `?ws=ws://host:port/lobby` to the page URL
// — the landing page picks up the query-string override.
(function () {
    if (!window.MTG_WS_URL) {
        var proto = (window.location && window.location.protocol === "https:") ? "wss:" : "ws:";
        var host  = (window.location && window.location.host)                   || "localhost:8080";
        window.MTG_WS_URL = proto + "//" + host + "/lobby";
    }
})();
