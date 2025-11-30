---
title: 'Bug: Vibrant Cityscape tutor ability doesn''t search library (fizzles)'
status: open
priority: 2
issue_type: bug
created_at: 2025-11-30T01:38:10.628973363+00:00
updated_at: 2025-11-30T01:38:10.628973363+00:00
---

# Description

## Bug Report

**Card:** Vibrant Cityscape (Land)
**Deck:** decks/ryan_spiderman_draft.dck or decks/julian_spiderman_draft.dck

### Expected Behavior

Vibrant Cityscape has the activated ability:
- "{T}, Sacrifice this land: Search your library for a basic land card, put it onto the battlefield tapped, then shuffle."

When activated, the player should:
1. Be prompted to search their library
2. Choose a basic land card (or decline)
3. The chosen land enters the battlefield tapped
4. The library is shuffled

### Actual Behavior

When Vibrant Cityscape's ability is activated:
1. The player is given the option to activate it
2. Upon activation, the ability fizzles - no library search occurs
3. No basic land is put onto the battlefield
4. The land is sacrificed (cost is paid) but the effect does nothing

### Root Cause Hypothesis

Possible issues:
1. **Library search not implemented:** The ChangeZone effect with Origin=Library may not support player choice/searching
2. **Missing player interaction:** The game may not be prompting for the tutoring choice
3. **Ability parsing issue:** The ability script may not be fully parsed/executed

This is likely a **missing feature** rather than a bug in existing code - library searching/tutoring effects may not be implemented yet.

### Card Definition Reference

```
Name:Vibrant Cityscape
ManaCost:no cost
Types:Land
A:AB$ ChangeZone | Cost$ T Sac<1/CARDNAME> | Origin$ Library | Destination$ Battlefield | Tapped$ True | ChangeType$ Land.Basic | ChangeTypeDesc$ basic land | SpellDescription$ Search your library for a basic land card, put it onto the battlefield tapped, then shuffle.
```

### Reproduction

1. Play `./target/release/mtg tui --p1=fancy decks/ryan_spiderman_draft.dck decks/julian_spiderman_draft.dck`
2. Play Vibrant Cityscape
3. Activate its ability (tap and sacrifice)
4. Observe: No library search occurs, ability fizzles

### Impact

**Severity:** High (for this card)
- Makes Vibrant Cityscape completely useless
- Affects all tutoring/library search effects
- Common mechanic in Magic (fetchlands, tutors, etc.)

**Broader Impact:** Medium-High
- Library searching is a core Magic mechanic
- Affects many cards (fetchlands, Demonic Tutor, Evolving Wilds, etc.)
- May require significant player interaction infrastructure

### Related Issues/Features

- May need new player interaction type: "Choose from library"
- May need library viewing UI in TUI
- Related to general tutoring/searching mechanics

### Technical Notes

**ChangeZone with Origin=Library:**
- Requires player to search/view library
- Requires filtering by ChangeType (e.g., Land.Basic)
- Requires putting chosen card in Destination zone
- Requires shuffling library afterward

**Possible implementation:**
1. Add PlayerChoice::ChooseFromLibrary variant
2. Filter library by ChangeType
3. Present filtered cards to player
4. Move chosen card to Destination
5. Shuffle library

### Next Steps

1. Check if ChangeZone Origin=Library is implemented at all
2. Check if player library searching is implemented
3. Implement library search player interaction if missing
4. Add test case for basic tutoring effect
5. Consider adding library view to TUI (mtg-122 may be related)
