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
| ChangesZone B→Gy ValidCard$ Creature.DamagedBy                                   | BROKEN  | 2026-05-12_#2226(928ec99f) | mtg-f0bfb8  | Sengir Vampire   |
| DamageDoneOnce by ~ to creature/player                                           | WORKING | 2026-05-12_#2226(928ec99f) | (none)      | Hypnotic Specter |

## Activated abilities (A:) and cost shapes

| Construct                                  | Status  | Last verified              | Bug issue  | Sample cards        |
|--------------------------------------------|---------|----------------------------|------------|---------------------|
| AB$ Draw                                   | WORKING | 2026-05-12_#2226(928ec99f) | (none)     | Bazaar of Baghdad   |
| AB$ Discard with SubAbility chain          | WORKING | 2026-05-12_#2226(928ec99f) | (none)     | Bazaar of Baghdad   |
| AB$ DealDamage (single target, X = 1)      | WORKING | 2026-05-12_#2226(928ec99f) | (none)     | Triskelion          |
| Cost$ SubCounter<+1/+1>                    | WORKING | 2026-05-12_#2226(928ec99f) | (fixed)    | Triskelion          |
| Cost$ T (tap as cost)                      | WORKING | 2026-05-12_#2226(928ec99f) | (none)     | Bazaar of Baghdad   |
| Mode$ TgtChoose (controller picks discard) | WORKING | 2026-05-12_#2226(928ec99f) | (none)     | Bazaar of Baghdad   |

## Static abilities (S:)

| Construct                                       | Status  | Last verified              | Bug issue  | Sample cards |
|-------------------------------------------------|---------|----------------------------|------------|--------------|
| StaticAbility IsPresent$ <selector>             | BROKEN  | 2026-05-12_#2226(928ec99f) | mtg-o7dqu  | (multiple)   |
| StaticAbility Threshold$                        | BROKEN  | 2026-05-12_#2226(928ec99f) | mtg-o7dqu  | (multiple)   |

## Replacement effects (R:)

| Construct                | Status   | Last verified              | Bug issue | Sample cards |
|--------------------------|----------|----------------------------|-----------|--------------|
| DB$ Clone (CR 707 copy)  | WORKING  | 2026-05-27_#2354(dbf857a7) | (none)    | Copy Artifact |

## Mana production

| Produced$ form                  | Status  | Last verified              | Bug issue | Sample cards        |
|---------------------------------|---------|----------------------------|-----------|---------------------|
| Produced$ B                     | WORKING | 2026-05-12_#2226(928ec99f) | (none)    | Mox Jet             |
| Produced$ Any                   | WORKING | 2026-05-12_#2226(928ec99f) | (fixed)   | City of Brass       |

## Selectors / parameters

| Construct                                       | Status   | Last verified              | Bug issue   | Sample cards     |
|-------------------------------------------------|----------|----------------------------|-------------|------------------|
| Enchant <description>                           | PARTIAL  | 2026-05-12_#2226(928ec99f) | mtg-o7dqu   | Animate Dead     |
| ValidTgts$ Creature                             | WORKING  | 2026-05-12_#2226(928ec99f) | (none)      | Triskelion       |
| Affected$ <selector> (general)                  | PARTIAL  | 2026-05-12_#2226(928ec99f) | mtg-147     | (multiple)       |

---

History footnotes (most recent first):

- 2026-05-12_#2226(928ec99f) — Initial population by
  `compatibility_tracking` skill rollout. Seeded from the
  15-card session covering Bazaar of Baghdad, Chaos Orb,
  All Hallow's Eve, Animate Dead, Wheel of Fortune, Triskelion,
  Serra Angel, Black Knight, Sedge Troll, City of Brass,
  Strip Mine, Sengir Vampire, Mox Jet, Hypnotic Specter,
  Demonic Tutor.
