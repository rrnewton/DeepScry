# MTG Forge Card Script Specification

This document describes the Domain-Specific Language (DSL) used in Forge card scripts found in `cardsfolder/`.

## Overview

Forge card scripts define card properties and abilities using a key-value text format. Each card is stored in a `.txt` file with fields defining its properties.

## Basic Card Structure

```
Name:<card name>
ManaCost:<mana cost>
Types:<card types>
[PT:<power>/<toughness>]        # For creatures
[Loyalty:<loyalty>]              # For planeswalkers
[K:<keyword>]                    # Keyword abilities
[A:<ability script>]             # Activated/Spell abilities
[T:<trigger script>]             # Triggered abilities
[S:<static ability script>]      # Static abilities
[SVar:<variable name>:<value>]   # Script variables
[DeckHas:<deck constraints>]     # Deck building hints
Oracle:<oracle text>             # Official rules text
```

## Field Descriptions

### Name
The card's name.
```
Name:Lightning Bolt
```

### ManaCost
Mana cost using Forge notation:
- Numbers: Generic mana (1, 2, 3, etc.)
- Letters: Colored mana (W=White, U=Blue, B=Black, R=Red, G=Green)
- `no cost`: For lands and tokens
- Hybrid: `W/U`, `2/W`, etc.
- Phyrexian: `W/P`, `U/P`, etc.

Examples:
```
ManaCost:R                 # One red mana
ManaCost:2 U U             # Two generic, two blue
ManaCost:1 G               # One generic, one green
ManaCost:no cost           # Lands
```

### Types
Space-separated list of types and subtypes.

```
Types:Instant
Types:Creature Elf Druid
Types:Basic Land Mountain
Types:Legendary Planeswalker Jace
```

### PT (Power/Toughness)
For creatures only.
```
PT:2/2
PT:1/1
PT:4/4
```

### Loyalty
For planeswalkers only.
```
Loyalty:3
Loyalty:4
```

## Ability Scripts

Abilities use a pipe-delimited (|) format with a prefix indicating the ability type.

### Ability Prefixes

- **A:** Activated ability or Spell ability
  - `SP$` - Spell ability (for instants/sorceries)
  - `AB$` - Activated ability (requires Cost$)
- **T:** Triggered ability
- **S:** Static ability
- **K:** Keyword ability (simple form)

### Common Ability Keywords (K:)

Simple keywords without parameters:
```
K:Flying
K:Vigilance
K:First Strike
K:Double Strike
K:Trample
K:Haste
K:Lifelink
K:Deathtouch
```

### Spell Abilities (A:SP$)

Format: `A:SP$ <API> | <Parameter>$ <Value> | ... | SpellDescription$ <text>`

Common APIs:
- **DealDamage**: Deal damage to targets
- **Counter**: Counter spells
- **Draw**: Draw cards
- **Destroy**: Destroy permanents
- **ChangeZone**: Move cards between zones
- **GainLife**: Gain life
- **Pump**: Modify power/toughness

Example - Lightning Bolt:
```
A:SP$ DealDamage | ValidTgts$ Any | NumDmg$ 3 | SpellDescription$ CARDNAME deals 3 damage to any target.
```

Parameters:
- `ValidTgts$`: What can be targeted (Any, Creature, Player, etc.)
- `NumDmg$`: Amount of damage
- `SpellDescription$`: Rules text (CARDNAME replaced with card name)

Example - Counterspell:
```
A:SP$ Counter | TargetType$ Spell | ValidTgts$ Card | SpellDescription$ Counter target spell.
```

Example - Ancestral Recall:
```
A:SP$ Draw | NumCards$ 3 | ValidTgts$ Player | TgtPrompt$ Select target player | SpellDescription$ Target player draws three cards.
```

### Activated Abilities (A:AB$)

Format: `A:AB$ <API> | Cost$ <cost> | <parameters> | SpellDescription$ <text>`

The `Cost$` parameter defines what must be paid to activate.

Common costs:
- `T` - Tap this permanent
- `Sac<N/Type>` - Sacrifice N permanents of Type
- `<N> <Color>` - Pay mana
- `AddCounter<N/TYPE>` - Add counters (planeswalkers)
- `SubCounter<N/TYPE>` - Remove counters (planeswalkers)

Example - Llanowar Elves:
```
A:AB$ Mana | Cost$ T | Produced$ G | SpellDescription$ Add {G}.
```

Example - Prodigal Sorcerer:
```
A:AB$ DealDamage | Cost$ T | ValidTgts$ Any | NumDmg$ 1 | SpellDescription$ CARDNAME deals 1 damage to any target.
```

Example - Black Lotus:
```
A:AB$ Mana | Cost$ T Sac<1/CARDNAME> | Produced$ Any | Amount$ 3 | AILogic$ BlackLotus | SpellDescription$ Add three mana of any one color.
```

### Triggered Abilities (T:)

Format: `T:Mode$ <trigger type> | <conditions> | Execute$ <SVar> | TriggerDescription$ <text>`

Common trigger modes:
- `ChangesZone` - When cards change zones
- `Phase` - At the beginning/end of a phase
- `Attacks` - When a creature attacks
- `DamageDone` - When damage is dealt
- `SpellCast` - When a spell is cast

Example - Soul Warden:
```
T:Mode$ ChangesZone | Origin$ Any | Destination$ Battlefield | ValidCard$ Creature.Other | TriggerZones$ Battlefield | Execute$ TrigGainLife | TriggerDescription$ Whenever another creature enters, you gain 1 life.
SVar:TrigGainLife:DB$ GainLife | Defined$ You | LifeAmount$ 1
```

Parameters:
- `Mode$`: Type of trigger
- `Origin$` / `Destination$`: Zones for ChangesZone
- `ValidCard$`: What cards trigger this
- `TriggerZones$`: Where this ability works
- `Execute$`: References an SVar with the effect

