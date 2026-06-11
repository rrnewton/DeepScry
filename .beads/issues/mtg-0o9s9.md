---
title: 'NETARCH desync: triggered-ability Dig (Thundertrap Trainer ETB) bypasses controller-routed selection -> server/shadow diverge'
status: open
priority: 2
issue_type: bug
depends_on:
  mtg-415: related
  mtg-677: related
  mtg-708: related
created_at: 2026-06-11T03:29:59.237011110+00:00
updated_at: 2026-06-11T03:29:59.237011110+00:00
---

# Description

ROOT CAUSE PINNED (slot02, branch claude/netarch-cont, off integration 50d1c039). A native-vs-WASM network game of WC2025 decks (P1 native = decks/championship/2025/04_henry_temur_otters.dck, P2 browser = 02_shibata_izzet_lessons.dck) hits a server/shadow + rewind/replay divergence on Thundertrap Trainer's ETB Dig.

== SYMPTOM (byte-pinned from the repro) ==
On the SAME action (Thundertrap Trainer (57) ETB trigger resolving), the two engines disagree:
  SERVER (authoritative): "looks at the top 4 cards", "puts Roaring Furnace into Hand", "puts 3 cards on the bottom of their library"
  SHADOW (native client): "looks at the top 4 cards", "puts 4 cards on the bottom of their library" (0 to hand)
The shadow selected NOTHING to put into hand while the server selected Roaring Furnace -> hand/library zone contents + counts diverge -> later state-hash / rewind-replay FATAL. This matches the audit report's "forward puts N on the bottom vs replay puts card#X into Hand" LogMismatch class (same Dig, opposite branch).

== CARD ==
cardsfolder/t/thundertrap_trainer.txt:
  T:Mode$ ChangesZone | ... | Execute$ TrigDig | TriggerDescription$ When this creature enters, look at the top four cards of your library. You may reveal a noncreature, nonland card from among them and put it into your hand. Put the rest on the bottom of your library in a random order.
  SVar:TrigDig:DB$ Dig | DigNum$ 4 | ChangeNum$ 1 | Optional$ True | ForceRevealToController$ True | ChangeValid$ Card.nonCreature+nonLand | RestRandomOrder$ True

== ROOT CAUSE (definitive) ==
There are TWO Dig implementations:
  (A) NETWORK-SAFE, controller-routed: game_loop/priority.rs ~3416-3604 (the resolve_top_spell_with_discard_hook path). It calls choose_from_library_with_hook(controller, digger, &valid_ids) per pick and log_choice_point(...) so the shadow's RemoteController REPLAYS the server's authoritative CardId. Used for SPELL digs (Impulse, Seismic Sense - mtg-415) and tutors (mtg-589).
  (B) NETWORK-UNSAFE, local heuristic: actions/mod.rs ~5667-5700 (execute_effect's Effect::Dig). It ranks candidates with self.dig_card_score(...) and, for Optional digs, SKIPS when best_score < 30. Uses FULL information on the server but only the SHADOW's (reserved/unrevealed top cards) on the client -> different selection -> desync. Direct violation of "controllers must be information-independent" (CLAUDE.md / NETWORK_ARCHITECTURE.md).

The routing gate that decides A vs B is game_loop/priority.rs:3119-3137 resolve_top_spell_from_stack_interactive:
  let needs_interactive = card.effects.iter().any(|e| ... matches!(e, Effect::Dig { target_self: true, .. }) ...);
It inspects card.effects (the stack object's OWN base effects). For a SPELL whose Dig is a top-level effect this matches and routes to (A). But Thundertrap's Dig is a TRIGGERED ABILITY effect: it lives in the ability (Execute$ TrigDig -> SVar:TrigDig:DB$ Dig), NOT in card.effects. So for the ETB-trigger stack item, needs_interactive scans the CREATURE's base effects (no Dig present), returns FALSE, and the trigger resolves via the non-interactive resolve_top_spell_from_stack -> execute_effect -> path (B), the diverging heuristic.

target_self IS true for Thundertrap (effect_converter.rs:1204-1207: no Defined$ => target_self=true), so the ONLY reason it misses the safe path is that needs_interactive looks at the wrong effect list for a triggered ability.

== FIX DIRECTION (not yet implemented - desync-critical + overlaps mtg-245) ==
Make the A-vs-B routing detect an interactive Dig (and the other controller-routed effect classes) in a stack object's RESOLVED ability effects, not just card.effects - i.e. when the stack item is a triggered/activated ability, scan the ability's effects (the Execute$/SVar-expanded DB$ Dig) for the needs_interactive predicate and route its resolution through the controller-routed Dig path so the shadow replays the server's choice. Equivalent alternative: make execute_effect's Effect::Dig itself controller-aware (route the selection through choose_from_library) so path (B) stops being a local-heuristic island - but that is the larger execute_effect rework currently owned by mtg-245, so coordinate.

Litmus for the fix: rewind-to-turn-start + replay must reproduce bit-identical, and the WC2025 04v02 native-vs-WASM game must replay cleanly past the Thundertrap ETB.

== REPRO (deterministic) ==
Two-deck harness (gitignored): debug/repro_wc2025_04v02.js (a copy of web/test_network_gui_e2e.js parameterised --deck1/--deck2; native P1=deck1, browser P2=deck2, server --network-debug, replay verifier on). Run:
  NODE_PATH=web/node_modules node debug/repro_wc2025_04v02.js --seed 1
seed 1 shows the Thundertrap divergence directly in the interleaved server/native logs; the deck-pair also desyncs on several other seeds (turn-7 turn-start-hash @seed 42, server-vs-client state-hash @seed 2) - likely the SAME Dig root or adjacent reveal-timing classes.

Refs: mtg-708 audit item #2 (library-reorder / Dig "rest to bottom" undo-log + selection holes - names both Dig sites), mtg-415 (target_self Dig pre-reveal+sync, the safe path), mtg-589 (tutor controller-routing), mtg-677 (netarch rewind/replay PRIMARY), mtg-245 (execute_effect refactor - overlapping owner). Does NOT touch slot05's reveal_processor/client LOG-suppression work.
