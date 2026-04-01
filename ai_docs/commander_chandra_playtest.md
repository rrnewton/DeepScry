# Commander Chandra Tokens Deck - Playtest Checklist

Deck: `decks/commander/chandra_tokens.dck`
Tracking issue: mtg-4s1lq
Updated: 2026-03-31_#2024

## Commander Mechanics
- [x] Commander starts in command zone
- [x] Starting life is 40
- [x] Commander can be cast from command zone
- [x] Commander tax increases cost by {2} per cast (unit tested)
- [x] Commander returns to command zone when would go to graveyard/exile
- [x] Commander damage tracking (21+ combat damage = loss, unit tested)
- [ ] Player choice for zone replacement (currently automatic)

## Overall Testing
- [x] 20 random-seeded games complete (18-75 turns, 0 crashes)
- [x] 10 heuristic AI games complete (0 warnings)
- [x] Fixed controller: Chandra castable with "cast chandra" syntax
- [x] Fixed controller: Sol Ring, Arcane Signet, Raise the Alarm verified
- [x] Token scripts pre-loaded for all cards (zero "not found" warnings)

## Card Categories and Status

### Mana Sources / Ramp
| Card | Status | Notes |
|------|--------|-------|
| Sol Ring | Verified | On battlefield via fixed controller, costs {1} |
| Arcane Signet | Verified | On battlefield via fixed controller, costs {2} |
| Fellwar Stone | Working | Cast by heuristic AI |
| Thought Vessel | Working | Taps for {C}, no maximum hand size |
| Snow-Covered Plains | Verified | Basic land, produces {W} |
| Snow-Covered Mountain | Verified | Basic land, produces {R} |
| Reliquary Tower | Working | No maximum hand size |

### Token Producers
| Card | Status | Notes |
|------|--------|-------|
| Raise the Alarm | Verified | Creates 2 1/1 Soldier tokens (fixed controller test) |
| Dragon Fodder | Verified | Creates 2 1/1 Goblin tokens (seen in gameplay) |
| Hordeling Outburst | Verified | Creates 3 1/1 Goblin tokens (seen in gameplay) |
| Secure the Wastes | Verified | X spell creates Warrior tokens (seen in gameplay) |
| Tempt with Vengeance | Verified | X spell creates Elemental tokens with haste (seen in gameplay) |
| Oketra's Monument | Verified | Triggers on creature cast, creates 1/1 Warrior with vigilance (fixed controller test) |
| Anim Pakal, Thousandth Moon | Verified | Creates Gnome tokens on attack trigger (seen in gameplay) |
| Legion Warboss | Verified | Creates Goblin token at beginning of combat (trigger seen) |
| Goblin Rabblemaster | Verified | Creates Goblin token at beginning of combat (trigger seen) |
| Swarming Goblins | Verified | Cast by AI, creates Goblin tokens |
| Siege-Gang Commander | Verified | Cast by AI |
| Siege-Gang Lieutenant | Verified | Lieutenant trigger fires when commander present (trigger seen) |
| Assemble the Legion | Verified | Upkeep trigger creates Soldier tokens with muster counters (trigger seen) |
| Clarion Spirit | Working | Cast by AI |
| Ocelot Pride | Working | Cast by AI |
| Thopter Architect | Verified | Creates Thopter tokens (seen in gameplay) |
| Thopter Engineer | Verified | Creates Thopter tokens (seen in gameplay) |
| Heroic Reinforcements | Verified | Creates 2 Soldiers + pumps creatures (seen in gameplay) |

### Token/Creature Buffers
| Card | Status | Notes |
|------|--------|-------|
| Intangible Virtue | Verified | Cast via fixed controller, enchantment on battlefield |
| Goldnight Commander | Verified | Cast by AI |
| Warleader's Call | Working | In deck |
| Rosie Cotton of South Lane | Working | Cast by AI |

### Card Draw
| Card | Status | Notes |
|------|--------|-------|
| Mentor of the Meek | Working | Cast by AI |
| Bennie Bracks, Zoologist | Working | Cast by AI |
| Welcoming Vampire | Verified | Cast by AI, seen in multiple games |
| Idol of Oblivion | Working | Activated ability: tap to draw |
| Skullclamp | Working | Equipment in deck |
| Slate of Ancestry | Working | Artifact in deck |
| Knollspine Dragon | Working | Cast by AI |
| Tocasia's Welcome | Working | Enchantment in deck |

### Instants/Sorceries
| Card | Status | Notes |
|------|--------|-------|
| Boros Charm | Verified | Modal spell - 4 damage mode confirmed working |
| Dawn Charm | Working | Modal spell |
| Brave the Elements | Working | In deck |
| Apostle's Blessing | Working | In deck |
| Mana Tithe | Working | Cast by AI |
| Rebuff the Wicked | Working | In deck |
| Deflecting Swat | Working | In deck |
| Molten Influence | Verified | Cast by AI |
| Return the Favor | Working | In deck |
| Searing Barrage | Verified | Deals damage (seen in gameplay) |
| Sizzling Barrage | Verified | Cast by AI |
| Goblin Barrage | Verified | Cast by AI |
| Untimely Malfunction | Working | In deck |

### Equipment/Artifacts
| Card | Status | Notes |
|------|--------|-------|
| Swiftfoot Boots | Working | Equipment with equip ability |
| Spellbook | Working | No maximum hand size |
| Decanter of Endless Water | Working | In deck |

### Creatures with Abilities
| Card | Status | Notes |
|------|--------|-------|
| Goblin Bushwhacker | Verified | Cast by AI, kicker ability |
| Akroan Crusader | Verified | Heroic trigger creates Soldier token (trigger seen) |
| Muxus, Goblin Grandee | Verified | Cast by AI |
| Zada, Hedron Grinder | Working | Cast by AI |
| Akroan Hoplite | Working | Cast by AI |
| Goblin Sharpshooter | Verified | Activated ability: deals 1 damage (seen in gameplay) |
| Goblin Warchief | Working | Cast by AI, cost reduction |
| Goblin Trashmaster | Verified | Cast by AI |
| Shalai, Voice of Plenty | Working | Cast by AI |
| Arabella, Abandoned Doll | Verified | Cast via fixed controller, enters as 1/3 |

### Enchantments
| Card | Status | Notes |
|------|--------|-------|
| Legion's Landing | Verified | Transform trigger fires on 3+ attackers (trigger seen) |
| Thopter Arrest | Working | Exile enchantment |

### Commander
| Card | Status | Notes |
|------|--------|-------|
| Chandra, Torch of Defiance | Verified | Casts from command zone for 2RR, resolves, 2 loyalty abilities available, activated abilities shown in menu |
