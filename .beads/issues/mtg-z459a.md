---
title: WebSocket reconnect + deterministic state recovery (undo-log replay)
status: open
priority: 2
issue_type: feature
labels:
- network
- web
created_at: 2026-06-10T02:01:22.147487784+00:00
updated_at: 2026-06-10T02:01:22.147487784+00:00
---

# Description

WebSocket reconnect + deterministic state recovery (undo-log / choice-buffer replay).

PROBLEM (user report): On Safari, auto-run random-vs-random games to completion
frequently hit `WebSocket connection to 'wss://deepscry.net/lobby' failed: The
network connection was lost.` There is NO good recovery: on a mid-game socket
drop the client cannot rejoin the in-progress game and catch up. Today it at
best shows "Connection lost. Please reconnect." and the game is lost.

GOAL: on a drop, the client reconnects AND catches up — restored to the exact
server frontier, ready to continue, even from completely lost client state.

Full design doc (parent workspace, not project tree):
  ai_docs/RECONNECT_STATE_RECOVERY_DESIGN_20260609.md
  (= <parent>/ai_docs/RECONNECT_STATE_RECOVERY_DESIGN_20260609.md)

WHY LOG-REPLAY, NOT A SNAPSHOT (determinism-critical):
The sim is a deterministic sequential state machine (docs/NETWORK_ARCHITECTURE.md;
desync is ALWAYS fatal). The blocking model is rewind-to-turn-start + replay over
the undo_log; rewind+replay must return compute_state_hash EXACTLY (guarded by
mtg-engine/tests/rewind_replay_oracle_e2e.rs). A raw GameState snapshot restores
surface bytes but NOT the undo/choice/state-sync logs -> the first subsequent
rewind double-applies/skips -> immediate FATAL desync. A snapshot also forces a
second per-recipient redaction code path (info-leak/divergence risk). The
deterministic model wants the LOG, not a photograph. The user's instinct is
correct. Snapshot is only viable later as an accelerator LAYERED ON TOP of
log-replay + the hash gate, never instead of it.

KEY FINDING — the primitive already exists. Reconnect-resume is the EXISTING
per-choice catch-up mechanism widened from "facts since my last choice" to
"facts from action_count 1..frontier":
- ActionLog<T> keyed by action_count, append-only, NON-DESTRUCTIVE reads
  ("rewind/replay is free") — network/action_log.rs.
- ChoiceRequest carries `buffer: Vec<(u64, BufferedFact)>` (mtg-752) — the
  ascending-ac catch-up payload of reveal+opponent-choice facts; client.rs ~L84.
- apply_choice_buffer(buffer) (client.rs ~L801) and reset_state_sync_cursor()
  (client.rs ~L1053, explicitly "for snapshot-resume/rewind") already route
  facts into the state-sync + opponent-choice logs.
- compute_view_hash(view) (game/state_hash.rs ~L396) is the shadow-side hash the
  client already sends per ChoiceResponse under --network-debug -> our VERIFY GATE.
- ReconnectToken + ClientMessage::Reconnect + ServerMessage::ReconnectResult +
  GameStarted{reconnect_token} ALL EXIST (protocol.rs); lobby validates the token.

WHAT'S MISSING (the stubs):
- Server reconnect handler (server.rs ~L810) validates the token, replies
  success, logs "Phase 3 resume pending", then return Ok(()) — the socket is
  NEVER reattached to the running game loop (mtg-682 stub).
- Client _scheduleReconnect() (web/network.js ~L326) is a STUB: after 3s it just
  calls onError("Connection lost. Please reconnect.") — never opens a new socket,
  never resumes.
- Server ABORTS the whole game on a single-peer drop via Err(_) (server.rs ~L2427)
  -> resume is meaningless until the server holds the game open. This is the
  riskiest change (it relaxes a desync-fatal abort) and needs review BEFORE code.

