# Live smoke test: deepscry.net — 2026-05-28_#2332(b03e22cc)

2-client Playwright smoke test of the production deploy at https://deepscry.net,
run from `web/smoke_test_live.js`. Reproduce with:

```
DEEPSCRY_BASE_URL=https://deepscry.net node web/smoke_test_live.js
```

Screenshots are written to `web/screenshots/live_smoke/` (gitignored).
Findings JSON: `web/screenshots/live_smoke_findings.json`.

## Verdict: NOT smoke-test-green.

Two blocking production regressions found. The end-to-end lobby->game
flow does NOT work for a real user on production right now.

## Pass/fail per scenario

| Scenario | Result | Notes |
|---|---|---|
| Static landing page reachable | PASS | `GET /` -> 200 |
| Per-set bin index reachable | PASS | `GET /data/sets/index.json` -> 200 |
| Per-set bins fetched by browser | PASS | alice's tab downloaded 5 `/data/sets/*.bin` files, all 2xx |
| No `cards.bin` 404 (slim-binary smoke) | PASS | no `cards.bin` requests observed |
| Lobby WS connect (alice) | FAIL (blocking) | `wss://deepscry.net:8080/` -> `ERR_SSL_PROTOCOL_ERROR` |
| Lobby WS connect (bob) | FAIL (blocking) | same |
| alice create-game redirect | PASS | URL params `lobby_create` + `lobby_pass` present |
| bob sees alice's game | FAIL (blocking) | bob's lobby never connects, so list is empty |
| bob join flow | FAIL (blocking) | gated by the visibility failure |
| tui_game.html WASM init | FAIL (blocking) | `TypeError: cardDb.load_set is not a function` |
| Image gate — default hides Local | PASS | tui_game + native_game both omit `img-src-local-label` |
| Image gate — `?allow_local_img_load=true` shows Local | PASS | both pages |

## Root causes

### mtg-478 — lobby WS URL is wrong in `server-config.js`

`scripts/deploy-cloud.sh` writes:

```js
window.MTG_WS_URL = "wss://deepscry.net:8080";
```

Port 8080 is plain TCP on the origin (no TLS termination). Cloudflare
fronts 443; the working WS endpoint is `wss://deepscry.net/lobby` (same
origin, path-based, CF-fronted). Curl-with-Upgrade-headers to
`https://deepscry.net/lobby` returns 400 (handshake reached), confirming
the path-based endpoint is live. The deploy script needs to emit
`wss://${PUBLIC_HOST}/lobby` for the unified-axum deploy (or leave
`window.MTG_WS_URL` unset and let the committed default's same-origin
auto-detect run).

### mtg-479 — `cardDb.load_set is not a function` in tui_game.html

After the redirect to `tui_game.html?lobby_create=...`, WASM init fails:

```
Failed to launch TUI: TypeError: cardDb.load_set is not a function
  at loadSetFiles (tui_game.html:1349)
```

Per-set bin HTTP fetches succeed; the JS->WASM call drives a method that
the deployed wasm-bindgen module does not expose. Likely a rename in the
per-set-bins WASM API that wasn't reflected in `tui_game.html`.

## Other observations

- No CSP violations or unexpected 404s in the network log (modulo the
  expected scryfall/gatherer CDN image 404s which the smoke test
  filters out).
- The image-source gate works exactly as intended on both game pages
  (matches feat commit `2c551ea3`).
- Per-set bin loading at the HTTP layer is fine — the slim binary's
  data-serving routes are healthy.

## Test artifacts

- `web/smoke_test_live.js` — the smoke harness (committed).
- `web/screenshots/live_smoke/*.png` — 8 screenshots (gitignored).
- `web/screenshots/live_smoke_findings.json` — machine-readable
  findings (gitignored).

## Filed beads issues

- `mtg-478` (p2 bug): deploy-cloud.sh emits broken `wss://host:8080`
- `mtg-479` (p2 bug): tui_game.html calls missing `cardDb.load_set`
