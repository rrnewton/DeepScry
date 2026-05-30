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

## Triggers (T:)

| Trigger pattern                                                                  | Status  | Last verified              | Bug issue   | Sample cards     |
|----------------------------------------------------------------------------------|---------|----------------------------|-------------|------------------|
| ChangesZone B→Gy ValidCard$ Card.Self                                            | WORKING | 2026-05-29_#2440(00e22751) | (none)      | Su-Chi, (multiple) |
| ChangesZone B→Gy ValidCard$ Creature.DamagedBy                                   | WORKING | 2026-05-28_#2360(897881c9) | (fixed)     | Sengir Vampire   |
| DamageDoneOnce by ~ to creature/player                                           | WORKING | 2026-05-12_#2226(928ec99f) | (none)      | Hypnotic Specter |
| DamageDone Execute$ DB$ Discard Defined$ TriggeredTarget (damaged player discards)| WORKING | 2026-05-29_#2449(b5fd60b7) | (fixed)     | Hypnotic Specter |
| Phase Upkeep ValidPlayer$ Player Execute$ DB$ Destroy (each player's upkeep)     | WORKING | 2026-05-28_#2360(c5681a91) | mtg-583   | The Abyss        |
| Drawn ValidCard$ Card.OppOwn Execute$ DB$ DealDamage Defined$ TriggeredPlayer    | WORKING | 2026-05-29_#2449(b5fd60b7) | (none)      | Underworld Dreams |

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
| SP$ Destroy ValidTgts$ Artifact,Enchantment | WORKING | 2026-05-29_#2432(f85d828d) | (none)    | Disenchant          |
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
| SP$ Destroy ValidTgts$ Creature.nonArtifact+nonBlack NoRegen$ True | WORKING | 2026-05-29_#2461(53f1d817) | (none) | Terror |
| AB$ Draw Cost$ 2 T NumCards$ 1 + SubAbility$ DBDiscard | WORKING | 2026-05-29_#2461(53f1d817) | (none) | Jalum Tome |
| AB$ DestroyAll ValidCards$ Artifact,Creature,Enchantment | WORKING | 2026-05-29_#2461(53f1d817) | (none) | Nevinyrral's Disk |
| R:Event$ Moved Destination$ Battlefield ReplaceWith$ ETBTapped (enters tapped) | WORKING | 2026-05-29_#2461(53f1d817) | (none) | Nevinyrral's Disk |
| SP$ Charm modes enforce per-mode ValidTgts$ restriction | BROKEN | 2026-05-29_#2461(53f1d817) | mtg-af24s | Red Elemental Blast |
| DB$ GainLife LifeAmount$ X (X = Targeted$CardManaCost / dynamic) | BROKEN | 2026-05-29_#2461(53f1d817) | mtg-297 | Divine Offering, Swords to Plowshares |
| S:Mode$ CantBlockBy ValidBlocker$ Creature.Self (this creature can't block X) | BROKEN | 2026-05-29_#2456(e30f4ce1) | mtg-512 | Ironclaw Orcs |
| SP$ ChangeZoneAll Origin$ Hand,Graveyard (multi-origin / Hand origin) | BROKEN | 2026-05-29_#2456(e30f4ce1) | mtg-552 | Timetwister |
| SP$ Discard NumCards$ X ValidTgts$ Player Mode$ Random (X-paid discard) | WORKING | 2026-05-29_#2462(132ce6cc) | (fixed mtg-521) | Mind Twist |
| AB$ activation gate IsPresent$/PresentZone$/PresentCompare$ (EQ/GE/LE) | WORKING | 2026-05-29_#2470(be2f61b4) | (fixed mtg-517) | Library of Alexandria |

## Static abilities (S:)

| Construct                                       | Status  | Last verified              | Bug issue  | Sample cards |
|-------------------------------------------------|---------|----------------------------|------------|--------------|
| StaticAbility IsPresent$ <selector>             | BROKEN  | 2026-05-12_#2226(928ec99f) | mtg-203  | (multiple)   |
| ModifyPT Affected$ Card.Self + IsPresent$ <sel> | WORKING | 2026-05-29_#2430(048328c1) | (fixed)    | Sedge Troll  |
| StaticAbility Threshold$                        | BROKEN  | 2026-05-12_#2226(928ec99f) | mtg-203  | (multiple)   |
| RaiseCost ValidCard$ Card.<Color> Type$ Spell   | WORKING | 2026-05-29_#2469(6c054829) | (fixed)   | Gloom        |
| RaiseCost own-controller filter (hose effects)  | WORKING | 2026-05-29_#2469(6c054829) | (fixed)   | Gloom        |
| Continuous GainControl$ You (control Auras)     | WORKING | 2026-05-29_#2470(be2f61b4) | (fixed)   | Control Magic |

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

## Selectors / parameters

| Construct                                       | Status   | Last verified              | Bug issue   | Sample cards     |
|-------------------------------------------------|----------|----------------------------|-------------|------------------|
| Enchant <description>                           | PARTIAL  | 2026-05-12_#2226(928ec99f) | mtg-203   | Animate Dead     |
| ValidTgts$ Creature                             | WORKING  | 2026-05-12_#2226(928ec99f) | (none)      | Triskelion       |
| Affected$ <selector> (general)                  | PARTIAL  | 2026-05-12_#2226(928ec99f) | mtg-147     | (multiple)       |
| ValidTgts$ Creature.nonArtifact (excl. artifact)| WORKING  | 2026-05-28_#2360(c5681a91) | (none)      | The Abyss        |
| ValidTgts$ ...+ActivePlayerCtrl (active player) | WORKING  | 2026-05-28_#2360(c5681a91) | (none)      | The Abyss        |
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