### Script Variables (SVar:)

Define reusable values or sub-abilities.

Format: `SVar:<name>:<value>` or `SVar:<name>:DB$ <API> | <parameters>`

- `DB$` - "Do this" - defines a sub-ability
- Plain values - Store numbers, logic flags, etc.

Examples:
```
SVar:TrigGainLife:DB$ GainLife | Defined$ You | LifeAmount$ 1
SVar:NonCombatPriority:1
SVar:DBChangeZone:DB$ ChangeZone | Origin$ Hand | Destination$ Library | ChangeType$ Card | ChangeNum$ 2 | LibraryPosition$ 0 | Mandatory$ True
```

## Common Parameters

### Targeting
- `ValidTgts$`: What can be targeted
  - `Any` - Any target (player or permanent)
  - `Creature` - Any creature
  - `Player` - Any player
  - `Card` - Any card (for counters)
  - Modifiers: `.Other` (not this), `.YouCtrl` (you control), etc.

### Zones
- `Battlefield` - In play
- `Hand` - Player's hand
- `Graveyard` - Graveyard
- `Library` - Library/deck
- `Exile` - Exile zone
- `Stack` - On the stack
- `Command` - Command zone

### Definitions
- `You` - The controller
- `Opponent` - An opponent
- `Targeted` - The targeted object
- `Self` - This card
- `TriggeredCard` - Card that caused trigger

### Colors
- `W` - White
- `U` - Blue
- `B` - Black
- `R` - Red
- `G` - Green
- `Any` - Any single color
- `Each` - One mana of each color

## Complex Examples

### Jace, the Mind Sculptor (Planeswalker)

```
Name:Jace, the Mind Sculptor
ManaCost:2 U U
Types:Legendary Planeswalker Jace
Loyalty:3
A:AB$ Dig | Cost$ AddCounter<2/LOYALTY> | ValidTgts$ Player | TgtPrompt$ Select target player | DigNum$ 1 | AnyNumber$ True | DestinationZone$ Library | LibraryPosition2$ 0 | Planeswalker$ True | SpellDescription$ Look at the top card of target player's library. You may put that card on the bottom of that player's library.
A:AB$ Draw | Cost$ AddCounter<0/LOYALTY> | NumCards$ 3 | SubAbility$ DBChangeZone | Planeswalker$ True | SpellDescription$ Draw three cards, then put two cards from your hand on top of your library in any order.
SVar:DBChangeZone:DB$ ChangeZone | Origin$ Hand | Destination$ Library | ChangeType$ Card | ChangeNum$ 2 | LibraryPosition$ 0 | Mandatory$ True
A:AB$ ChangeZone | Cost$ SubCounter<1/LOYALTY> | Origin$ Battlefield | Destination$ Hand | ValidTgts$ Creature | TgtPrompt$ Select target creature | Planeswalker$ True | SpellDescription$ Return target creature to its owner's hand.
A:AB$ ChangeZoneAll | Cost$ SubCounter<12/LOYALTY> | Origin$ Library | Destination$ Exile | ValidTgts$ Player | TgtPrompt$ Select target player | SubAbility$ DBChangeZone2 | Planeswalker$ True | Ultimate$ True | SpellDescription$ Exile all cards from target player's library, then that player shuffles their hand into their library.
SVar:DBChangeZone2:DB$ ChangeZoneAll | Origin$ Hand | Destination$ Library | Defined$ Targeted | ChangeType$ Card | Shuffle$ True
```

## Implementation Status (Rust Parser)

### ✅ Currently Implemented
- Basic card loading (Name, ManaCost, Types, PT)
- Simple DealDamage parsing from `A:SP$` lines
- Extracts `NumDmg$` parameter
- Creates Effect::DealDamage with TargetRef::None

### ❌ Not Yet Implemented

#### High Priority (needed for common cards)
- [ ] Keyword abilities (K: lines) - Flying, Vigilance, etc.
- [ ] ValidTgts$ parsing (Any, Creature, Player)
- [ ] Activated abilities (AB$ with Cost$)
- [ ] Mana abilities (AB$ Mana)
- [ ] Draw effects (SP$ Draw)
- [ ] Counter effects (SP$ Counter)

#### Medium Priority (common mechanics)
- [ ] Triggered abilities (T: lines)
- [ ] Static abilities (S: lines)
- [ ] ChangeZone effects
- [ ] GainLife effects
- [ ] Destroy effects
- [ ] Pump effects (modify P/T)

#### Low Priority (advanced features)
- [ ] SVar resolution and DB$ sub-abilities
- [ ] Planeswalker loyalty abilities
- [ ] Complex targeting with modifiers (.Other, .YouCtrl)
- [ ] Modal abilities (player chooses one)
- [ ] Replacement effects
- [ ] Continuous effects

### Parser Expansion Plan

1. **Phase 1**: Keywords (K:) - Simple, no parameters
2. **Phase 2**: Activated abilities (AB$) with basic costs
3. **Phase 3**: More spell effects (Draw, Counter, Destroy)
4. **Phase 4**: Triggered abilities (T:) - Common triggers
5. **Phase 5**: Advanced targeting and modifiers

## References

- Card files: `./forge-java/forge-gui/res/cardsfolder/`
- Total cards: ~31,000
- Java parser: `forge-java/forge-gui/src/main/java/forge/card/CardScriptInfo.java` (likely location)

## Notes

- The DSL is case-sensitive
- Whitespace around `|` separators is ignored
- `CARDNAME` in descriptions is replaced with the card's name
- Multiple abilities of the same type (A:, T:, K:) are listed on separate lines
- The Oracle field contains official Wizards text but isn't parsed - it's for reference
