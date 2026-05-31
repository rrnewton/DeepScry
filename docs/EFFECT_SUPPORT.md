# MTG Card-Script Effect / Keyword Support Matrix

This file is the **per-construct** companion to the per-card
`Card Compatibility: <name>` beads issues. A row exists for every
keyword, trigger pattern, activated/static ability shape, replacement
effect, mana-production form, and SVar/cost/selector primitive that
we have evaluated.

See `.claude/skills/compatibility_tracking/SKILL.md` for the full
workflow, row format, and update rules.

## Conventions

- **Status:** `WORKING` / `PARTIAL` / `BROKEN`. PARTIAL means the
  parser/engine handles the common shape but at least one variant or
  qualifier is silently dropped or mis-evaluated.
- **Last verified:** `YYYY-MM-DD_#<gitdepth>(<short>)` per CLAUDE.md
  beads conventions. Bump on every status flip.
- **Bug issue:** beads ID for the open bug, or `(fixed)` /
  `(none)` if working with no outstanding work.
- **Sample cards:** at least one `Card Compatibility:` issue that
  exercises the construct.
- **Append-only.** When a construct flips status, append a new dated
  entry below the row in a sub-bullet rather than rewriting the
  history; the live status is the row's `Status` column.

## Keywords

| Keyword                      | Status   | Last verified                  | Bug issue   | Sample cards          |
|------------------------------|----------|--------------------------------|-------------|-----------------------|
| Flying                       | WORKING  | 2026-05-12_#2226(928ec99f)     | (none)      | Serra Angel           |
| First Strike                 | WORKING  | 2026-05-12_#2226(928ec99f)     | (none)      | Black Knight          |
| Vigilance                    | WORKING  | 2026-05-12_#2226(928ec99f)     | (none)      | Serra Angel           |
| Protection from <color>      | WORKING  | 2026-05-12_#2226(928ec99f)     | (none)      | Black Knight          |
| Regenerate                   | WORKING  | 2026-05-12_#2226(928ec99f)     | (none)      | Sedge Troll           |
| Swampwalk                    | WORKING  | 2026-05-12_#2226(928ec99f)     | (none)      | Sedge Troll           |
| ETBReplacement:Copy (Clone)  | WORKING  | 2026-05-27_#2354(dbf857a7)     | (none)      | Copy Artifact         |
| ETBReplacement ChoosePlayer (as ~ enters, choose an opponent) | WORKING | 2026-05-31_#2546(34f76ca3) | (fixed) | Black Vise |

## Triggers (T:)

