// Server-config: WebSocket URL of the native Rust lobby server.
//
// This file is normally generated/overwritten by scripts/deploy-cloud.sh on
// deploy. The committed version points at localhost so a developer running
// `make wasm-serve` + a local `mtg server` gets a working lobby out of the
// box.
//
// To override at runtime without redeploying, set
//   window.MTG_WS_URL = "ws://example.net:17810"
// before this script tag, OR add `?ws=ws://host:port` to the page URL —
// the landing page picks up the query-string override.
(function () {
    if (!window.MTG_WS_URL) {
        // Default: same host as the page, on the canonical lobby port.
        // Falls back to localhost when opened via file://.
        var host = (window.location && window.location.hostname) || "localhost";
        window.MTG_WS_URL = "ws://" + host + ":17810";
    }
})();
