---
title: 'Local-vs-network gamelog divergence: heuristic Demonic Tutor library-search picks different card on server vs local'
status: closed
priority: 2
issue_type: bug
created_at: 2026-05-30T05:33:11.396423426+00:00
updated_at: 2026-05-30T14:13:25.366753025+00:00
---

# Description

Found by the randomized local-vs-network equivalence fuzz sweep (scripts/fuzz_determinism_netequiv.sh). A REAL network-determinism / information-independence violation of the kind docs/NETWORK_ARCHITECTURE.md calls fatal: the SAME deterministic game (same seed, same decks, same heuristic controllers) produces DIFFERENT gamelogs in local single-process mode vs network (server + 2 loopback clients) mode.

## Reproducer (deterministic, reproduces every run)
    MTG_BIN=target/release/mtg bash scripts/fuzz_determinism_netequiv.sh \
        --invariant equivalence \
        --decks "decks/old_school/01_rogue_rogerbrand.dck decks/old_school/02_thedeck_peterschnidrig.dck" \
        --pair-mode all --start-seed 6 --seeds 1 --controllers heuristic --keep-logs --out debug/repro
Also reproduces at seed 4 (same pair, heuristic, diff=122 lines). Seed 6 diff=27 lines (used for analysis).

## First divergence point (seed 6)
Both modes agree through Gabriel Turn2 M1 Demonic Tutor resolving:
    [GAMELOG Turn2 M1] Demonic Tutor (118) searches Gabriel's library for a Card card and puts it into Hand
Then:
  * NETWORK (server) -- Gabriel immediately casts Mox Sapphire (60).
  * LOCAL -- Gabriel casts NOTHING further on Turn 2.
The network CLIENT shadow log (NetworkLocalController) shows the search RESULT:
    choose_from_library: library_search_names=Some(52), valid_cards.len=0
    choose_from_library: ChoiceAccepted received, library_search_result=Some(60)
i.e. on the SERVER the Demonic Tutor search returns CardId(60)=Mox Sapphire which the heuristic casts; in LOCAL the heuristic tutors a DIFFERENT card. From this single different library-search choice the games diverge completely (different draws/combat/winner).

## Root-cause hypothesis
The heuristic library-search target selection (choose_from_library, the Demonic Tutor unconstrained "search library for any card" path) does not produce the same choice on the server (full library visible) as what is replayed in local mode. "valid_cards.len=0" alongside "library_search_names=Some(52)" in the client log is suspicious (client candidate set differs from what it computes). Same FAMILY as historically-disabled mtg-252 (library search state divergence; client did not know which CardId the server chose), which ChoiceAccepted.library_search_result was meant to fix -- but for an UNCONSTRAINED tutor with a heuristic chooser local and network still disagree.

## Heavy-sweep census (partial, 24 of 216 combos before manual stop)
chain pair-mode, 6 seeds, controllers {heuristic,random} over old-school decks:
  * random   : 11 PASS, 1 FAIL  (FAIL = NETWORK HANG "network timeout (c1done=1 c2done=0)" on 02_thedeck vs 03_robots seed=3: one client got GameEnded, the other never did)
  * heuristic:  7 PASS, 5 FAIL  (4 gamelog-content divergences + 1 hang "c1done=0 c2done=0" on 02_thedeck vs 03_robots seed=2)
TWO network failure modes: (1) heuristic library-search/Demonic-Tutor gamelog divergence (above); (2) a network HANG where one/both clients never receive GameEnded (seen on 02_thedeck vs 03_robots with BOTH heuristic and random). The hang is a SILENT stall -- server --network-debug printed no FATAL SYNC line -- so it may be an undetected desync.

## NOT a determinism bug
Both identically-seeded LOCAL runs are byte-identical and both NETWORK runs are byte-identical. The divergence is LOCAL-choice vs NETWORK-choice for the same logical game -> information-independence/sync bug, per CLAUDE.md "Controllers must be information-independent". The determinism invariant is rock solid: 2448/2448 combos clean (153 deck pairs x 8 seeds x {heuristic,random}).

## Relationship to Java Forge
Forge-Java runs a single authoritative game model (no separate client shadow game replaying server choices) so it cannot exhibit a local-vs-network search-result divergence; this is a Rust-reimplementation-specific invariant (two-store ActionLog<T> netarch) that must hold for the deterministic-sequential-simulation network model. Demonic Tutor MTG rules (CR 701.23 search) are mode-independent; the bug is purely in how the Rust engine replays the heuristic search choice over the network.

RESOLVED 2026-05-30 (commit on fix-mtg-yulth-heuristic-tutor-desync): the shadow client scored Demonic Tutor candidates by name against an EMPTY game.card_definitions map (init_game_reserve_only never populated it), so it panicked / picked a different index than the full-info server. Fix: (1) HeuristicController::choose_from_library_by_names scores each public CardDefinition by name with the same evaluate_card_definition_for_library + strict > first-max tiebreak as server choose_from_library, over the index-aligned name list; (2) client populates game.card_definitions from both public deck lists at init via shared GameInitializer::populate_card_definitions (identical to server). Verified byte-identical local==network gamelogs: seed 6 PASS, seeds 1-10 PASS=10/10, bounded slice 5x stable. tests/fuzz_determinism_netequiv_e2e.sh equivalence sweep now includes heuristic as the regression guard. MTG rules review: PASS (CR 701.23 search / CR 401.2 hidden library; reads only public names+definitions).
