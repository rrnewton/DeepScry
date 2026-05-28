# Landing Page + Lobby UX — Playwright QA Report

Stamp: `2026-05-27_#2303(d8b2448f)`
Branch under test: `playwright-qa-landing-page` (= `landing-page-lobby` HEAD `d8b2448f`)
Mode: local (static `python3 -m http.server 8080` from `web/`; native `mtg server --port 17810`)
Driver: `web/test_landing_page_ux.js` (Playwright/Chromium, headless)

## Executive Summary

| Scenario                                                        | Result   |
|-----------------------------------------------------------------|----------|
| Landing page renders, connects to lobby WebSocket               | PASS     |
| Username entry validates and reveals lobby                      | PASS     |
| Username uniqueness enforcement (taken-name protection)         | **FAIL** |
| Create-game form validates required name                        | PASS     |
| Create-game with passcode — game becomes visible to a 2nd user  | **FAIL** (BLOCKING) |
| Create-game without passcode — game becomes visible             | **FAIL** (BLOCKING) |
| Second user can Join with wrong passcode -> error               | UNTESTABLE (no game ever appears) |
| Second user can Join with correct passcode -> success           | UNTESTABLE |
| Lobby auto-refresh when games change                            | UNTESTABLE (no games appear) |
| Launch-page routing (`native_game.html`, `tui_game.html`, `demo.html`) returns 200 | PASS |
| Launch pages successfully boot WASM and play                    | UNTESTABLE in this run (WASM bundle not built) |
| Lobby behavior with server down                                 | PASS (graceful "Disconnected — retrying in 5s") |
| Mobile viewport (375x667)                                       | PASS (layout stacks correctly) |
| Form labels / accessibility                                     | PARTIAL (labels present; first Tab skips the input) |

**Bottom line:** The landing page itself is well-built — copy is clear, layout is responsive, the WebSocket lobby connects cleanly. **However the one user-visible promise of the lobby — "create a game; have a friend join" — does not work** because the lobby never sends `create_game` to the server. Instead it redirects to `native_game.html?lobby=create&...`, and that page has no lobby-param handler. The advertised flow is non-functional end-to-end.

## Step-by-step Narrative

### 1. Initial landing page (alice opens `http://localhost:8080/`)

![landing_01_initial](../web/screenshots/landing_page_qa/landing_01_initial.png)

The hero clearly explains what the project is, links upstream Forge, and surfaces a single primary CTA: "Pick a Username". The connection indicator on the meta line shows `Connected | ws://localhost:17810` immediately once the WS handshake completes. Username field is autofocused. Good.

### 2. Username submitted ("alice"); lobby pane revealed

![landing_02_username_entered](../web/screenshots/landing_page_qa/landing_02_username_entered.png)

Username pane hides; "Lobby" pane shows the create form on the left, the (empty) games table on the right, and a "Launch a Game" panel below. Welcome banner reads "Welcome, alice · change name". Open Games shows "No open games. Create one to start." as expected.

### 3. Create game pressed (name "qa-test-game", passcode "secret")

![landing_03_game_created](../web/screenshots/landing_page_qa/landing_03_game_created.png)

The user is redirected to:
`http://localhost:8080/native_game.html?lobby=create&game=qa-test-game&pass=secret&name=alice&ws=ws%3A%2F%2Flocalhost%3A17810`

The page renders a "Loading…" header and an otherwise blank body. In this test the WASM bundle was not built (`pkg/mtg_forge_rs.js` returns 404), but **even with the bundle**, `native_game.html` contains zero references to `lobby`, `URLSearchParams`, or `searchParams` (verified by grep). The lobby intent — "create a game on the server and wait for a peer" — is silently dropped.

### 4. Second user "bob" joins the lobby, refreshes Open Games

![landing_04_join_wrong_passcode](../web/screenshots/landing_page_qa/landing_04_join_wrong_passcode.png)

Bob's Open Games table reads "No open games. Create one to start." — confirming that alice's create attempt never produced a server-side game record. The wrong-passcode error scenario is therefore unreachable.

### 5. After bob also attempts to "create" an open (no-passcode) game

![landing_05_joined](../web/screenshots/landing_page_qa/landing_05_joined.png)

Bob is redirected to `native_game.html?lobby=create&game=open-game&name=bob&ws=...`, same blank "Loading…" outcome. The page never recovers.

### 6. Mobile viewport (375x667)

![landing_06_mobile_initial](../web/screenshots/landing_page_qa/landing_06_mobile_initial.png)
![landing_07_mobile_lobby](../web/screenshots/landing_page_qa/landing_07_mobile_lobby.png)

The hero, form, and lobby all stack to a single column. Spacing is comfortable and tap targets are reasonably sized. The `.grid-2` and `.launchers` media queries fire correctly at 720px and 600px respectively. No clipping or overflow observed.

