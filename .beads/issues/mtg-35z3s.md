---
title: 'REDO: lobby/launcher/game UI overhaul done RIGHT — 4-page split, native default, ONE launcher, gated on an end-to-end PLAYED-GAME test'
status: open
priority: 1
issue_type: task
created_at: 2026-06-01T12:31:21.332680376+00:00
updated_at: 2026-06-01T12:31:21.332680376+00:00
---

# Description

## Lobby/Launcher/Game REDO — Spec & Acceptance (2026-06-01)

_Authoritative spec after the AFK build FAILED to implement the agreed architecture
(no page split, renderer selector left on the lobby, double launcher, game froze
after first land). This doc is the contract; every merge is diffed against it.
Tracking: **mtg-redo** (this effort). Supersedes the architecture decisions in
LOBBY_LAUNCHER_GAME_ARCHITECTURE_20260531.md where they conflict._

## What went wrong (root cause = wrong acceptance gate)
Merges were gated on `make validate` green + a skeptic reading the diff. NEITHER
exercises the human play journey. Result: code that compiles + passes shallow web
smoke, but the lobby→game flow was never once driven end-to-end. "validate green"
is necessary, NOT sufficient. Also: agents drifted from the design doc and the
coordinator rubber-stamped plausible-looking diffs instead of diffing intent vs result.

## The architecture (LOCKED — 4 pages, clear separation)
1. **Lobby** (`index.html`, served at `/`): register a unique name → browse live
   games → **Create** or **Join**. NO renderer choice here. NO deck picker here.
   Creating/joining a game says NOTHING about how you personally render or which
   deck — those are launcher concerns. After Create/Join → go to the LAUNCHER.
2. **Launcher** (`launcher.html`, NEW): the per-player pre-game screen. Here you
   pick: **your deck** (with deck COLLECTIONS, not a flat dropdown) + a **"Deck
   Editor" launch** button + **your renderer: Native GUI (DEFAULT) or Web TUI**
   (alternate). Renderer is a per-player experience detail chosen HERE, never in
   the lobby. Then "Play" → go to the matching game page. This is the ONE launcher.
   (The good launcher logic — collections/decks/controllers — currently lives
   built-into native_game.html lines ~824-929; extract it here.)
3. **Native game** (`native_game.html`): PURE renderer (card DOM). NO built-in
   launcher (delete the `#launcher` block). Receives everything via params.
4. **Web TUI game** (`tui_game.html`): PURE renderer (ratzilla). NO built-in launcher.
   - **Native is the default.** A creator and joiner each pick their own renderer
     in their own launcher; the two can differ (one native, one TUI) — same game.
5. **Deck editor** (`deck_editor.html`): reachable from the launcher's Deck Editor
   button (already exists; wire it in).

Flow: `index(lobby)` → create/join → `launcher.html?game=…&role=create|join&…`
(register/create/join already happened on the lobby WS; launcher picks deck+renderer
+ ready) → `native_game.html` OR `tui_game.html` with the full param contract.

## Determinism / netarch dependency (the freeze)
The reported "only first land worked, then froze" + "reload/back corrupts state"
are network-play robustness failures rooted in the UNFINISHED netarch (mtg-c9fuc:
live step_harness doesn't rewind for begin-of-phase/combat/reset re-entry;
mtg-uzvu4: human-controller network desync). A clean UI on a broken play path is
still broken. So the played-game acceptance test (below) is what surfaces these,
and the netarch finish (mtg-c9fuc) is in-scope for "playable", not optional.

## ACCEPTANCE TEST (the definition of done — build FIRST, red is expected)
A single automated harness (Playwright 2-client + headless WASM, under
`web/test_*_e2e` invoked by a dedicated make target, NOT just the existing shallow
smoke) that drives the REAL flow and PASSES:
1. Two clients register distinct names on the lobby; duplicate name rejected.
2. Client A Create → lands on launcher → picks deck + renderer; Client B sees the
   game in the list → Join → lands on launcher → picks deck + renderer.
3. Both reach a game page (A native default, B native default; also a leg with B = TUI).
4. **Play MULTIPLE FULL TURNS** (not just turn 1 / first land): plays lands, casts,
   passes through combat, advances ≥3 turns with both clients staying in sync
   (no freeze, no desync, hashes agree).
5. **Reload-resilience**: reload one client mid-game → it reconnects (reconnect
   token) and resumes in sync, OR fails CLEANLY with a clear message — never a
   silent corrupt/frozen state.
6. Renderer: native game page shows CARDS (not the ratzilla TUI).
NO lobby/launcher/game task is "done" until this is GREEN. The coordinator RUNS it
(does not trust an agent's claim) and diffs the implementation against THIS doc's
architecture before any merge.

## Process corrections (coordinator)
- Build the acceptance harness BEFORE implementation; it defines done.
- Every agent brief cites THIS spec; every result is checked by (a) running the
  harness and (b) diffing intent-vs-result against the architecture section, not
  by reading the agent's prose.
- Stage to a NON-LIVE URL / verify before promoting to deepscry.net.
- `make validate` green is a precondition, never the done-signal.

## Open known issues this must also resolve (from the failed build)
- Renderer selector on lobby (index.html:376-384) → REMOVE.
- TUI is default → change to NATIVE default (in the launcher).
- Double launcher (lobby flat picker + native_game built-in) → ONE launcher (launcher.html).
- Join has no per-player renderer choice → launcher gives each player their own.
- Game freeze after first land → netarch mtg-c9fuc / play-path robustness.
- Reload/back corrupts → reconnect-token resume or clean failure.
