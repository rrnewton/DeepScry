---
title: 'Card Compatibility: Howling Mine'
status: closed
priority: 3
issue_type: task
created_at: 2026-06-10T18:55:00.157659961+00:00
updated_at: 2026-06-10T18:55:05.857889649+00:00
---

# Description

Card Compatibility tracking for Howling Mine (1994 World Championship compat — under mtg-709 / mtg-713 B11).

Oracle: "At the beginning of each player's draw step, if Howling Mine is untapped, that player draws an additional card."
Script: cardsfolder/h/howling_mine.txt
  T:Mode$ Phase | Phase$ Draw | ValidPlayer$ Player | TriggerZones$ Battlefield | PresentDefined$ Self | IsPresent$ Card.untapped | Execute$ TrigDraw
  SVar:TrigDraw:DB$ Draw | Defined$ TriggeredPlayer

CARD STATUS: WORKING (2026-06-10_#3135 region)

== Findings (2026-06-10) ==
[x] Triggered ability (Phase$ Draw, each player): fires on EACH player's draw step (ValidPlayer$ Player → controller_turn_only=false). WORKING.
[x] TriggeredPlayer routing: the extra draw goes to the active player whose draw step fired (Defined$ TriggeredPlayer → DrawCards routed via TriggerContext::drawing_player), NOT Howling Mine's controller. Fixed earlier (commit c8a9059b, B11 routing). Verified again this session.
[x] Intervening-if 'if untapped' (CR 603.4): NEWLY FIXED this session. IsPresent$ Card.untapped is now parsed into Trigger::present_self_condition = PresentSelfCondition::Untapped and enforced in BOTH trigger-fire paths (check_triggers_for_controller in actions/mod.rs and check_phase_triggers in game_loop/steps.rs). A TAPPED Howling Mine grants NO extra draw. Previously the parser only modeled counters_… self-conditions, so a tapped Howling Mine still wrongly drew.
[x] Parses with cost {2}, Artifact type. WORKING.
[N/A] Casting/alt-cost/targeting at cast: vanilla artifact, no targeting.
[N/A] Activated abilities: none.
[N/A] Replacement effects: none (the trigger is a triggered ability, not a replacement).

== Evidence (real mtg tui game log) ==
Untapped (each player draws extra on own draw step):
  mtg tui --start-state test_puzzles/howling_mine_each_player_draws.pzl --p1 fixed --p2 fixed --p1-fixed-inputs '*' --p2-fixed-inputs '*' --seed 42 --verbosity 3
  → 'Player 2 draws Island' then 'Howling Mine trigger effect' then 'Player 2 draws Island' (P2's step); 'Player 1 draws Plains'+trigger+'Player 1 draws Plains' (P1's step).

Tapped (no extra draw until untap):
  mtg tui --start-state test_puzzles/howling_mine_tapped_no_draw.pzl --p1 fixed --p2 fixed --p1-fixed-inputs '*' --p2-fixed-inputs '*' --seed 42 --verbosity 3
  → first draw step: 'Howling Mine (3) (tapped)' and NO 'Howling Mine trigger effect' (only the normal draw). After untap step, later draw steps fire the trigger.

== Tests ==
- mtg-engine/src/game/actions/tests/spell_casting.rs::test_howling_mine_trigger_parse_shape (parser shape: BeginningOfDraw, not controller-only, Untapped intervening-if, TriggeredPlayer draw).
- mtg-engine/tests/puzzle_e2e.rs::test_howling_mine_draws_for_active_player (routing).
- mtg-engine/tests/puzzle_e2e.rs::test_howling_mine_tapped_no_extra_draw (tapped → no draw).
- Puzzles: test_puzzles/howling_mine_each_player_draws.pzl, test_puzzles/howling_mine_tapped_no_draw.pzl
