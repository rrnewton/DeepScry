---
title: 'Web UI fixes (deepscry.net native_game): in-list AI-watch choices, WS-disconnect red banner, debug-gated Network logs, Safari card-flicker node reuse'
status: open
priority: 2
issue_type: task
created_at: 2026-06-09T21:47:08.267689746+00:00
updated_at: 2026-06-09T22:50:01.565199501+00:00
---

# Description

User-facing fixes for the native card-style web game (web/native_game.html) on
deepscry.net, on branch claude/web-ui-fixes.

FIX 1 — affordance as NORMAL choice-list options (not a footer).
When an AI seat is paused and it is our move to advance, the action pane shows
ordinary numbered choices "1. Play next <Kind> AI move" / "2. Auto-run" — the
same UI a human game uses — instead of a separate footer banner. Implemented via
computeMetaChoices(state) + data-meta action-items; click, 1-9 number keys, and
Space all dispatch through selectMetaChoice() (continue = single step, autorun =
enable auto-run). The bottom footer (renderActionAffordance) is now the GREEN
"auto-running" banner ONLY (the user wanted to keep that).

FIX 3a — wrong "waiting for the other player" text on our own turn.
That contradictory footer is removed. The synthetic "continue" choice is
suppressed exactly when the engine is genuinely blocked on the server (it sets
current_prompt = "Waiting for server..."), centralized as
ENGINE_PROMPT_WAITING_FOR_SERVER.

FIX 2 — escalate WebSocket disconnects to the bottom red banner.
network.js: a NON-clean onclose and onerror produce an actionable message routed
through onError (clean closes stay quiet). native_game.html: new
window.__showNetworkError (sharing a DRY appendBanner helper with
__showWasmEngineError) renders it in the existing red banner; networkClient.onError
is wired to it. Builds on the spirit of mtg-270.

FIX 3b — gate [Network] traffic log spam behind debug tracing.
network.js: a per-client `debug` flag (default OFF) gates every
"[Network] Received/Sending/State/Connecting/…" console.log via a _log() helper;
genuine errors keep console.error AND surface via onError. native_game.html sets
networkClient.debug = isDebugMode() and routes its per-frame [Network] logs
through debugLog. (The one-time load-time "features available" notices stay plain
console.log — they run before bootConfig's `let` initializes; gating them via
debugLog/isDebugMode would TDZ-crash the page.)

SAFARI CARD FLICKER (audit ai_docs/transient/NATIVE_GUI_RENDERING_AUDIT_20260609.md).
The battlefield rebuilt via full innerHTML teardown every ~150ms, recreating every
card <img>; Safari blanks each freshly-created element while it re-decodes ->
flicker. Fixed by DOM reconcile / node reuse in renderBattlefield: card tiles are
pooled by card_id and reused; an unchanged card's <img> is never recreated.
createCardElement stamps data-img-key; updateCardElement patches in place,
rebuilding the <img> only when that key changes. Plus a decode-timing narrowing
(decoding="sync", no loading="lazy") on the small on-screen tiles. Also cut
per-frame churn massively (real-advance battlefield mutations ~24 -> ~3).

TESTS (all wired into make validate browser suite):
  - web/test_action_affordance.js rewritten for the choice-based design: 9/9 PASS.
  - web/test_render_skip.js extended with node-reuse property (C): 36/36 surviving
    battlefield cards kept their <img> node identity across an advance.
  - test_aura_render.js (7/7), test_game_gui_rebuild.js (19/19),
    test_image_flicker_memo.js (8/8) still green after the badge move + reconcile.

Commits: 4bf17463 (fixes 1/2/3), 6229c948 (flicker + tests). Presentation layer
only; no Rust, no engine/protocol/determinism change. Related (closed): mtg-270
(network console spam behind debug flag), mtg-378 (replay verifier in native_game).

FOLLOW-UP (commit 7cdc602f): gating the [Network] Received log broke the network
e2e completion detection (tests scraped that log for "type":"game_ended"), and
the onclose escalation raised a spurious 'connection lost' red banner on the
normal post-game 1006 socket close. Fixed: network.js sets a gameEnded flag +
emits ONE clean '[Network] Game ended' notice on the game_ended message;
onclose/onerror skip escalation/reconnect once gameEnded; tests detect
completion via that notice + view-model game_over. Bisection: clean integration
3/3 PASS, pre-fix 3/3 FAIL, post-fix 3/3 PASS. Commits now: 4bf17463, 6229c948,
8d24320a (data-nonimg sig), 7cdc602f (net e2e + banner fix).
