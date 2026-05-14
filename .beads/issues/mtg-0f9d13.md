---
title: 'Deck Compatibility: thedeck (02_thedeck_peterschnidrig.dck)'
status: open
priority: 2
issue_type: task
created_at: 2026-05-13T19:18:20.391758180+00:00
updated_at: 2026-05-13T19:18:20.391758180+00:00
---

# Description

Track compatibility of all cards in the Old School 93/94 'The Deck' (Peterschnidrig) deck — the canonical control deck of the format.

**Files:**
- decks/old_school/02_thedeck_peterschnidrig.dck
- decks/old_school/02_thedeck_peterschnidrig.txt
- decks/rn_os/02_thedeck_peterschnidrig.txt

**Set References:**
- mtg-3c7c63 (LEA - Limited Edition Alpha)
- mtg-997323 (ARN - Arabian Nights)
- mtg-07ff20 (ATQ - Antiquities)

**Sister deck tracking issues:**
- mtg-526f25 rogue_rogerbrand (template/baseline)

---

## Deck list (60 main + 15 sideboard)

### Main (60)

#### Creatures (10)
- 4 Savannah Lions (LEA) — vanilla 2/1 for {W}
- 4 Su-Chi (ATQ) — 4/4 artifact creature for {4}, "When ~ dies, add {C}{C}{C}{C}" (death trigger)
- 2 Serra Angel (LEA) — flying + vigilance (mtg-c6dfe3 already filed; status open)

