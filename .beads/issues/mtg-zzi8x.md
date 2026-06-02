---
title: 'Waiting-room polish (live playtest): WS keepalive + ready-resets-on-all-config + creator Start-Game label + DeepScry image-source label'
status: open
priority: 3
issue_type: task
created_at: 2026-06-02T14:32:48.490481611+00:00
updated_at: 2026-06-02T14:33:09.354997578+00:00
---

# Description

Polish of the launcher/waiting-room from live playtest feedback. Web + lobby-protocol ONLY (no in-game engine changes). Child of mtg-682. Branch waiting-room-polish, base integration 406a4935.

FOUR FIXES (all in web/launcher.html unless noted):

(A) Waiting-room WebSocket idle-disconnect. A creator waiting ~1min got "Disconnected from server" because the idle waiting socket (sends nothing while waiting for an opponent) hit Cloudflare's ~100s idle-WS timeout. NOT a game timeout (DEFAULT_GAME_TIMEOUT=4h; WAIT_FOR_JOINER=30min and is DISABLED in rendezvous mode per server.rs:1168 `if !rendezvous`). FIX: client-side keepalive — launcher sends ClientMessage::Ping {type:"ping", timestamp_ms} every 25s (KEEPALIVE_INTERVAL_MS, < ~100s cutoff) on the waiting lobby socket, started in onopen, cleared in onclose. The server ALREADY answers Ping with Pong in BOTH rendezvous waiting-loops (server.rs:1344 creator, :1765 joiner) and there is NO time-based prune of waiting_games (only removed on disconnect/join/both-ready), so an idle "waiting for an opponent" session now survives for hours, up to the 4h game timeout. NO server/protocol change needed (Ping/Pong already existed). Pre-game network only — no game RNG / no controller state — determinism preserved.

(B) Ready must reset on ANY pre-game config change. Previously ready auto-reset on deck/collection/renderer change but NOT on Debug or image-source/show-images. FIX: attach autoUnready('settings changed') to the show-images, scryfall, gatherer, local, and debug checkboxes (in addition to the existing deck/collection/renderer handlers). autoUnready re-broadcasts SetReady=false and is a no-op when not currently ready, so "ready" now always means the exact config that will launch.

(C) Creator button label reacts to the joiner joining. Button was static "Ready — start on P2 join" even after P2 joined. FIX: track joinerPresent in renderWaitingRoom (driven by WaitingRoomUpdate); readyButtonLabel() returns "Ready — start on P2 join" before a joiner is present and "Start Game!" once present (creator), "Ready" for the joiner side, "Cancel ready" when self-ready. Variant-1 semantics unchanged: the game still AUTO-starts only when BOTH are ready (this is a label/state cue, not a start trigger).

(D) Image-source label rename. "Local (fastest, offline)" was misleading on a hosted site (the VM/DeepScry server serves the images, not a local cache). Renamed user-facing label to "Load from DeepScry server". Internal ids (#img-src-local) and the img_src=local token are UNCHANGED. Game pages (native_game.html/tui_game.html) had no user-facing "Local (fastest, offline)" label — only internal code comments — so no game-page label change was needed.

TESTS (web/test_redo_lobby_e2e.js, in make validate via validate-network-e2e-step):
 - wr-creator-button-start-on-join: creator btn flips to "Start Game!" when joiner joins.
 - wr-joiner-button-ready: joiner btn reads "Ready".
 - wr-keepalive-ping-sent + wr-keepalive-survives: drives ONE real ping over the live waiting socket via window.__launcherWaiting.sendKeepalivePing() (same path the 25s timer uses; a true 100s idle is impractical to e2e), asserts pingsSent increments and the socket stays connected (server accepted it).
 - wr-ready-resets-on-debug-toggle: toggling Debug after Ready clears ready.
 - parity-img-src-label + parity-img-src-label-old: label reads "Load from DeepScry server" and the old "fastest, offline" text is gone.
 All pre-existing launcher-parity + waiting-room assertions preserved and still green.

RELATIONSHIP TO JAVA FORGE: N/A — this is the Rust/web lobby rendezvous frontend (Variant-1 launcher waiting room), which has no Java-Forge analogue.