### 7. WebSocket down (override `?ws=ws://localhost:9/`)

![landing_08_ws_down](../web/screenshots/landing_page_qa/landing_08_ws_down.png)

The conn indicator turns red and reads "Disconnected — retrying in 5s | ws://localhost:9/". The user can still type a username and submit, but the (correctly-disabled) Join button would not be reachable since there are no listings. Acceptable failure mode.

### 8. Launch-page smoke

![landing_09_native_game](../web/screenshots/landing_page_qa/landing_09_native_game.png)
![landing_10_tui_game](../web/screenshots/landing_page_qa/landing_10_tui_game.png)
![landing_11_demo](../web/screenshots/landing_page_qa/landing_11_demo.png)

All three pages return HTTP 200 and render their respective shells. None successfully boot the engine in this test because the WASM bundle (`web/pkg/mtg_forge_rs.js`) was not built. That is a test-environment artifact, not a regression introduced by this PR; the build agent's commit doesn't change those pages.

## Bug List

### BLOCKING-1: "Create game" never actually creates a game

- **Severity:** BLOCKING. This is the primary advertised feature.
- **Repro:** From the landing page, enter a username, fill in a game name + passcode, press "Create & Wait". Then in a second browser context as a different user, enter the lobby and press "Refresh List".
- **Expected:** the new game appears in Open Games for the second user.
- **Actual:** Open Games stays empty. Forever.
- **Screenshots:** `landing_03_game_created.png` (alice redirected to blank native_game), `landing_04_join_wrong_passcode.png` (bob sees no games).
- **Root cause:** `index.html` `createGame()` (lines ~469-498) builds a query string and `window.location.href = 'native_game.html?...';`. It NEVER sends a `create_game` WebSocket message. `native_game.html` does not parse `lobby=`, `game=`, `pass=`, or `name=` — `grep -n 'lobby\|searchParams\|URLSearchParams' web/native_game.html` returns zero hits. The commit message acknowledges this ("the game pages will pick these up when their network flow is wired through the lobby; in the meantime they ignore them") — but the lobby is shipped as if it works, with no in-UI hint that the flow is a stub.
- **Fix sketch:** Either (a) have the lobby itself send `client_message::CreateGame` over WS, render a "Waiting for opponent" state in-place, and only redirect once a peer joins; or (b) wire `native_game.html` to recognize `?lobby=create` and perform the create-and-wait there. (a) is simpler and matches the lobby-as-coordinator mental model in the commit message.

### BLOCKING-2: "Join game" is unreachable

- **Severity:** BLOCKING (derivative of BLOCKING-1).
- **Repro:** Same as BLOCKING-1.
- **Actual:** Since no game is ever listed, the Join button never appears, so passcode enforcement and the join-then-launch flow cannot be exercised at all from the UI. The Join code path in `joinGame()` (also redirects to `native_game.html?lobby=join&...`) has the same `lobby` param drop-on-floor problem.

### MAJOR-1: Username uniqueness is best-effort against waiting-game host names only

- **Severity:** MAJOR. Documented limitation in the commit body, but visible to users as a real correctness gap.
- **Repro:** Two browsers, both enter username "alice". Both succeed; both see "Welcome, alice" simultaneously. Confirmed in test run.
- **Root cause:** `nameIsTaken()` (index.html ~404) only checks `creator_name` of games in the latest `game_list`. With no games created (see BLOCKING-1), it always returns false. Even WITH games, it only collides on hosts of WAITING games — not joined peers or idle lobby users.
- **Fix sketch:** Either add a server-side `RegisterUsername` protocol message (the commit body acknowledges this is out of scope), or at minimum show an explicit UI warning that "names are advisory until you create or join a game". Today the UI implies stronger guarantees than it provides.

### MAJOR-2: Game pages ignore the entire lobby query-string contract

