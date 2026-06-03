---
title: 'Network desync detection: choice_seq<->action_count<->hash misalignment between WASM shadow and server'
status: open
priority: 2
issue_type: bug
created_at: 2026-06-03T23:48:25.882492954+00:00
updated_at: 2026-06-03T23:48:25.882492954+00:00
---

# Description

Found during mtg-mb668 class-A snapshot verification (commit 54a246d4). The browser desync-DETECTION bookkeeping is misaligned between the WASM shadow and the server: the per-choice (seq, action_count, hash) the server reports in its mismatch box does NOT correspond to the WASM's submission for that seq.

EVIDENCE (robots seed 2; P1=WASM since NativeAI=player 0):
- Server-rejected P1 seq=175 client_hash=6a046cea @ ac=950. WASM's OWN submit (WASM_SUBMIT, logged at the ClientMessage::SubmitChoice build site) for seq=175 = hash=0d60bb6a @ ac=831. 6a046cea is NEVER produced by the WASM on any path.
- seq 173/174 hashes differ: WASM_SUBMIT a078a280/7179b30c vs SRV_P1_RECV 72e7d101/55c125fa.
- WASM per-request action_count (831 for seq 175) != server expected (950); WASM shadow ac maxes 861 vs server 950.

The ~89 action_count GAP itself is REAL+trustworthy (shadow skips ~89 reserved-id actions = the class-A branch-on-absence = mtg-mb668 fix target). It is the seq<->hash CORRELATION the detection uses that is suspect.

RESOLUTION PROTOCOL: resolves itself once class-A action-skips are fixed. Post-fix, re-run the browser sweep: shadow ac == server ac AND seed flips GREEN => bookkeeping was benign. Matching action_counts + matching state but STILL desync => this misalignment is real+blocking, fix before trusting green. Do NOT chase now; let the post-fix sweep decide. Diagnostics in tree (network_debug-gated): WASM_SUBMIT (wasm/network/client.rs), SRV_P1_RECV (network/server.rs), WASM_CARD_DETAIL (wasm/network/local_controller.rs). Related: mtg-mb668.