#### Removal / Burn (6)
- 4 Swords to Plowshares (LEA) — exile creature, controller gains life equal to power. **KNOWN BUG: mtg-1ad808** (exiles but does not grant life)
- 2 Psionic Blast (LEA) — 4 damage to any target + 2 to you. **KNOWN BUG: DamageResolve effect not implemented in Rust engine** (\`grep -r DamageResolve mtg-engine/src\` returns no hits). The card script SubAbility chain ends with \`DB\$ DamageResolve\`, which currently resolves as an unknown no-op (cf. mtg-19a6ee pattern). May result in unresolved damage map / missing lifelink/replacement triggers. Per-card bd issue TODO.

#### Artifact Removal (4)
- 4 Disenchant (LEA) — destroy target artifact or enchantment

#### Counters (7)
- 4 Counterspell (LEA) — counter target spell
- 1 Mana Drain (LEG) — counter target spell, add {C}{C}{C}{C} equal to its CMC at next main phase. Requires DELAYED mana ability — may not be supported.
- 1 Flash Counter (mostly LEG) — counter target instant. Likely vanilla counterspell variant; should work if Counterspell works.

#### Card Draw / Tutors / Combo (8)
- 1 Ancestral Recall (LEA) — draw 3 / target player draws 3
- 1 Recall (ARN) — return X cards from graveyard to hand by discarding/exiling cards. Complex — likely BROKEN.
- 1 Demonic Tutor (LEA) — search library, put a card into hand
- 1 Mind Twist (LEA) — target player discards X cards at random. **Random-discard requires controller hidden-information masking** — risk of info-leak bug like the one fixed in c19429a5 (normal draws).
- 1 Braingeyser (LEA) — target player draws X
- 1 Balance (LEA) — equalize lands/creatures/cards in hand to lowest. Multi-step Balance effect; partly tested per mtg-oewn8 (closed: log-multiplicity bug). Status TBD.
- 1 Time Walk (LEA) — extra turn. **WORKING** (mtg-fd5bf7 closed; infinite-loop mtg-aeee14 closed).
- 1 Chaos Orb (LEA) — physical-flip destroy. **PARTIAL: mtg-ad79fd open** (only self-destroys; opponent permanent NOT destroyed).

#### Mana Artifacts (7) — Power 9 artifacts + Sol Ring
- 1 Sol Ring (LEA) — {T}: add {C}{C}. Pattern matches Mox cycle (mtg-fa9c28 verified WORKING).
- 1 Mox Jet (LEA) — **WORKING** (mtg-fa9c28 closed)
- 1 Mox Sapphire (LEA) — same pattern, expected WORKING (per mtg-fa9c28 generalisation note)
- 1 Mox Pearl (LEA) — same pattern, expected WORKING
- 1 Mox Emerald (LEA) — same pattern, expected WORKING
- 1 Mox Ruby (LEA) — same pattern, expected WORKING
- 1 Black Lotus (LEA) — **NEW BUG SUSPECT**: card has \`Produced\$ Any | Amount\$ 3\` (three mana of any ONE color) plus a \`T Sac<1/CARDNAME>\` cost. The Mox cycle uses fixed \`Produced\$ B\` etc; Black Lotus needs:
  1. Choose-color step before producing (cf. Effect::ChooseColor)
  2. Amount\$ 3 — multi-mana add
  3. Sac<1/CARDNAME> as cost (similar to Strip Mine, which is closed/working)
  Suspected status: PARTIAL/BROKEN. Needs verification via puzzle. Per-card bd issue TODO.

#### Lands (18)
- 4 Mishra's Factory (ATQ) — animate-into-2/2 artifact creature with Assembly-Worker pump. Complex animate. Per-card bd issue TODO; may be PARTIAL/BROKEN.
- 2 City of Brass (ARN) — mtg-ef504b open. {T}: add one mana of any color, takes 1 dmg when tapped.
- 1 Library of Alexandria (ARN) — {T}: draw if you have 7 cards in hand. Conditional triggered ability — likely needs custom card support.
- 1 Strip Mine (ATQ) — mtg-0e702a closed (verified working) and mtg-36d76b open follow-up.
- 1 Island, 2 Plains (basic lands)
- 4 Tundra (LEA) — Plains/Island dual, no drawback. Should work.
- 1 Scrubland (LEA) — Plains/Swamp dual.
- 3 Underground Sea (LEA) — Island/Swamp dual.

### Sideboard (15)
- 4 Divine Offering (LEA-ish) — destroy artifact, gain life equal to CMC. Per-card issue TODO.
- 1 Spirit Link (LEG) — aura: lifelink-by-trigger. Pre-modern lifelink (triggered, not keyword). Per-card issue TODO.
- 2 City in a Bottle (ARN) — anti-Arabian-Nights hate card. Set-specific filter. Likely UNSUPPORTED, per-card issue TODO.
- 2 Maze of Ith (ARN) — untap attacking creature, prevent its damage. Combat-replacement ability. Per-card issue TODO.
- 1 Wrath of God (LEA) — destroy all creatures, no regen. Mass-removal. Per-card issue TODO.
- 3 Blue Elemental Blast (LEA) — counter red spell OR destroy red permanent (modal). Per-card issue TODO.
- 1 Circle of Protection: Red (LEA) — pay {1}: prevent damage from one red source. Per-card issue TODO.
- 1 Power Sink (LEA) — counter unless they pay {X} (additional-cost counter). Per-card issue TODO.

---

## Top priority gaps highlighted by this deck

1. **DamageResolve effect** missing from Rust engine — blocks Psionic Blast and any other card whose subability chain ends with \`DB\$ DamageResolve\`. **High priority** — this is the issue specifically called out in the originating tg task.
2. **Black Lotus** — Amount\$ 3 + Produced\$ Any + Sac<1/CARDNAME> combination needs verification. Power-9 mana is a defining feature of this deck; if Black Lotus is broken, 'The Deck' cannot be playtested faithfully.
3. **Mind Twist** — random discard from opponent's hand must NOT leak hidden information to controllers (cf. c19429a5).
4. **Mishra's Factory** — animate-land → 2/2 attacker is a key win condition; needs verification.
5. **Library of Alexandria** — conditional draw at 7-cards is the deck's iconic engine; almost certainly missing.

## Existing related per-card / per-bug issues

- mtg-fa9c28 [closed] Mox Jet — WORKING
- mtg-c6dfe3 [open]   Serra Angel
- mtg-ef504b [open]   City of Brass
- mtg-0e702a [closed] Strip Mine — WORKING
- mtg-36d76b [open]   Strip Mine (follow-up)
- mtg-ad79fd [open]   Chaos Orb (PARTIAL)
- mtg-4c1696 [closed] Chaos Orb FlipOntoBattlefield always self-targets
- mtg-1ad808 [open]   Swords to Plowshares exiles but does not grant life
- mtg-aeee14 [closed] Time Walk infinite extra turns loop
- mtg-fd5bf7 [closed] ExtraTurn effect (Time Walk)

## Notes

- Worktree: reuse mtg-forge-rs-compat if present (per mtg-526f25 convention); else create on demand.
- Baseline commit: 2d8d77 (origin/integration tip at task start).

---

## Status updates

### Mana resolver: prefer cheap sources, sacrifice last (fix-mana-sacrifice-ordering)

The mana engine now ranks sources by a side-cost severity score before
choosing what to tap. Order (cheapest → most expensive):

1. **Plain free sources** — basic lands, Moxen, dual lands. `ManaSideCost::None`.
2. **Utility lands** — Mishra's Factory, Strip Mine, Mutavault. Free to tap
   for mana but their other (non-mana) ability slot is consumed, so prefer
   plain lands first. `ManaSideCost::Utility`.
3. **Pain lands** — City of Brass, Mana Confluence. `ManaSideCost::PayLife(n)`,
   weighted linearly by `n`.
4. **Sacrifice sources** — Black Lotus, Lotus Petal, Treasure tokens. Tap
   only when nothing cheaper covers the cost. `ManaSideCost::Sacrifice`.

Code touched:

- `mtg-engine/src/core/mana_production.rs` — new `ManaSideCost` enum + field
  on `ManaProduction`; `side_cost_score()` helper.
- `mtg-engine/src/core/card.rs` — `derive_mana_production_from_abilities`
  inspects each mana ability's `Cost` (Sacrifice / PayLife) and the card's
  *other* activated abilities to decide between None/Utility/PayLife/Sacrifice.
- `mtg-engine/src/game/mana_payment.rs` — `score_for_color` and a new
  `generic_score` bake side-cost into the resolver's priority. The generic
  pip phase now sorts candidates instead of iterating in cache order, which
  fixes the original Mishra's Factory / Black Lotus ordering bugs.

Regression coverage:

- `tests/puzzle_e2e.rs::test_psionic_blast_does_not_waste_black_lotus`
  (e2e, drives the puzzle below via FixedScriptController).
- `puzzles/mana_sacrifice_last.pzl` (also `test_puzzles/mana_sacrifice_last.pzl`)
  with Underground Sea + Tundra + Mox Emerald + Black Lotus + Psionic Blast.
- `mana_payment::tests::test_greedy_resolver_prefers_non_sacrifice_sources`
- `mana_payment::tests::test_greedy_resolver_avoids_lotus_when_duals_suffice`
- `mana_payment::tests::test_greedy_resolver_prefers_basic_over_utility_land`
- `mana_payment::tests::test_greedy_resolver_prefers_basic_over_pain_land`

Reproducer:
```
cargo build --bin mtg --release
./target/release/mtg tui --start-state puzzles/mana_sacrifice_last.pzl \
    --p1 heuristic --p2 zero --seed 42
```
Expected: Psionic Blast taps Underground Sea + Mox Emerald + Tundra; Black
Lotus stays on the battlefield. Pre-fix the resolver could choose any
ordering of the complex sources, including the wasteful Lotus tap.

### Mishra's Factory animate fix (fix-mishras-factory-attacker)

Follow-up patch on top of the resolver fix. The `AB$ Animate` parser was
ignoring `Types$ Artifact,Creature,Assembly-Worker` and
`RemoveCreatureTypes$ True`, so when Mishra's Factory's `{1}: become a 2/2
Assembly-Worker artifact creature` activated ability resolved, the card's
typeline never actually changed. The declare-attackers step's
`card.is_creature() && !card.tapped` filter therefore excluded the
animated Factory, and the manland could never attack.

Code touched:

- `mtg-engine/src/core/effects.rs` — `Effect::SetBasePowerToughness` gains
  `types_added`, `subtypes_added`, `remove_creature_subtypes` fields.
- `mtg-engine/src/loader/effect_converter.rs` — Animate parser reads the
  `Types$` parameter, splitting tokens between known `CardType` variants
  and creature subtypes (anything not a known card type).
- `mtg-engine/src/core/card.rs` — Card gains
  `temp_animate_types`, `temp_animate_subtypes`, `temp_removed_subtypes`
  to track what was added/removed for end-of-turn rollback.
- `mtg-engine/src/game/actions/mod.rs` — `Effect::SetBasePowerToughness`
  resolution now adds the types/subtypes, refreshes
  `definition.cache.update_from_types/subtypes` (so `is_creature()` /
  `is_artifact()` flip immediately), and bumps the per-player
  `ManaSourceCache` if the card is a mana source (Mishra's Factory: simple
  colorless source ↔ complex source as it animates and reverts).
- `mtg-engine/src/game/state.rs` — `cleanup_temporary_effects` rolls back
  the temp types/subtypes at end of turn, refreshes the cache, and
  re-marks the mana caches dirty if any animated mana source reverted.
- `mtg-engine/src/game/game_loop/{priority.rs,logging.rs,mod.rs}` —
  pattern updates + new `get_available_attacker_creatures_for_test` hook.

Regression coverage:

- `tests/puzzle_e2e.rs::test_mishras_factory_animates_and_is_eligible_attacker`
  applies the animate effect directly, asserts the Factory becomes a
  Creature/Artifact with the Assembly-Worker subtype, asserts it shows up
  in `get_available_attacker_creatures`, then runs `cleanup_temporary_effects`
  and asserts the typeline rolls back to land-only.
- Existing `tests/mana_cache_debug_stress_test.rs` (oldschool tournament,
  20 games of 4 decks) now passes — the mana cache invalidation hooks
  prevent the cache desync that the stress test would otherwise flag when
  Mishra's Factory animates mid-game.
- New puzzle: `puzzles/mishras_factory_attacks.pzl` (also in
  `test_puzzles/`) for manual replay.

### Action menu side-cost hints (fix-mishras-factory-attacker)

Closes the second open follow-up: cast actions in the menu now surface
predicted sacrifice / pain costs so the player sees them before
accepting. Renders e.g.
`[2] cast Psionic Blast (sacrificing Black Lotus)` or
`[3] cast Lightning Bolt (1 damage from City of Brass)` when those are
the only payment options the resolver would pick.

Code touched:

- `mtg-engine/src/game/controller.rs` — new `predicted_side_costs_hint`
  helper runs the GreedyManaResolver speculatively against the player's
  current sources, inspects each tap'd source's
  `mana_production.side_cost`, and buckets them into a `(sacrificing X;
  N damage from Y)` parenthetical. Hooked into
  `format_spell_ability_choice` for the `CastSpell` arm.

Regression coverage:

- `tests/puzzle_e2e.rs::test_action_menu_shows_sacrifice_and_pain_hints`
  builds three small in-memory scenarios:
    1. Black Lotus alone + 3-mana spell — hint must contain
       "sacrificing Black Lotus".
    2. Three Forests + Black Lotus + 3-mana spell — hint must NOT
       contain "sacrificing" (resolver picks the Forests).
    3. City of Brass alone + Lightning Bolt — hint must contain
       "damage from City of Brass".

### Open follow-ups (not in this fix)

- **Mishra's Factory + summoning-sickness corner cases**: the current
  patch deliberately doesn't reset `turn_entered_battlefield` when a
  land animates, mirroring Forge-Java's "becomes a creature doesn't
  reset summoning sickness" rule. A standalone test exercising the
  same-turn-played-then-animated case would confirm we don't accidentally
  let a freshly-played Mishra's Factory attack on its own turn.
- **`AnimateAll` doesn't yet take the `Types$` parameter** — same
  pattern as the per-card Animate. Cards that mass-animate land into
  creatures (e.g. some Avatar set "all your lands become 1/1 Elementals"
  effects) still need this. Filed for future work.