- **Severity:** MAJOR. The five params the lobby produces (`lobby`, `game`, `pass`, `name`, `ws`) have zero consumers in `native_game.html` or `tui_game.html`.
- **Root cause:** Same as BLOCKING-1. This is a documentation / contract bug as much as a code bug — there is no `// TODO(mtg-...)` referencing the gap, and no in-UI indication that arriving at the game page from "Create & Wait" should yield a network game (vs. the page's standalone-WASM default).

### MINOR-1: 404 on `pkg/mtg_forge_rs.js` is silent to the user

- **Severity:** MINOR.
- **Repro:** Open `native_game.html` (or `tui_game.html`, or `demo.html`) when the WASM bundle hasn't been built.
- **Actual:** Page shows "Loading…" indefinitely. No error surfaces to the user.
- **Fix sketch:** Catch the dynamic-import failure and render a clear "WASM bundle missing — run `make wasm-export`" message.

### MINOR-2: Empty `pass=` parameter still appears in URL when user didn't enter a passcode (only sometimes)

- **Severity:** POLISH.
- **Observed URL after open create:** `…?lobby=create&game=open-game&name=bob&ws=…` (no `pass=` — code does correctly omit when empty). Verified clean. Recording for completeness.

### MINOR-3: First Tab from page load focuses "Enter Lobby" button, not the username input

- **Severity:** POLISH / minor a11y.
- **Repro:** Load page, immediately press Tab.
- **Actual:** Focus lands on `#btn-name`. The username input is autofocused already (so its first focus state is "no Tab needed"), but a user who has touched another element first will Tab past the most important field. Acceptable but counter to convention.

### MINOR-4: "Refresh List" button does not visually indicate the request is in flight

- **Severity:** POLISH.
- **Observed:** Clicking the button has no visual feedback; the rendered table simply re-paints after the reply arrives. A `disabled`-while-`listPending` would help.

## UX Observations

- **Copy is good.** Hero clearly states what mtg-forge-rs IS (Rust port of Forge, AI-research-oriented, WASM in the browser). "No accounts, no passwords" framing for the username sets the right expectation. "Create & Wait" / "Refresh List" button labels are unambiguous.
- **Connection indicator is excellent.** The colored dot + state text + WS URL on the hero meta line is the right level of detail for a developer-targeted prototype. It correctly transitions Connecting → Connected → Disconnected with retry.
- **Affordance for the create-vs-launchers distinction is weak.** The page has TWO "start a game" surfaces: the lobby's "Create & Wait" (which should produce a multiplayer slot) and the "Launch a Game" panel below (which the comments describe as standalone-no-lobby). For a first-time visitor that distinction is not visually expressed — both look like equally-prominent CTAs. Recommend adding a one-line subtitle to each panel that explicitly says "Multiplayer" vs "Solo / Spectate".
- **Error messaging is mostly fine.** Name-validation status messages render in the right place with the right color. The only gap is server-side errors: a non-empty server password would surface as "Lobby refused list: …" which is technically correct but unfriendly.
- **Mobile is solid.** No horizontal scroll, no clipped buttons, fonts legible at 375px.
- **Form labels exist** (`label[for=username]`, etc) — screen reader basics are covered. Tab order is mostly sensible aside from the "first Tab" oddity noted above.
- **One small wart:** the welcome banner says "Welcome, **alice** · change name" — the "change name" link, when clicked, returns to the username pane but does NOT clear the input. If a user mistypes and wants to fix it, the fix is to backspace 32 chars. Minor.

## Recommendations (Prioritized)

1. **[BLOCKING] Wire `Create & Wait` and `Join` to actual `client_message::CreateGame` / `JoinGame` over the existing WebSocket** from the lobby itself, rendering "Waiting for opponent…" in-place. Only redirect to `native_game.html` once both players are paired. This is the single fix that turns the lobby from cosmetic into functional. (Alternative: make `native_game.html` honor `?lobby=create` / `?lobby=join` and reuse the same protocol there.)
2. **[BLOCKING-supporting] File a beads issue tracking the `lobby` query-string contract** between `index.html` and `native_game.html` / `tui_game.html`, and add a `// TODO(mtg-XXXX)` in `applyLaunchLinks()` so the gap is discoverable from code.
3. **[MAJOR] Extend the protocol with a `RegisterUsername` (or equivalent) message** so uniqueness is enforced server-side; without it, the "no accounts, no passwords" pitch is misleading.
4. **[MINOR] Surface WASM-bundle-missing as a real error** on the launch pages.
5. **[POLISH] Disable "Refresh List" while a `ListGames` request is in flight**, and make "change name" pre-select the input contents for easy overtype.
6. **[POLISH] Add a panel-subtitle distinguishing multiplayer lobby from standalone launchers.**

## Verdict on "Landing Page Prototype"

If "prototype" means *the static landing page + WebSocket lobby UI scaffolding*, this is an **acceptable prototype**: visually polished, responsive, properly connected to the lobby server, and good at the parts it does. As a stand-alone deliverable for marketing / orientation purposes, it works.

If "prototype" implies *a user can actually start a 2-player game from this page*, it is **not yet acceptable**: the create/join flows are non-functional end-to-end because the click-through pages do not consume the lobby intent. The commit message explicitly acknowledges this gap; the gap should be tracked as a follow-up issue and the UI should either be honest about it (e.g. greyed-out "Create & Wait (coming soon)") or completed.

Recommend merging the landing-page chrome but holding the "lobby works" claim until BLOCKING-1 and BLOCKING-2 are closed.
