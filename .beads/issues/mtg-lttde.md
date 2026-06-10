---
title: 'Card Compatibility: Diamond Valley'
status: closed
priority: 3
issue_type: task
created_at: 2026-06-10T22:23:00.216875747+00:00
updated_at: 2026-06-10T22:23:04.073057514+00:00
---

# Description

Test all behavioral aspects of Diamond Valley in MTG Forge-rs.

Card: cardsfolder/d/diamond_valley.txt
Set: 1994 World Championship pool (mtg-709 umbrella; root-cause backlog mtg-713 B10)
Test puzzle: test_puzzles/diamond_valley_sacrifice_lifegain.pzl

Card text:
  (Land, no mana cost)
  {T}, Sacrifice a creature: You gain life equal to the sacrificed creature's toughness.

Script:
  A:AB$ GainLife | Cost$ T Sac<1/Creature> | LifeAmount$ X
  SVar:X:Sacrificed$CardToughness

Findings (2026-06-10_#3151(f5bf81eb), compat-1994-wave10):

1. [x] Parses as a Land with one activated ability.
2. [BROKEN→FIXED] Activated GainLife with dynamic LifeAmount$ X = Sacrificed$CardToughness:
   - The GainLife effect converter only accepted an INTEGER LifeAmount$
     (params.get_i32("LifeAmount").ok()?), so "X" -> Sacrificed$CardToughness
     returned None and the WHOLE activated ability was silently dropped — never
     offered, no life gained.
   - Root cause class: effect-converter hardcoding (mtg-713 B10).
   - Fix (5 parts, all rewind-safe, public-state-only):
     a. New DynamicAmount::SacrificedToughness variant + parse of
        Sacrificed$CardToughness (core/effects.rs).
     b. params_to_effect_with_svars routes a non-fixed activated GainLife through
        Effect::GainLifeDynamic (loader/effect_converter.rs).
     c. The creature sacrificed to pay the cost is recorded in
        SubActionScratch::sacrificed_for_cost during cost payment
        (mana_payment/payment_execution.rs SacrificePattern handler).
     d. The activated-ability resolution loop fills the GainLifeDynamic recipient
        (controller) and SacrificedToughness reference (the sacrificed creature)
        from that scratch (game_loop/priority.rs), then clears it after effects run.
     e. resolve_dynamic_amount reads the sacrificed creature's last-known toughness
        (CR 608.2g — it already left the battlefield), clamped >= 0 (CR 119.4).
   - Rewind safety: activated abilities resolve IMMEDIATELY (no choice/priority
     boundary between cost payment and the effect, see priority.rs TODO mtg-70),
     so the #[serde(skip)] scratch is provably None at every serialize/choice
     boundary — same pattern as current_damage_source. On rewind+replay the same
     sacrifice re-runs and re-populates it identically.

Reproducer:

```sh
./target/release/mtg tui --start-state test_puzzles/diamond_valley_sacrifice_lifegain.pzl \
  --p1 fixed --p1-fixed-inputs 'activate diamond valley;pass;pass;pass;pass;pass;pass;pass;pass;pass' \
  --p2 fixed --p2-fixed-inputs 'pass;pass;pass;pass;pass;pass;pass' --seed 42 --verbosity 3
```

Expected log evidence:

```
  Diamond Valley activates ability: You gain life equal to the sacrificed creature's toughness.
  Hill Giant (4) goes to graveyard
  Player 1 gains 3 life (life: 23)
```

Unit test: test_card_compat_diamond_valley in mtg-engine/src/game/actions/tests/effects.rs
E2E test:  test_diamond_valley_sacrifice_lifegain in mtg-engine/tests/puzzle_e2e.rs

Known follow-up (not blocking WORKING for this puzzle): the SacrificePattern
cost handler auto-selects WHICH creature to sacrifice rather than asking the
controller (pre-existing TODO in payment_execution.rs, independent of this fix).
For a single-creature board (the common Diamond Valley case) this is exact.

CARD STATUS: WORKING — activated sacrifice-lifegain gains life = sacrificed creature's toughness.
