---
title: Consolidate network/debug verification behind ONE per-game runtime DEBUG flag (launcher checkbox)
status: open
priority: 2
issue_type: task
labels:
- debug
- network
created_at: 2026-06-02T14:04:00.403555773+00:00
updated_at: 2026-06-02T14:04:00.403555773+00:00
---

# Description

GOAL (user direction 2026-06-02): replace the scattered debug/verification toggles with ONE per-game runtime DEBUG flag, surfaced as a single DEBUG checkbox on launcher.html. Willing to pay a well-predicted runtime `if debug` branch rather than compile-time cfg or many separate knobs. SEQUENCING: dispatch ONLY AFTER the netarch reveal-history-buffer work (mtg-610, branch netarch-undo-holes / agent a1059056) LANDS — it rewrites the same sync/verifier/protocol path (network/controller.rs, apply_state_sync, replay_verifier, protocol.rs, the WASM client); concurrent work = guaranteed conflict, and the consolidation wants netarch's clean reveal-buffer + verifier in place first.

CURRENT FRAGMENTED STATE (what to unify):
- DebugStateDump: a `#[cfg(debug_assertions)]` ServerMessage (protocol.rs ~805 + client match-arms wasm/network/client.rs ~862, network/client.rs ~198) — full-state JSON dumped to client ON hash-mismatch. COMPILED OUT of release-deploy → unavailable in prod.
- network_debug: runtime bool, but set by the SERVER's `--network-debug` CLI launch flag (global), pushed to clients. Gates the per-choice compute_view_hash client<->server comparison (local_controller.rs ~198/257; server.rs log_state_hash_mismatch + FATAL ~2449/2640). Prod server-web launches WITHOUT it → OFF in prod.
- verify_rewind_replay: runtime, WASM export tui_set_verify_rewind_replay (fancy_tui.rs ~125). Gates the rewind/replay self-consistency verifier (replay_verifier::verify_replay; REWIND/REPLAY FATAL). TUI-ONLY today (native_game never calls it) — an inconsistency to fix.
- set_log_level('trace'): WASM export; TRACE unlocks the log::trace action-log dumps. The launcher 'Debug logging (TRACE)' checkbox -> &debug -> set_log_level + (on TUI only) tui_set_verify_rewind_replay. So today's launcher checkbox already does DIFFERENT things per renderer (mislabel + asymmetry).
- netarch undo-log dump: env MTG_NET_FULL_UNDO_DUMP / --undo-dump (netarch branch only).

CONSOLIDATED DESIGN — ONE per-game `debug` flag. When ON for a game (off by default; one launcher checkbox), enable ALL of:
1. TRACE logging (set_log_level).
2. PER-CHOICE client<->server compute_view_hash comparison (network_debug) — PER-GAME, NOT requiring the server to launch with --network-debug. Carry the flag in CreateGame/JoinGame; the server applies it to THAT game's controllers only. (Advanced per-game control on the web-server, as the user wants.)
3. Rewind/replay verifier (verify_rewind_replay) — make it CONSISTENT across BOTH renderers (native + tui), not TUI-only.
4. DebugStateDump on mismatch — MOVE from cfg(debug_assertions) to a runtime branch so it's available per-game in prod (the message variant becomes always-compiled, runtime-gated).

UNDO-LOG DUMP — NOT unconditional when debug is on (user refinement). Treat it as a CRASH DUMP: by default it fires ON MISMATCH only (like DebugStateDump). Expose granular control under the debug flag:
- UndoLogDump=onCrash (default when debug on) — dump the full undo log only on a hash mismatch.
- UndoLogDump=always — separate opt-in for targeted every-action tracing (high volume; for deep debugging only).

PROPAGATION: launcher DEBUG checkbox -> &debug param -> (client) set_log_level + verify_rewind_replay + (via CreateGame/JoinGame) server-side per-game network_debug + onCrash dumps. Off by default; one branch cost in normal games.

SUPERSEDES the earlier 'split the launcher debug toggle into verbose-logging vs rewind-verify' idea — user wants ONE master switch, not many separate ones.

OUT OF SCOPE: game/mana_engine.rs debug_assertions (unrelated internal mana-cache invariant; leave as cfg).

FILES (expected): network/protocol.rs (per-game debug field in CreateGame/JoinGame; DebugStateDump runtime; undo-dump granularity enum), network/server.rs (per-game network_debug + onCrash dumps), wasm/network/client.rs + wasm/fancy_tui.rs + web/native_game.html + web/tui_game.html (single &debug -> log+verifier+network_debug, consistent both renderers), web/launcher.html (one DEBUG checkbox, off default) + lobby_launcher.js.