| Trigger pattern                                                                  | Status  | Last verified              | Bug issue   | Sample cards     |
|----------------------------------------------------------------------------------|---------|----------------------------|-------------|------------------|
| ChangesZone B→Gy ValidCard$ Card.Self                                            | WORKING | 2026-05-29_#2440(00e22751) | (none)      | Su-Chi, (multiple) |
| ChangesZone B→Gy ValidCard$ Creature.DamagedBy                                   | WORKING | 2026-05-28_#2360(897881c9) | (fixed)     | Sengir Vampire   |
| DamageDone/DamageDealtOnce ValidTarget gating (player-only vs creature vs any) at combat-damage firing site | WORKING | 2026-05-31_#2539(05d48a7b) | (fixed)   | Spirit Link, Hypnotic Specter |
| DamageDone Execute$ DB$ Discard Defined$ TriggeredTarget (damaged player discards)| WORKING | 2026-05-29_#2449(b5fd60b7) | (fixed)     | Hypnotic Specter |
| Phase Upkeep ValidPlayer$ Player Execute$ DB$ Destroy (each player's upkeep)     | WORKING | 2026-05-28_#2360(c5681a91) | mtg-583   | The Abyss        |
| Drawn ValidCard$ Card.OppOwn Execute$ DB$ DealDamage Defined$ TriggeredPlayer    | WORKING | 2026-05-29_#2449(b5fd60b7) | (none)      | Underworld Dreams |
| Phase Upkeep ValidPlayer$ Player Execute$ DB$ DealDamage Defined$ TriggeredPlayer NumDmg$ X (variable damage to active player, counted against that player) | WORKING | 2026-05-30_#2530(f7a005ca) | (fixed)   | Karma            |
| Phase Upkeep ValidPlayer$ Player.Chosen Execute$ DB$ DealDamage Defined$ ChosenPlayer NumDmg$ X (fires only on the ETB-chosen player's upkeep; chosen_player_turn_only gate) | WORKING | 2026-05-31_#2546(34f76ca3) | (fixed) | Black Vise |
| Discarded ValidCard$ Card.Self ValidCause$ SpellAbility.OppCtrl Execute$ DB$ LoseLife (opponent-forced-discard punisher) | BROKEN | 2026-05-30_#2530(f7a005ca) | mtg-czz3f | Psychic Purge |
| DamageDealtOnce ValidSource$ Card.AttachedBy Execute$ GainLife LifeAmount$ TriggerCount$DamageAmount (triggered pseudo-lifelink aura; fires on combat damage to players AND creatures) | WORKING | 2026-05-31_#2539(05d48a7b) | (fixed) | Spirit Link |
| Phase Draw ValidPlayer$ You Execute$ DB$ Draw (beginning-of-your-draw-step: draw an additional card) | WORKING | 2026-05-30_#2532(4646ddd1) | (fixed) | Grafted Skullcap |
| Phase Draw ValidPlayer$ You Execute$ AB$ ChooseCard Cost$ Draw<N/You> + RepeatEach + UnlessCost$ PayLife (draw-then-choose-then-pay-or-return) | PARTIAL | 2026-05-30_#2532(4646ddd1) | mtg-548 | Sylvan Library |

## Activated abilities (A:) and cost shapes

| Construct                                  | Status  | Last verified              | Bug issue  | Sample cards        |
|--------------------------------------------|---------|----------------------------|------------|---------------------|
| AB$ Draw                                   | WORKING | 2026-05-12_#2226(928ec99f) | (none)     | Bazaar of Baghdad   |
| AB$ Discard with SubAbility chain          | WORKING | 2026-05-12_#2226(928ec99f) | (none)     | Bazaar of Baghdad   |
| AB$ DealDamage (single target, X = 1)      | WORKING | 2026-05-12_#2226(928ec99f) | (none)     | Triskelion          |
| Cost$ SubCounter<+1/+1>                    | WORKING | 2026-05-12_#2226(928ec99f) | (fixed)    | Triskelion          |
| Cost$ T (tap as cost)                      | WORKING | 2026-05-12_#2226(928ec99f) | (none)     | Bazaar of Baghdad   |
| Mode$ TgtChoose (controller picks discard) | WORKING | 2026-05-12_#2226(928ec99f) | (none)     | Bazaar of Baghdad   |
| AB$ Pump Cost$ R NumAtt$ +1 (firebreathing) | WORKING | 2026-05-29_#2432(f85d828d) | (none)    | Shivan Dragon       |
| AB$ Pump Cost$ T ValidTgts$ Creature.X (targeted activated pump) | WORKING | 2026-05-30_#2525(d40c4206) | (fixed) | Mishra's Factory |
| R$ Event$ Untap Layer$ CantHappen (doesn't-untap lock) | WORKING | 2026-05-30_#2525(d40c4206) | (fixed) | Paralyze |
| K$ DoesNotUntap (forced stay-tapped, untap step) | WORKING | 2026-05-30_#2525(d40c4206) | (fixed) | Paralyze |
| T$ Phase Upkeep ValidPlayer$ Player.EnchantedController + UnlessCost untap | BROKEN | 2026-05-30_#2525(d40c4206) | mtg-92jcg | Paralyze |
| SP$ Destroy ValidTgts$ Artifact,Enchantment | WORKING | 2026-05-29_#2432(f85d828d) | (none)    | Disenchant          |
| AB$ ChangeZone ActivationZone$ Graveyard   | BROKEN  | 2026-05-30_#2488(f9fcef95) | mtg-d8zuh  | Earthquake Dragon   |
| AB$ Destroy Cost$ T ValidTgts$ Creature.tapped | WORKING | 2026-05-29_#2449(b5fd60b7) | (none) | Royal Assassin      |
| AB$ Regenerate Cost$ B (self)              | WORKING | 2026-05-29_#2449(b5fd60b7) | (none)    | Will-o'-the-Wisp    |
| SP$ Mana Produced$ B Amount$ 3 (ritual)    | WORKING | 2026-05-29_#2449(b5fd60b7) | (none)    | Dark Ritual         |
| SP$ Destroy ValidTgts$ Land                | WORKING | 2026-05-29_#2449(b5fd60b7) | (none)    | Sinkhole            |
| SP$ ChangeZone Library->Hand (tutor->SearchLibrary) | WORKING | 2026-05-29_#2449(b5fd60b7) | (none) | Demonic Tutor   |
| AB$ Draw Cost$ B PayLife<2>                 | WORKING | 2026-05-29_#2449(b5fd60b7) | (none)    | Greed               |
| AB$ Mana Cost$ T Produced$ C Amount$ 2 (fast mana) | WORKING | 2026-05-29_#2449(b5fd60b7) | (none) | Sol Ring        |
| AB$ Mana Cost$ T Sac<1/CARDNAME> Produced$ Any Amount$ 3 | WORKING | 2026-05-29_#2449(b5fd60b7) | (none) | Black Lotus |
| SP$ AddTurn NumTurns$ 1 (extra turn, CR 500.7) | WORKING | 2026-05-29_#2456(e30f4ce1) | (fixed mtg-551) | Time Walk |
| SP$ DealDamage ValidTgts$ Any NumDmg$ 3       | WORKING | 2026-05-29_#2456(e30f4ce1) | (none)     | Lightning Bolt      |
| SP$ DealDamage + chained DB$ DealDamage Defined$ You (downside) | WORKING | 2026-05-29_#2456(e30f4ce1) | (none) | Psionic Blast |
| SP$ Draw NumCards$ N/X ValidTgts$ Player      | WORKING | 2026-05-29_#2456(e30f4ce1) | (none)     | Ancestral Recall, Braingeyser |
| SP$ Counter TargetType$ Spell                 | WORKING | 2026-05-29_#2456(e30f4ce1) | (none)     | Counterspell        |
| SP$ Counter UnlessCost$ X + SubAbility$ TapAll → DB$ DrainMana Defined$ TargetedController | WORKING | 2026-05-30_#2530(199b91e1) | (fixed mtg-532) | Power Sink |
| SP$ Counter RememberCounteredCMC$ True (record countered spell's mana value for a chained effect) | WORKING | 2026-05-30_#2538(58715f3f) | (fixed mtg-519) | Mana Drain |
| DB$ DelayedTrigger Mode$ Phase Phase$ Main1,Main2 ValidPlayer$ You Execute$ DB$ Mana (one-shot "at the beginning of your next main phase, add {C}×rememberedNumber"; general phase-delayed-trigger construct) | WORKING | 2026-05-30_#2538(58715f3f) | (fixed mtg-519) | Mana Drain |
| DB$ DrainMana Defined$ TargetedController (empty a player's unspent mana pool, CR 106.4/500.4) | WORKING | 2026-05-30_#2530(199b91e1) | (fixed mtg-532) | Power Sink |
| SP$ Destroy ValidTgts$ Creature.nonArtifact+nonBlack NoRegen$ True | WORKING | 2026-05-29_#2461(53f1d817) | (none) | Terror |
| AB$ Draw Cost$ 2 T NumCards$ 1 + SubAbility$ DBDiscard | WORKING | 2026-05-29_#2461(53f1d817) | (none) | Jalum Tome |
| AB$ DestroyAll ValidCards$ Artifact,Creature,Enchantment | WORKING | 2026-05-29_#2461(53f1d817) | (none) | Nevinyrral's Disk |
| R:Event$ Moved Destination$ Battlefield ReplaceWith$ ETBTapped (enters tapped) | WORKING | 2026-05-29_#2461(53f1d817) | (none) | Nevinyrral's Disk |
| SP$ Charm modes enforce per-mode ValidTgts$ color restriction | WORKING | 2026-05-29_#2470(be2f61b4) | (fixed mtg-af24s) | Red/Blue Elemental Blast |
| DB$ GainLife LifeAmount$ X (X = Targeted$CardPower / Targeted$CardManaCost; dynamic-amount life gain) | WORKING | 2026-05-30_#2489(1db3e6c7) | (fixed mtg-297) | Swords to Plowshares, Divine Offering |
  - 2026-05-30_#2489(1db3e6c7): BROKEN→WORKING. Added the general
    `DynamicAmount` construct (`core/effects.rs`) and `Effect::GainLifeDynamic
    { player, amount, reference }`. The amount source is a strong-typed enum
    (`TargetPower` / `TargetManaValue` / `DamageDealt`) parsed from the
    `Targeted$<Characteristic>` SVar, NOT a stringly amount. The recipient is
    resolved from `Defined$` (`TargetedController` → new `PlayerId::target_controller()`
    sentinel; `You` → caster). Amount captured as last-known information
    (CR 608.2g/2h) via a pre-resolution power/mana-value snapshot using the
    CR 613 layer system, so continuous static buffs (e.g. Sedge Troll +1/+1
    while controlling a Swamp) are counted before the exile/destroy strips
    them. Info-independent / deterministic (public state only). `DamageDealt`
    (Drain Life) is enum-reserved but NOT yet wired — see the Drain Life row.
| DB$ GainLife LifeAmount$ <SVar with Count$TotalDamageDoneByThisTurn + LimitMax cap> (damage-dealt drain) | BROKEN | 2026-05-30_#2489(1db3e6c7) | mtg-501 | Drain Life |
| S:Mode$ CantBlockBy ValidBlocker$ Creature.Self (this creature can't block X) | BROKEN | 2026-05-29_#2456(e30f4ce1) | mtg-512 | Ironclaw Orcs |
| SP$ ChangeZoneAll Origin$ Hand,Graveyard (multi-origin / Hand origin) + Shuffle$ True | WORKING | 2026-05-30_#2533(b052ce01) | (fixed mtg-552) | Timetwister |
  - 2026-05-30: `Effect::ChangeZoneAll.origin: Zone` -> `origins: SmallVec<[Zone;2]>`
    plus a new `shuffle: bool` field (derived from `Shuffle$ True`). The
    converter now parses comma-separated `Origin$` into a zone list; the
    application handler iterates every origin and gained Hand & Library origin
    support; when `shuffle` is set and the destination is the library, affected
    libraries are shuffled via the deterministic game RNG (replay-safe, no
    hidden-info leak — only RNG advances). Ordered library moves
    (`LibraryPosition$ -1`, e.g. Manifold Insights) keep `shuffle=false` and are
    untouched. Lifts every multi-origin / Shuffle$ True mass move (Timetwister,
    Mnemonic Nexus, Midnight Clock). Gamelog is now human-readable
    ("moves all cards from Hand+Graveyard to Library").
| SP$ ChangeZoneAll ChangeType$ Artifact Origin$ Battlefield Destination$ Hand (mass owner-filtered bounce) | WORKING | 2026-05-30_#2533(b052ce01) | (none) | Hurkyl's Recall |
| AB$ ManaReflected (reflected/filter mana, ReflectProperty$ Produce) | WORKING | 2026-05-30_#2536(compat-wave18) | (fixed) | Fellwar Stone |
| SP$ DealDamage + SubAbility$ Effect chain + ReplaceDyingDefined$ (exile instead) | WORKING | 2026-05-30_#2535(48fc49b8) | (fixed mtg-ioesm) | Disintegrate |
| SP$ DealDamage NumDmg$ X (DealDamageXPaid) at a creature — display target binding | WORKING | 2026-05-30_#2535(48fc49b8) | (fixed mtg-ioesm) | Disintegrate, Blaze |
| ReplaceDyingDefined$ ThisTargetedCard.Creature (exile if would die this turn) | WORKING | 2026-05-30_#2535(48fc49b8) | (fixed mtg-ioesm) | Disintegrate |
| SP$ DealDamage DivideEvenly$ RoundedDown + TargetMin/TargetMax (variable-count multi-target X-burn) | WORKING | 2026-05-31_#2542(348be74e) | (fixed mtg-tyvcn) | Fireball |
| S:Mode$ RaiseCost Relative$ True (per-target {1} surcharge, cost computed after target selection) | WORKING | 2026-05-31_#2542(348be74e) | (fixed mtg-tyvcn) | Fireball |
| T:Mode$ Always state-trigger + setARN set-origin match + S:Mode$ CantBeCast/CantPlayLand | WORKING | 2026-05-31_#2540(ad30b333) | (fixed mtg-3hwz3) | City in a Bottle |
| Valid filter `set<CODE>` set-origin qualifier (e.g. `setARN`) | WORKING | 2026-05-31_#2540(ad30b333) | (fixed mtg-3hwz3) | City in a Bottle |
| Valid filter `Other` self-exclusion qualifier (matches_excluding) | WORKING | 2026-05-31_#2540(ad30b333) | (fixed mtg-3hwz3) | City in a Bottle |
| T:Mode$ Always state-trigger sweep -> StaticAbility::SacrificeMatchingPresent (SBA-like, CR 603.8/704.3) | WORKING | 2026-05-31_#2540(ad30b333) | (fixed mtg-3hwz3) | City in a Bottle |
| S:Mode$ CantBeCast ValidCard$ <filter> (cast prohibition gate) | WORKING | 2026-05-31_#2540(ad30b333) | (fixed mtg-3hwz3) | City in a Bottle |
| S:Mode$ CantPlayLand ValidCard$ <filter> (land/spell-play prohibition gate) | WORKING | 2026-05-31_#2540(ad30b333) | (fixed mtg-3hwz3) | City in a Bottle |
| SP$ Discard NumCards$ X ValidTgts$ Player Mode$ Random (X-paid discard) | WORKING | 2026-05-29_#2462(132ce6cc) | (fixed mtg-521) | Mind Twist |
| AB$ activation gate IsPresent$/PresentZone$/PresentCompare$ (EQ/GE/LE) | WORKING | 2026-05-29_#2470(be2f61b4) | (fixed mtg-517) | Library of Alexandria |
| AB$ ChooseSource Choices$ Card.<Color>Source (source-filtered damage prevention, Circle of Protection) | WORKING | 2026-05-30_#2491(dded4d83) | (fixed mtg-490) | Circle of Protection: Red |
  - 2026-05-30: New general damage-prevention construct (CR 615.1/615.6).
    `core/prevention.rs` adds strong-typed `DamageSourceFilter` (Color /
    SpecificSource / ColoredSource) + `DamagePreventionShield` (scope
    AllThisTurn / NextEvent / NextPoints) stored per-player in
    `Player::source_prevention_shields`. `Effect::PreventDamageFromSource
    { protected, color, source }` is produced from the CoP card shape
    (`AB$ ChooseSource | Choices$ Card.<Color>Source | SubAbility$ DBEffect`)
    by reading the colour from the tokenized `Choices$` filter (NOT
    substring-matched), generalizing across all five Circles of Protection.
    The shield is consulted on the damage path in BOTH combat attribution
    (`actions/combat.rs`) and direct/burn damage (`deal_damage` via the
    transient `GameState::current_damage_source`), preventing the chosen
    source's next damage event and expiring at cleanup (CR 514.2).
    CoP:Red is verified end-to-end (primary target, mtg-490). The chooser
    offers matching-colour creatures (battlefield) + spells (stack) and
    excludes the Circle's own enchantment, so all five Circles
    (White/Blue/Black/Green) work via the identical colour-filter path —
    CoP:White verified preventing a white attacker in the same way. The only
    intentional gap is non-creature damaging permanents (e.g. a coloured
    damage artifact) as a chosen source, which the current chooser omits to
    keep the AS auto-pick aimed at real threats (heuristic follow-up).

## Static abilities (S:)

| Construct                                       | Status  | Last verified              | Bug issue  | Sample cards |
|-------------------------------------------------|---------|----------------------------|------------|--------------|
| StaticAbility IsPresent$ <selector>             | BROKEN  | 2026-05-12_#2226(928ec99f) | mtg-203  | (multiple)   |
| ModifyPT Affected$ Card.Self + IsPresent$ <sel> | WORKING | 2026-05-29_#2430(048328c1) | (fixed)    | Sedge Troll  |
| StaticAbility Threshold$                        | BROKEN  | 2026-05-12_#2226(928ec99f) | mtg-203  | (multiple)   |
| RaiseCost ValidCard$ Card.<Color> Type$ Spell   | WORKING | 2026-05-29_#2469(6c054829) | (fixed)   | Gloom        |
| RaiseCost own-controller filter (hose effects)  | WORKING | 2026-05-29_#2469(6c054829) | (fixed)   | Gloom        |
| Continuous GainControl$ You (control Auras)     | WORKING | 2026-05-29_#2470(be2f61b4) | (fixed)   | Control Magic |
| GrantKeyword Affected$ Creature (AllCreatures)  | WORKING | 2026-05-30_#2487(30dd3c20) | (none)    | Concordant Crossroads |

## Replacement effects (R:)

| Construct                | Status   | Last verified              | Bug issue | Sample cards |
|--------------------------|----------|----------------------------|-----------|--------------|
| DB$ Clone (CR 707 copy)  | WORKING  | 2026-05-27_#2354(dbf857a7) | (none)    | Copy Artifact |

## Mana production

| Produced$ form                  | Status  | Last verified              | Bug issue | Sample cards        |
|---------------------------------|---------|----------------------------|-----------|---------------------|
| Produced$ B                     | WORKING | 2026-05-12_#2226(928ec99f) | (none)    | Mox Jet             |
| Produced$ W                     | WORKING | 2026-05-29_#2432(f85d828d) | (none)    | Mox Pearl           |
| Produced$ R                     | WORKING | 2026-05-29_#2432(f85d828d) | (none)    | Mox Ruby            |
| Produced$ G                     | WORKING | 2026-05-29_#2432(f85d828d) | (none)    | Mox Emerald         |
| Produced$ U                     | WORKING | 2026-05-29_#2456(e30f4ce1) | (none)    | Mox Sapphire        |
| Produced$ Any                   | WORKING | 2026-05-12_#2226(928ec99f) | (fixed)   | City of Brass       |
| Produced$ C Amount$ 4 (DB$ Mana on dies trigger) | WORKING | 2026-05-29_#2440(00e22751) | (none) | Su-Chi |
| Intrinsic basic-land mana (CR 305.6: 1 {T}:Add ability per basic subtype) | WORKING | 2026-05-29_#2456(e30f4ce1) | (none) | Island, Plains, Tundra, Underground Sea, Badlands, Scrubland, Bayou, Plateau, Volcanic Island |
| Produced$ C Amount$ 3 on a LAND (multi-mana land tap) | WORKING | 2026-05-30_#2533(b052ce01) | (fixed mtg-523) | Mishra's Workshop |
  - 2026-05-30: `tap_for_mana_for_cost`'s LAND path previously added exactly 1
    pip, ignoring the cached `mana_production.amount`. Now multiplies by the
    land's per-activation amount (Workshop = 3), mirroring the Black-Lotus
    any-color branch. Lifts any land producing >1 mana per tap. NOTE: the
    `RestrictValid$ Spell.Artifact` spend-restriction is still unenforced
    (mtg-53gp9) — Workshop mana is currently spendable on any spell.
| RestrictValid$ on mana ability (spend-only-on-X) | BROKEN | 2026-05-30_#2533(b052ce01) | mtg-53gp9 | Mishra's Workshop |

## Selectors / parameters

| Construct                                       | Status   | Last verified              | Bug issue   | Sample cards     |
|-------------------------------------------------|----------|----------------------------|-------------|------------------|
| Enchant <description>                           | PARTIAL  | 2026-05-12_#2226(928ec99f) | mtg-203   | Animate Dead     |
| ValidTgts$ Creature                             | WORKING  | 2026-05-12_#2226(928ec99f) | (none)      | Triskelion       |
| Affected$ <selector> (general)                  | PARTIAL  | 2026-05-12_#2226(928ec99f) | mtg-147     | (multiple)       |
| ValidTgts$ Creature.nonArtifact (excl. artifact)| WORKING  | 2026-05-28_#2360(c5681a91) | (none)      | The Abyss        |
| ValidTgts$ ...+ActivePlayerCtrl (active player) | WORKING  | 2026-05-28_#2360(c5681a91) | (none)      | The Abyss        |
| Count$Valid <BasicLandSubtype>.ActivePlayerCtrl (count active player's Swamps/etc.) | WORKING  | 2026-05-30_#2530(f7a005ca) | (fixed)     | Karma            |
| Count$ValidHand <selector>[/Minus.N or /Plus.N] (count cards in a player's hand, with arithmetic modifier) | WORKING | 2026-05-31_#2546(34f76ca3) | (fixed) | Black Vise |
| DB$ Destroy NoRegen$ True (can't be regenerated)| WORKING  | 2026-05-28_#2360(c5681a91) | (none)      | The Abyss        |
| ChangeZone Origin$ Stack Destination$ Exile     | WORKING  | 2026-05-28_#2362(f454dccb) | (none)      | All Hallow's Eve |
| RememberChanged$ True + Defined$ Remembered      | WORKING  | 2026-05-28_#2362(f454dccb) | (none)      | All Hallow's Eve |
| TriggerZones$ Exile (exile-resident phase trig) | WORKING  | 2026-05-28_#2362(f454dccb) | (none)      | All Hallow's Eve |
| IsPresent$ ...counters_GE/EQ_<TYPE> (interv.-if)| WORKING  | 2026-05-28_#2362(f454dccb) | (none)      | All Hallow's Eve |
| ChangeZone Defined$ Self (non-stack self-move)  | WORKING  | 2026-05-28_#2362(f454dccb) | (none)      | All Hallow's Eve |
| ConditionDefined$ Self + ConditionPresent$ ctr  | WORKING  | 2026-05-28_#2362(f454dccb) | (none)      | All Hallow's Eve |
| ChangeZoneAll Graveyard→Battlefield (mass reanim)| WORKING  | 2026-05-28_#2362(f454dccb) | (none)      | All Hallow's Eve |

---

History footnotes (most recent first):

- 2026-05-28_#2362(f454dccb) — All Hallow's Eve brought to WORKING
  (mtg-464870 / mtg-393). Added general exile-resident phase
  triggers (`Trigger::trigger_zones` + `present_self_condition`,
  scanned in `check_phase_triggers`), the `MoveSelfBetweenZones` and
  `ConditionalSelfCounter` effects, and counter-gated mass
  resurrection via `ChangeZoneAll` Graveyard→Battlefield. The
  ChangeZone Stack→Exile + RememberChanged path was already present.
- 2026-05-12_#2226(928ec99f) — Initial population by
  `compatibility_tracking` skill rollout. Seeded from the
  15-card session covering Bazaar of Baghdad, Chaos Orb,
  All Hallow's Eve, Animate Dead, Wheel of Fortune, Triskelion,
  Serra Angel, Black Knight, Sedge Troll, City of Brass,
  Strip Mine, Sengir Vampire, Mox Jet, Hypnotic Specter,
  Demonic Tutor.
