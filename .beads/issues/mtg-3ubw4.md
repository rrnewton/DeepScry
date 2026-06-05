---
title: 'NETARCH cleanup: finish eager->buffer migration for CardRevealed/LibraryReordered/SearchCandidates + delete RemoteController legacy mode (audit A2/A3/A4/Q3)'
status: open
priority: 3
issue_type: task
depends_on:
  mtg-o99ow: related
created_at: 2026-06-05T14:08:25.837544183+00:00
updated_at: 2026-06-05T14:08:25.837544183+00:00
---

# Description

From audit §A2/A3/A4 + Q3 LibraryReordered list. The eager message zoo is being deleted asymmetrically: OpponentChoice is already dead (see sibling issue), but CardRevealed (server.rs:2162-2206 setup, 2934 mid-game flush), LibraryReordered (still DUAL-EMITTED server.rs:3132), and SearchCandidates (server.rs:2984) are still sent alongside their BufferedFact:: forms. A4 (Phase-2, SEQUENCE AFTER deep-AC, high risk — reveal/replay path): converge to buffer-only; remove ServerMessage::LibraryReordered (protocol.rs:730-745), GameToHandler::LibraryReordered plumbing (server.rs:455/2498-2513/2704-2719/3119-3140), client decode arms (client.rs apply at 766-775; wasm 1007/1336-1345), and retarget stale comments describing the eager LibraryReordered protocol as current (state_sync.rs:47; controller.rs:982/1007; action_log.rs:12/120; game/state.rs:311-334/750/819-832/4188/4230/4240; undo.rs:311/2565; server.rs:3434). NOTE game-setup LibraryReordered/CardRevealed sends need a replacement initial-sync path BEFORE deletion, not a blind cut. A2 (SAFE): delete RemoteController::new legacy constructor + Option<shared_state>/panic mode (remote_controller.rs:48/59-66/95-97/232-237) -> tighten to non-optional Arc<SharedNetworkState> (only new_with_shared_state used, client.rs:2258). A3 (SAFE): delete #[allow(dead_code)] handle_choice (local_controller.rs:383-397) after confirming no callers.
