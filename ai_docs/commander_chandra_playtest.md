# Commander Chandra Tokens Deck - Playtest Checklist

Deck: `decks/commander/chandra_tokens.dck`
Tracking issue: mtg-4s1lq

## Commander Mechanics
- [x] Commander starts in command zone
- [x] Starting life is 40
- [x] Commander can be cast from command zone
- [x] Commander tax increases cost by {2} per cast
- [x] Commander returns to command zone when would go to graveyard/exile
- [ ] Commander damage tracking (21+ combat damage = loss)
- [ ] Player choice for zone replacement (currently automatic)

## Card Categories and Status

### Mana Sources / Ramp
| Card | Status | Notes |
|------|--------|-------|
| Sol Ring | Working | Taps for {C}{C} |
| Arcane Signet | Working | Taps for one mana of commander's color identity |
| Fellwar Stone | Working | Taps for mana of colors opponents can produce |
| Thought Vessel | Working | Taps for {C}, no maximum hand size |
| Snow-Covered Plains | Working | Basic land |
| Snow-Covered Mountain | Working | Basic land |
| Reliquary Tower | Working | No maximum hand size |

### Token Producers
| Card | Status | Notes |
|------|--------|-------|
| Raise the Alarm | Working | Creates 2 1/1 Soldier tokens |
| Dragon Fodder | Working | Creates 2 1/1 Goblin tokens |
| Hordeling Outburst | Working | Creates 3 1/1 Goblin tokens |
| Secure the Wastes | TODO | X spell - creates X 1/1 Warrior tokens |
| Tempt with Vengeance | TODO | X spell - tempt with vengeance |
| Oketra's Monument | Working | Triggers on creature cast, creates 1/1 Warrior |
| Anim Pakal, Thousandth Moon | Working | Creates Gnome tokens on attack |
| Legion Warboss | TODO | Creates Goblin tokens |
| Krenko, Mob Boss | TODO | Taps to create Goblins equal to Goblins you control |
| Goblin Rabblemaster | TODO | Creates attacking Goblin tokens |
| Swarming Goblins | TODO | ETB create Goblin tokens |
| Siege-Gang Commander | TODO | ETB create Goblin tokens |
| Siege-Gang Lieutenant | TODO | ETB create Goblin tokens |
| Assemble the Legion | TODO | Enchantment - accumulates counters, creates tokens |
| Clarion Spirit | TODO | Triggers on second spell per turn |
| Ocelot Pride | TODO | Creates Cat tokens |
| Thopter Architect | TODO | Creates Thopter tokens |
| Thopter Engineer | TODO | ETB creates Thopter token |

### Token/Creature Buffers
| Card | Status | Notes |
|------|--------|-------|
| Intangible Virtue | Working | +1/+1 and vigilance to tokens |
| Goldnight Commander | TODO | Triggers on creature ETB, gives +1/+1 |
| Warleader's Call | TODO | Creatures ETB deal damage |
| Rosie Cotton of South Lane | TODO | Token creation triggers |

### Card Draw
| Card | Status | Notes |
|------|--------|-------|
| Mentor of the Meek | TODO | Draw when creature with power 2 or less enters |
| Bennie Bracks, Zoologist | TODO | Draw when token created |
| Welcoming Vampire | TODO | Draw when creature enters (once per turn) |
| Idol of Oblivion | Working | Tap to draw if token created this turn |
| Skullclamp | TODO | Equipped creature gets +1/-1, draw 2 on death |
| Slate of Ancestry | TODO | Tap, discard hand, draw equal to creatures |
| Knollspine Dragon | TODO | ETB draw cards equal to damage dealt |
| Tocasia's Welcome | TODO | Draw when creature with MV 3 or less enters |

### Instants/Sorceries
| Card | Status | Notes |
|------|--------|-------|
| Boros Charm | Working | Modal - 4 damage, double strike, or indestructible |
| Dawn Charm | Working | Modal - fog, regenerate, or counter spell |
| Brave the Elements | TODO | Protection from color for white creatures |
| Apostle's Blessing | TODO | Protection from color or artifact |
| Mana Tithe | TODO | Counter unless opponent pays {1} |
| Rebuff the Wicked | TODO | Counter spell targeting permanent you control |
| Deflecting Swat | TODO | Free if commander on battlefield |
| Molten Influence | Working | Counter or take 4 damage |
| Return the Favor | TODO | Copy or redirect |
| Searing Barrage | Working | Deals damage |
| Sizzling Barrage | TODO | Deals damage to blocking/blocked creature |
| Goblin Barrage | TODO | Deals damage, kicker to sac |
| Heroic Reinforcements | TODO | Creates tokens and gives +1/+1 |
| Untimely Malfunction | TODO | Counter artifact or creature, create Clue |

### Equipment/Artifacts
| Card | Status | Notes |
|------|--------|-------|
| Swiftfoot Boots | Working | Hexproof and haste |
| Spellbook | Working | No maximum hand size |
| Decanter of Endless Water | TODO | No max hand size, draw extra |

### Creatures with Abilities
| Card | Status | Notes |
|------|--------|-------|
| Goblin Bushwhacker | TODO | Kicker - haste to all creatures |
| Akroan Crusader | TODO | Heroic - creates Soldier token |
| Muxus, Goblin Grandee | TODO | ETB reveal top 6, put Goblins on battlefield |
| Zada, Hedron Grinder | TODO | Copies spells targeting it to all creatures |
| Akroan Hoplite | TODO | Gets +X/+0 for attacking creatures |
| Goblin Sharpshooter | TODO | Untaps when creature dies, taps to deal 1 |
| Goblin Warchief | TODO | Goblins cost less and have haste |
| Goblin Trashmaster | TODO | Sac Goblin to destroy artifact |
| Shalai, Voice of Plenty | TODO | Hexproof to you and permanents |

### Enchantments
| Card | Status | Notes |
|------|--------|-------|
| Legion's Landing | TODO | Flip card - creates token, flips to land |
| Thopter Arrest | TODO | Exile target artifact or creature |

### Commander
| Card | Status | Notes |
|------|--------|-------|
| Chandra, Torch of Defiance | Partial | Planeswalker - loyalty abilities need testing |
