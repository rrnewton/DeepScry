---
title: 'Network desync detection: choice_seq<->action_count<->hash misalignment between WASM shadow and server'
status: open
priority: 2
issue_type: bug
created_at: 2026-06-03T23:48:25.882492954+00:00
updated_at: 2026-06-04T00:25:38.488941204+00:00
---

# Description

Network desync detection: choice_seq<->action_count<->hash misalignment between WASM shadow and server.

Found during mtg-mb668 class-A snapshot verification (commit 54a246d4). The browser desync-DETECTION bookkeeping is misaligned between the WASM shadow and the server: the per-choice (seq, action_count, hash) the server reports in its mismatch box does NOT correspond to the WASM's submission for that seq.

ORIGINAL EVIDENCE (robots seed 2; P1=WASM since NativeAI=player 0):
- Server-rejected P1 seq=175 client_hash=6a046cea @ ac=950. seq 173/174 hashes differ WASM_SUBMIT vs SRV_P1_RECV. WASM shadow ac maxes 861 vs server 950.

=========================================================================
UPGRADE 2026-06-03 (slot03): CONFIRMED REAL + BLOCKING (not benign). This is the
class-A seed-2 residual. The prior "shadow skips ~89 reserved-id actions
(branch-on-absence)" framing is WRONG — see below.

DECISIVE EVIDENCE (robots seed 2, fresh run w/ MTG_NET_FULL_UNDO_DUMP=1):
1. CONTROLLER-level lockstep is BYTE-PERFECT. The server's player=1 NetworkController
   SERVER_FULL_UNDO_DUMP and the WASM shadow's WASM_FULL_UNDO_DUMP align EXACTLY on
   (choice_seq -> action_count): seq 172->821, 173->824, 174->828, 175->831, 176->834,
   177->842, 178->848, 179->861 on BOTH sides. No drift, no skip. The shadow is NOT
   behind at the controller level.
2. All pre-resolution P1 hashes MATCH (SRV_P1_RECV seq 172/173/174 = 16728ac4 /
   72e7d101 / 55c125fa, client==server).
3. The Timetwister reserved-id EFFECT PRIMITIVE is correct: new native oracle
   shadow_timetwister_mass_shuffle_draw_matches_golden_mb668_seed2 (in
   actions/tests/netarch_reserved_zone.rs) runs ChangeZoneAll [Hand,Graveyard]->Library
   shuffle=true + draw7 on golden vs reserved-opponent shadow and is GREEN (library
   counts, RNG state, drawn ids all match). So sig-2c already moves reserved Hand AND
   Graveyard cards; the move/shuffle/draw lockstep is NOT the bug.
4. THE PAIRING IS CROSSED. The fatal "P1 seq=175 action_count=950" maps to the ONLY
   seq=175/ac=950 dump in the whole game, which is the player=0 (P0/NativeAI)
   controller's POST-resolution request. The P1 controller has NO request at ac=950
   (its max is seq=179 @ ac=861). So P1's stale hash (computed @ its local ac=861,
   Timetwister cast+on-stack, UNRESOLVED) is validated against the server's hash @
   ac=950 (Timetwister RESOLVED). compute_view_hash includes action_count + zone sizes
   + stack, so 861-state vs 950-state necessarily differ -> fatal.

INTERPRETATION: between P1's seq=179 pass (ac=861) and P0's seq=175 priority (ac=950)
the SERVER resolves Timetwister entirely server-side (~89 actions, no intervening
network choice). The WASM shadow, being the OPPONENT's client, is NOT asked for a
choice across that span, so its last submitted hash is the pre-resolution one @861. The
server's validation compares that stale P1 hash against its post-resolution @950 hash.
Either (a) the coordinator pairs P1's response with the wrong (P0/post-resolution)
server_state_hash, or (b) the WASM shadow must fast-forward-replay through the
opponent's mass resolution (861->950) before its hash is validated and currently does
not. Both are NETWORK-LAYER (server coordinator validation OR wasm/network replay), NOT
reserved-id effect bugs.

SCOPE: this is the dominant seed-2 blocker. robots seeds {5,6,9,11,18,19,20} are very
likely the same Timetwister/Wheel mass-resolution-between-choices family. mtg-mb668
class-A CANNOT go green until this is fixed. Diagnostics in tree (network_debug-gated):
SERVER_FULL_UNDO_DUMP (network/controller.rs, gate MTG_NET_FULL_UNDO_DUMP=1),
WASM_FULL_UNDO_DUMP (wasm/network/local_controller.rs), SRV_P1_RECV (network/server.rs
~2481). Related: mtg-mb668, mtg-725.

=========================================================================
NEXT-STEP LOCALIZATION 2026-06-03 (slot03) — for whoever takes the fix:

The WASM shadow runs its OWN GameLoop::run_until_input (wasm/network/ai_harness.rs)
with a WasmRemoteController for the opponent + the local controller for itself.
Per local_controller.rs:178-180 the client ECHOES the server's action_count
("local WASM game state doesn't actually execute server actions, so
view.action_count() would be wrong"). BUT compute_view_hash (state_hash.rs:415)
hashes view.action_count() — the LOCAL one — plus zone sizes + stack. So the
echo masks a local_ac != server_ac gap that the hash then catches.

MECHANISM (seed 2): the shadow's game loop blocks at local_ac=861 (P1 priority,
Timetwister cast + ON STACK, unresolved) and submits hash@861. The server, for the
corresponding P1 validation, has already resolved Timetwister server-side
(ac=950) — the ~89-action resolution happens between two choice points with no
intervening P1 NETWORK choice. The shadow's single current_choice_request slot
(client.rs:260) / frontier gate (has_opponent_choice / is_choice_actionable,
client.rs:1058-1069, "K > frontier => NeedsInput") lets the local P1 controller
answer while the game-loop replay frontier is still at 861, so it submits a
hash@861 that the server validates against its @950 state.

TWO CANDIDATE FIXES (pick after a trace):
 (A) WASM side (wasm/network/* — slot03 territory): do NOT let the local
     controller submit until the replay frontier has advanced through the
     opponent's mass-resolution to the request's action_count (gate the local
     submit on request.action_count <= local frontier, mirroring the
     opponent-choice "K > frontier => NeedsInput" rule). I.e. the shadow must
     fast-forward-replay 861->950 BEFORE answering.
 (B) hash side (state_hash.rs — shared): action_count is the one field the shadow
     legitimately can't match during opponent-only resolutions; but removing it
     alone does NOT fix this seed (stack/hand/lib SIZES also differ at 861 vs
     950), so (A) is the real fix; (B) is at most a defense-in-depth simplification.

CONCRETE NEXT STEP: add a targeted trace (network_debug) logging, at the local
controller submit, BOTH the request.action_count AND the replay frontier AND
whether has_opponent_choice() is true — confirm the submit fires while frontier <
request.action_count, then gate it. Repro: node web/test_network_gui_e2e.js --deck
decks/old_school/03_robots_jesseisbak.dck --seed 2 (with MTG_NET_FULL_UNDO_DUMP=1
for the server tail). Diagnostics already in tree. Owner TBD (overlaps slot02/04
network coordinator); escalated to team-lead.
