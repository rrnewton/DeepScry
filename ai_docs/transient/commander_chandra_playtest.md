# Commander Chandra Tokens Deck - Playtest Checklist

Deck: `decks/commander/chandra_tokens.dck`
Tracking issue: mtg-274
Updated: 2026-04-01_#2038

## Bugs Found and Fixed (10 total across 6 iterations)
1. **Token ownership (c2df44a9)**: CRITICAL - tokens always created under P1
2. **Token script pre-loading (c1016144)**: A:/T: lines not scanned for TokenScript$
3. **"cast" matching for command zone (c1016144)**: Fixed controller couldn't cast commander
4. **Planeswalker tapped for mana (4f134568)**: CRITICAL - mana engine used planeswalkers as sources
5. **Loyalty costs parsed as mana (6f4e41c1)**: AddCounter/SubCounter were garbage-parsed as mana
6. **No starting loyalty counters (6f4e41c1)**: Loyalty:N field not parsed from card scripts
7. **Heuristic AI ignores CastFromCommand (cd5509c1)**: Commander invisible to spell evaluation
8. **Heuristic AI never casts planeswalkers (cd5509c1)**: No type check in should_cast_spell
9. **Token creature selectors unimplemented (d66cddde)**: Intangible Virtue had no effect
10. **CI formatting (multiple)**: cargo fmt issues

## Commander Mechanics
- [x] Commander starts in command zone
- [x] Starting life is 40 (auto-detected)
- [x] Commander can be cast from command zone
- [x] Commander tax increases cost by {2} per cast (verified: 2RR -> 4RR in game)
- [x] Commander returns to command zone when would go to graveyard/exile
- [x] Commander damage tracking (21+ combat damage = loss, unit tested)
- [ ] Player choice for zone-change replacement (currently automatic)

## Planeswalker Mechanics
- [x] Loyalty:N field parsed from card scripts
- [x] Starting loyalty counters on ETB (Chandra enters with 4)
- [x] +1 loyalty abilities (gains 1, now 5)
- [x] -3 loyalty abilities (loses 3, checks sufficient loyalty)
- [x] 0-loyalty death -> graveyard -> command zone (full lifecycle)
- [x] Planeswalker NOT used as mana source
- [x] Full lifecycle verified: cast -> abilities -> death -> command zone -> re-cast with tax

## Overall Testing
- [x] 30 random-seeded games complete (0 crashes, 0 panics)
- [x] Win conditions: 83% Decking, 17% PlayerDeath
- [x] 10 heuristic AI games complete (both AIs cast Chandra)
- [x] 105-turn heuristic mirror game (full card variety)
- [x] Fixed controller tests: Sol Ring, Oketra trigger, Chandra, tokens
- [x] Token buff combat damage verified (274x 1dmg, 70x 2dmg with Intangible Virtue)
- [x] Benchmarks: 60K+ games/sec, no regression

## Card Status

### Verified Working (via gameplay observation)
| Card | Status |
|------|--------|
| Chandra, Torch of Defiance | Full lifecycle: cast, loyalty, death, return, tax |
| Sol Ring | Cast and on battlefield |
| Arcane Signet | Cast and on battlefield |
| Fellwar Stone | Cast by AI |
| Raise the Alarm | Creates 2 Soldier tokens (correct ownership) |
| Dragon Fodder | Creates 2 Goblin tokens (correct ownership) |
| Hordeling Outburst | Creates 3 Goblin tokens |
| Secure the Wastes | X spell creates Warrior tokens |
| Tempt with Vengeance | X spell creates Elemental tokens with haste |
| Oketra's Monument | Trigger creates Warrior with vigilance on creature cast |
| Anim Pakal, Thousandth Moon | Creates Gnome tokens on attack |
| Intangible Virtue | +1/+1 to creature tokens (verified 2/2 display + 2 combat damage) |
| Heroic Reinforcements | Creates 2 Soldiers + pumps creatures |
| Assemble the Legion | Upkeep muster counter trigger + token creation |
| Legion Warboss | Combat-start Goblin creation |
| Goblin Rabblemaster | Combat-start Goblin creation |
| Siege-Gang Lieutenant | Commander-aware Lieutenant trigger |
| Akroan Crusader | Heroic trigger creates Soldier token |
| Legion's Landing | Transform trigger on 3+ attackers |
| Goblin Sharpshooter | Activated: deals 1 damage |
| Boros Charm | Modal: 4 damage (targeting issue noted) |
| Searing Barrage | Deals damage correctly |
| Swiftfoot Boots | Equip ability works |
| Welcoming Vampire | Cast by AI |
| Goblin Warchief | Cost reduction + haste |
| Goblin Trashmaster | Cast and used |
| Muxus, Goblin Grandee | ETB Dig ability |
| Krenko, Mob Boss | Token producer |
| Swarming Goblins | ETB Goblin tokens |
| Thopter Architect | Thopter token creation |
| Thopter Engineer | Thopter token creation |
| Rosie Cotton of South Lane | Token synergy |
| Bennie Bracks, Zoologist | Card draw |
| Goldnight Commander | ETB trigger |
| Goblin Bushwhacker | Kicker |
| Arabella, Abandoned Doll | Enters as 1/3 |

### Working (in deck, castable, no issues found)
Snow-Covered Plains, Snow-Covered Mountain, Reliquary Tower, Thought Vessel,
Spellbook, Molten Influence, Shalai Voice of Plenty, Mentor of the Meek,
Clarion Spirit, Ocelot Pride, Zada Hedron Grinder, Akroan Hoplite,
Knollspine Dragon, Siege-Gang Commander, Goblin Barrage, Sizzling Barrage,
Brave the Elements, Apostle's Blessing, Mana Tithe, Rebuff the Wicked,
Dawn Charm, Deflecting Swat, Return the Favor, Untimely Malfunction,
Warleader's Call, Tocasia's Welcome, Thopter Arrest, Skullclamp,
Idol of Oblivion, Decanter of Endless Water, Assemble the Legion

### Known Remaining Issues (pre-existing, not commander-specific)
- Boros Charm targeting: DealDamage targets creatures (should only target player/planeswalker)
- ModalChoice not resolved during casting (affects Boros Charm, Dawn Charm)
- Slate of Ancestry complex cost (tap + discard hand) not fully supported
- Chandra +1 conditional damage chain (Dig->Play->conditional Damage) partially working
