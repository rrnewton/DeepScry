---
title: 'netarch Phase 2 steps 3-4: native parity + engine choose_at unification'
status: open
priority: 2
issue_type: task
created_at: 2026-05-30T04:45:39.533970539+00:00
updated_at: 2026-05-30T04:45:39.533970539+00:00
---

# Description

Continuation of the network ActionLog<T> rearchitecture (PR #12 / docs/NETWORK_ACTION_LOG.md + NETWORK_ACTION_LOG_MIGRATION.md). PR #12 landed Phase 1 (generic ActionLog<T> primitive, two-store ownership design, 11 invariants) + Phase 2 steps 1-2 (WASM reveal/reorder via ActionLog<StateSyncEntry>; WASM opponent choices via ActionLog<ChoiceEntry>). Independent architecture review verdict: MERGE_OK_WITH_FOLLOWUP — clean, strictly-better, no dueling codepaths (6/14 DELETE-table paths gone, all WASM-side; native untouched). The REMAINING work, tracked here:

1. **Native parity (migration steps 3-4).** Remove the 8 surviving DELETE-table paths, ALL native-side: SharedNetworkState::drain_reveals_up_to / drain_all_reveals / drain_all_reveals_if_ready / wait_for_library_reorders / drain_all_library_reorders, the pending_reveals/pending_library_reorders VecDeques, library_reorder_condvar, choice_pending AtomicBool, take_choice_accepted_for_seq / take_remote_choice. Highest priority: wait_for_library_reorders is a timeout-BLOCK that NETWORK_ARCHITECTURE.md explicitly forbids. Migrate native to the same ActionLog shape as WASM. Until done, design invariant #10 (same primitive native+WASM) is unmet.

2. **Engine choose_at unification (the real architectural debt).** Realise Controller::choose_at(view, action_count) + GameState::apply_state_sync_at(K) as the uniform engine API. Retire the WASM pop_opponent_choice + next_opponent_choice_cursor FIFO shim and the OpponentChoiceData re-materialisation (network/client.rs:939-991, 75-92). This is what finally kills the get_controller_type()==Remote special-casing in game/game_loop/network_choice.rs (lines 68,132,171,209) and delivers design invariant #9 (engine controller-agnostic).

3. **Perf nit (low priority).** apply_state_sync_up_to_frontier clones every windowed entry out of the log (network/client.rs:1046-1051, entry.clone()) to dodge the borrow checker. Fine at <=1e4 entries but violates OPTIMIZATION.md no-clone; use split-borrow or apply-by-index.

4. **Wire-protocol step 5 (optional, enables stronger asserts).** WASM state_sync currently uses a synthetic local counter (next_state_sync_ac, client.rs:1009-1012) for reveal/reorder action_counts (client-derived, not server-validated). Promote CardRevealed/LibraryReordered to carry a server-authoritative action_count (the choice buffer already uses the real wire action_count). Enables the in-data state-hash desync assert the design calls for.

Source: architecture review 2026-05-29 on PR #12 HEAD. Each item should land with mtg-rules-review where it touches gameplay semantics, and a regression test proving native-vs-WASM + local-vs-network gamelog identity.
