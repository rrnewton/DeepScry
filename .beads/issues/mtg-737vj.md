---
title: Thread multiplayer RNG seed through CreateGame to both clients (server side of mtg-2csf2)
status: open
priority: 3
issue_type: feature
created_at: 2026-06-05T16:40:34.609055201+00:00
updated_at: 2026-06-05T16:43:49.608719008+00:00
---

# Description

FOLLOW-UP to mtg-2csf2 (web advanced-options + multiplayer-seed UI field).

The launcher (web/launcher.html) now shows an advanced RNG-seed input for the game CREATOR, gated behind ?advanced_options=true. It is currently UI-ONLY and intentionally NOT wired into the live game launch.

WHY it can't be wired client-side: a network game's controller seed must be IDENTICAL on both clients for the deterministic sequential simulation (docs/NETWORK_ARCHITECTURE.md). Today both clients default controllerSeed to 0. The launcher's per-client redirect (lobby_launcher.js buildRedirectQuery) cannot share a creator-chosen seed with the joiner — each client builds its own URL and consumeLobbyParams does not even parse seed for network boots. Forwarding the seed to ONLY the creator would DESYNC the joiner (fatal).

REQUIRED server work (do NOT do client-side only):
1. Add an optional seed field to ClientMessage::CreateGame in mtg-engine/src/network/protocol.rs.
2. Server stores the creator's seed for the waiting-room game and includes it in GameStarted (or equivalent) sent to BOTH players.
3. Game pages (native_game.html / tui_game.html) read the server-provided seed into bootConfig.seed so controllerSeed matches on both sides.
4. consumeLobbyParams / the network boot path must consume the server seed, not a URL param (URL param would let a creator pre-test a known hand — keep it server-authoritative + advanced-only).
5. MTG rules review N/A (no gameplay logic change); but determinism must be verified: run the same seed twice and confirm identical gamelogs on BOTH clients.

Purpose: run the SAME random/random multiplayer game repeatedly and verify an identical outcome (testing/benchmarking). Until this lands, the launcher seed field stays display-only with a 'pending server support' note.

UI side delivered in: web/launcher.html (#advanced-seed-field), gated by resolveAdvancedOptions().
