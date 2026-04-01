---
title: Commander format support
status: open
priority: 1
issue_type: feature
created_at: 2026-04-01T02:43:55.841215877+00:00
updated_at: 2026-04-01T05:47:19.837345735+00:00
---

# Description

## Commander Format Support

Tracking issue for implementing Commander (EDH) format support in the MTG engine.

## Implementation Status (2026-04-01_#2027(d0e0d98f))

### Core Infrastructure - COMPLETE
- [x] [Commander] section in .dck deck loader
- [x] Per-player command zone in PlayerZones
- [x] Commander tracking (commander_id, cast_count, damage)
- [x] is_commander_game flag on GameState
- [x] Auto-detect commander format (40 starting life)
- [x] Cast from command zone (SpellAbility::CastFromCommand)
- [x] Commander tax ({2} per cast, unit tested)
- [x] Zone-change replacement (graveyard/exile -> command zone)
- [x] Commander damage (21+ combat damage = loss, unit tested)
- [x] Fixed controller "cast" matches CastFromCommand
- [x] Fancy TUI command zone overlay (bottom-left)
- [x] Simple TUI command zone in zone summary

### Bugs Fixed
1. **Token ownership (c2df44a9)**: Critical - tokens always created under P1
2. **Token script loading (c1016144)**: A: and T: lines not scanned
3. **Cast matching (c1016144)**: Fixed controller couldn't cast from command zone
4. **CI formatting (0c18b5e1)**: display.rs chain formatting
5. **CI clippy (d0e0d98f)**: Wildcard enum match arm

### Testing
- 634+ library tests, 13 e2e tests (commander_e2e.sh)
- 20 random-seeded games complete (18-75 turns, 0 crashes)
- 10 heuristic AI games (0 token warnings after fix)
- Fixed controller tests: Sol Ring, Oketra trigger, Chandra casting
- 42/66 unique cards observed played across 10 games
- Benchmarks: no regression (60K+ games/sec simple_bolt)

### Known Remaining Work
- [ ] Player choice for zone-change replacement (currently automatic)
- [ ] Color identity validation
- [ ] Intangible Virtue display shows 1/1 not 2/2 in attacker list (display-only)
- [ ] Heuristic AI doesn't prioritize mana rocks (Sol Ring etc.)
- [ ] Modal spell warnings (pre-existing, not commander-specific)

## Test Deck
- decks/commander/chandra_tokens.dck - Boros tokens, Chandra commander

## Related Issues
- mtg-3: MTG feature completeness
- mtg-4: Gameplay features (TUI)
