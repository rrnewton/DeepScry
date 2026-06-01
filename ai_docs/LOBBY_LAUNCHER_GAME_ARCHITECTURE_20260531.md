# Lobby / Launcher / Game — Architecture & Storyboard

_AI design doc (2026-05-31). Authoritative tracker: **mtg-khy7x**. Status: **awaiting user sign-off** before implementation._

This captures the agreed redesign of the DeepScry web flow, replacing the
current confusing path (duplicate deck-pickers, a client-side "waiting room"
that registers nothing server-side, a native-GUI checkbox that still renders the
TUI). It records the decisions reached in the 2026-05-31 design discussion.

## Decisions (locked)

1. **Page topology: 3 thin pages + a 4th for the deck editor.** WASM is per-page
   and dies on navigation; we accept that and use **prefetch** (HTTP byte-cache
   warming) rather than trying to keep a warm instance across a navigation.
   - **`lobby.html` (a.k.a. the light page) = lobby + launcher/waiting room.**
     No game render. Talks only WebSocket. Hosts: user registration, live game
     list (browse/create/join), and the **waiting room** where you pick deck +
     options + renderer. Begins a **background prefetch** of the WASM bundle and
     the selected decks' per-set `.bin`s during the idle wait so the game page's
     re-fetch is a cache hit.
   - **`native_game.html`** — thin shell: imports the shared layer, mounts the
     **native card renderer only**.
   - **`tui_game.html`** — thin shell: imports the shared layer, mounts the
     **ratzilla TUI renderer only**.
   - **`deck_editor.html`** (4th page, later) — reachable from a "Deck Editor"
     button on the launcher.
   - **Renderer is chosen in the launcher**, which then navigates to the
     matching single-renderer game page. Because each game page owns exactly one
     renderer, the "native URL but TUI renders" bug is structurally impossible.
   - **No live mid-game TUI↔native switching** (explicitly out of scope now).

2. **Prefetch, not warm-instance (for now).** Assets are content-addressed +
   `immutable` (the cache-skew fix), so a background `fetch()` of
   `pkg/mtg_engine.<hash>.{js,wasm}` + the selected decks' `*.<hash>.bin` warms
   the HTTP cache; the game page then instantiates/parses from cache with no
   network round-trip. Accepted residual cost on the "Play" critical path:
   WASM instantiate + bincode parse of 2–5 per-set bins (no network).
   _Revisit later if that residual proves too slow — the fallback is the
   "warm-instance, launcher+game same page" design, deliberately deferred._

3. **DRY: a real shared layer.** Everything above the renderer is duplicated
   today across `native_game.html` (2895 lines) and `tui_game.html` (4017
   lines). Extract it into shared ES modules (the way `lobby_launcher.js`
   already works). The renderer is the ONLY page-specific piece. See "Shared
   layer" below. (mtg-tnsk7)

4. **Server owns authoritative state** (today it owns almost nothing until the
   game page fires `CreateGame`). Chosen scope:
   - **Registered users with unique names** (server-enforced; today advisory).
   - **Waiting games + liveness/heartbeat** — registered the instant you create,
     evicted on WS-drop / missed heartbeat (fixes stale-games, mtg-dw9j3).
   - **Per-player deck + ready state** in the waiting room.
   - **Reconnect tokens** (design for it; a game living at its own URL +
     token is the rejoin handle — favors separate game pages).

5. **One deck picker, in the waiting room** (launcher view). Delete the
   downstream picker on the game pages; the chosen deck flows in via params.

## Shared layer (extract — consumed by both game pages; some by lobby/editor)

| Module (proposed) | Responsibility | Today |
|---|---|---|
| `lobby_launcher.js` (exists) | param contract: consume/apply/build redirect | keep, extend (mtg-1vwpd) |
| `wasm_boot.js` | `init()`, manifest (`sets/index.json`) resolution, `load_set`/`load_tokens`, prefetch helper | duplicated in both pages |
| `card_images.js` | image cascade local→scryfall→gatherer, `allow_local_img_load` gate | dup'd (native has cascade, gate in both) |
| `net_game_driver.js` | **renderer-agnostic** network client + controller loop (consumes WS choice requests, drives a controller); pluggable view | **the layering bug** — both pages currently funnel into the TUI renderer (mtg-tnsk7) |
| `help_dialog.js` | shared help modal (`tui_get_help_text`) | dup'd ~20 refs each |
| `bug_report.js` | report UI incl. optional **advanced "trusted password"** field (blank ⇒ untrusted) | — |
| per-page | **renderer only**: native card DOM vs ratzilla TUI | the legitimate difference |

## Storyboard (screen by screen)

```
[lobby.html]
  (1) REGISTER          server: register name -> unique?  (rejects dup live)
        |
        v
  (2) LOBBY/BROWSE      live game list (heartbeat-pruned) | [Create] [Deck Editor]
        |  create or join
        v
  (3) WAITING ROOM      ONE deck picker + options (card images, renderer TUI|native)
        | server: CreateGame NOW -> game is REAL + joinable immediately
        | server tracks: my deck, my ready state; opponent appears when they join
        | background: PREFETCH wasm + selected-deck bins (idle warm)
        | both players Ready
        v  navigate to native_game.html OR tui_game.html  ?game&pass&name&ws&deck&token
[native_game.html | tui_game.html]
  (4) GAME             boot WASM (from cache), load decks' set bins, mount ONE renderer
                        net_game_driver drives controller; reconnect via token+URL

[deck_editor.html]     (4th page, later) reached from (2)/(3); build/edit decks
```

### Server-state lifecycle (waiting room is REAL now)
- `CreateGame` fires when you enter the waiting room (not after a 2nd click on
  the game page) → opponent sees it instantly in the list.
- Waiting room is backed by the live lobby WS; closing the tab drops the WS →
  server evicts the room (no more stale games).
- Per-player deck + ready tracked server-side; match starts when both ready.

## Known bugs folded into this work
- **Native-renders-TUI** (`native_game.html` network path calls
  `launch_network_game` = ratzilla; no native network renderer exists) → fixed
  by `net_game_driver.js` + a real native renderer. (mtg-tnsk7)
- **Stale games** in the list → server heartbeat/eviction. (mtg-dw9j3)
- **Double deck-picker / double create-game** → one picker in the waiting room;
  real server-side create on entry. (mtg-khy7x)
- **`cards.bin` + harness pages still fixed-name** (content-addressing hole) —
  related cleanup. (mtg-33fmb)

## Deferred / out of scope now
- Live mid-game renderer switching.
- Warm-instance (launcher+game same page) — fallback if prefetch proves slow.
- WASM-powered deck builder requiring a JSON card catalog (deck *editor* is its
  own page; selection-by-name needs no WASM via `deck_names` in index.json).

## Open sub-questions for implementation
- Does `lobby.html` replace `index.html` or is `index.html` the lobby? (naming)
- Deck *validation* in the waiting room: selection-by-name is WASM-free; do we
  validate the chosen deck against the card DB before launch, or trust the
  manifest list? (Leaning: trust manifest now; editor validates.)
