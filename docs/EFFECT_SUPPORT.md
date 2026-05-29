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
| ChangesZone B→Gy ValidCard$ Card.Self                                            | WORKING | 2026-05-12_#2226(928ec99f) | (none)      | (multiple)       |
| ChangesZone B→Gy ValidCard$ Creature.DamagedBy                                   | WORKING | 2026-05-28_#2360(897881c9) | (fixed)     | Sengir Vampire   |
| DamageDoneOnce by ~ to creature/player                                           | WORKING | 2026-05-12_#2226(928ec99f) | (none)      | Hypnotic Specter |
| Phase Upkeep ValidPlayer$ Player Execute$ DB$ Destroy (each player's upkeep)     | WORKING | 2026-05-28_#2360(c5681a91) | mtg-583   | The Abyss        |

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

## Static abilities (S:)

| Construct                                       | Status  | Last verified              | Bug issue  | Sample cards |
|-------------------------------------------------|---------|----------------------------|------------|--------------|
| StaticAbility IsPresent$ <selector>             | BROKEN  | 2026-05-12_#2226(928ec99f) | mtg-203  | (multiple)   |
| ModifyPT Affected$ Card.Self + IsPresent$ <sel> | WORKING | 2026-05-29_#2430(048328c1) | (fixed)    | Sedge Troll  |
| StaticAbility Threshold$                        | BROKEN  | 2026-05-12_#2226(928ec99f) | mtg-203  | (multiple)   |

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
| Produced$ Any                   | WORKING | 2026-05-12_#2226(928ec99f) | (fixed)   | City of Brass       |
| Intrinsic dual-land mana (CR 305.6: 2 basic subtypes → 2 {T}:Add abilities) | WORKING | 2026-05-29_#2432(f85d828d) | (none) | Badlands, Scrubland, Bayou |

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