WHAT slot05's JUST-LANDED fix (commit 66bc207c / 34454349, mtg-grofw,
isLegitimateGameEnd) ALREADY COVERS: a mid-game peer reload made the server send
the survivor a DEGENERATE game_ended {winner:null,reason:"draw",action_count:0};
the web UI treated any game_ended as a clean end and SUPPRESSED reconnect ->
survivor silently FROZE. The fix added isLegitimateGameEnd (winner OR
action_count>0) so the abort teardown is routed through the normal
connection-lost path instead of being swallowed. It does NOT make
_scheduleReconnect actually reconnect, does NOT resume/catch-up, and does NOT
stop the server aborting the game on a peer drop. It only makes the give-up path
fire instead of freezing.

DESIGN — log-replay resume:
On reconnect the server streams the player the FULL ordered catch-up payload
action_count 1..K (K=frontier), redacted to that player's shadow view via the
SAME redaction used for ChoiceRequest.buffer. Client re-inits deterministically
(decks+opening), reset_state_sync_cursor(), apply_choice_buffer, drives forward
to K, then compute_view_hash gate: match -> resume; MISMATCH -> FATAL DESYNC
(no stumble-along). Protocol add: ServerMessage::ResumeState { your_player_id,
frontier_action_count, buffer: Vec<(u64,BufferedFact)>, frontier_view_hash,
pending_choice: Option<ChoiceRequest> }. Reusing BufferedFact (not a bespoke
snapshot type) keeps ONE tested redaction path. Determinism-safety: we never
transplant opponent state; we replay the same facts in the same ac order through
the same apply_* code, reconstructing the state-sync+choice logs so the first
subsequent rewind behaves identically to a never-dropped client; resume is gated
on PROVEN bit-identity (compute_view_hash), never assumed.

EDGE CASES: server-aborts-on-drop (Phase-1 prerequisite); reconnect mid-rewind
(safe — we re-init from scratch to clean frontier K, never resume a half-rewound
state); lobby-WS-vs-in-game-WS (pre-GameStarted drop has no token -> normal lobby
rejoin); rapid drops / reconnect storms (token doesn't rotate -> idempotent;
guard two sockets claiming one player_index; capped backoff); K=0/fresh client
(degenerates to a normal new join); expired token / ended game (success:false ->
real "cannot rejoin" message).

SAFARI: aggressive idle-socket reaping (backgrounded tabs frozen, energy-saver
timer suspension, blips -> 1006). Gaps: no app-level keepalive ping, no real
auto-reconnect (the stub), timer suspension freezes the WASM loop. Add keepalive
(reduces drop FREQUENCY); replay-resume makes a drop RECOVERABLE (the real fix).

PHASED PLAN w/ review checkpoints:
- Phase 0 (low risk, ship first): app-level keepalive ping/pong + honest
  "reconnecting" banner. Zero determinism impact. SMALLEST SAFE FIRST INCREMENT.
- Phase 1 (HIGH review BEFORE code): server holds game open on single-peer drop
  (pause+hold for reconnect window instead of Err(_) abort). Relaxes a
  desync-fatal abort -> requires NETWORK_ARCHITECTURE review first.
- Phase 2 (HIGH review): server reconnect socket rebind + build ResumeState.buffer
  from undo_log 1..K via the EXISTING redaction (must not fork it) +
  frontier_view_hash.
- Phase 3 (HIGH review): real client _scheduleReconnect -> new socket + Reconnect;
  on ResumeState re-init + apply_choice_buffer + drive to K + compute_view_hash
  gate FATAL-on-mismatch. Deterministic e2e: forced mid-game drop+reconnect
  asserts identical frontier hash and same gamelog as an undropped control; plus
  a NEGATIVE test (corrupt one buffered fact -> resume MUST FATAL).

DETERMINISM RISK IN CURRENT PATH: the _scheduleReconnect stub is safe-but-useless
(can't desync — never resumes). The real latent risk is Phase 1 relaxing the
server peer-drop abort; the mandatory compute_view_hash FATAL gate on resume is
what keeps a stale/forged client from rebinding into a diverged state.

RELATED: mtg-grofw (slot05 reload-freeze isLegitimateGameEnd fix), mtg-682
(reconnect token / in-game reattach stub — this issue COMPLETES it), mtg-752
(BufferedFact catch-up protocol / ActionLog), mtg-176 (network tracking),
mtg-212/218 (reveal/hidden-info). Refs: docs/NETWORK_ARCHITECTURE.md,
docs/NETWORK_ACTION_LOG.md.
