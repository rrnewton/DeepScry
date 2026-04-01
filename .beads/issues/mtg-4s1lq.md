---
title: Commander format support
status: open
priority: 1
issue_type: feature
created_at: 2026-04-01T02:43:55.841215877+00:00
updated_at: 2026-04-01T04:19:59.995185539+00:00
---

# Description

## Commander Format Support

Tracking issue for implementing Commander (EDH) format support in the MTG engine.

## Commander Rules (MTG CR 903)
- 100-card singleton decks (including commander)
- Designated legendary creature (or planeswalker with "can be your commander") as commander
- Commander starts in the command zone
- Starting life: 40
- Commander can be cast from the command zone
- Commander tax: costs {2} more for each previous cast from command zone
- When commander would go to graveyard or exile, owner may put it in command zone instead
- Commander damage: 21+ combat damage from a single commander = lose
- Color identity: deck cards must match commander's color identity

## Implementation Status (2026-03-31_#2022(723ed2fe))

### Phase 1: Core Infrastructure - DONE
- [x] Create deck file with [Commander] section support
- [x] Add command zone to PlayerZones (per-player)
- [x] Add commander tracking to Player (commander_id, commander_tax, commander_damage)
- [x] Add is_commander_game flag to GameState
- [x] Support 40 starting life (auto-detected from deck)
- [x] Parse [Commander] section in deck loader
- [x] Card.is_commander field for commander designation

### Phase 2: Commander Mechanics - DONE
- [x] Cast commander from command zone (SpellAbility::CastFromCommand)
- [x] Commander tax calculation and payment ({2} per cast count)
- [x] Zone-change replacement effect (graveyard/exile -> command zone, automatic)
- [x] Commander damage tracking and loss condition (21+ combat damage)
- [ ] Player choice for zone replacement (currently automatic - always returns to command zone)
- [ ] Color identity validation (warning only for now)

### Phase 3: TUI and Display - DONE
- [x] Simple TUI: Command zone shown in zone summary line
- [x] Fancy TUI: Command zone overlay in bottom-left of battlefield (magenta)
- [ ] Commander tax indicator in casting cost display (shows in action menu)
- [ ] Commander damage display in player info panel

### Phase 4: Testing - DONE
- [x] Commander deck: decks/commander/chandra_tokens.dck (99 + 1 commander)
- [x] E2E test: tests/commander_e2e.sh (deck loading, 40 life, casting, 5-seed stability)
- [x] Random controller tests across 5+ seeds (20-35 turns each, all complete)
- [x] Heuristic AI: works correctly (token swarm gameplay, 20-turn wins)
- [x] 632 library tests pass, 13 shell script e2e tests pass
- [x] All 66 unique cards in deck have script files in cardsfolder
- [ ] Card-by-card mechanics validation (playtest checklist in ai_docs/)

## Test Deck
- decks/commander/chandra_tokens.dck - Boros token-swarm with Chandra, Torch of Defiance

## Related Issues
- mtg-3: MTG feature completeness
- mtg-4: Gameplay features (TUI)
- mtg-147: Affected$ selectors (commander selectors)
