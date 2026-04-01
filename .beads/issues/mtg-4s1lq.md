---
title: Commander format support
status: open
priority: 1
issue_type: feature
created_at: 2026-04-01T02:43:55.841215877+00:00
updated_at: 2026-04-01T07:05:38.349516838+00:00
---

# Description

## Commander Format Support

Tracking issue for Commander (EDH) format support. Deck: decks/commander/chandra_tokens.dck

## Implementation Status (2026-04-01_#2039(76fbbca8))

### Core Infrastructure - COMPLETE
- [x] [Commander] deck section, per-player command zone, commander tracking
- [x] Auto-detect commander (40 life), cast from command zone
- [x] Commander tax ({2} per cast), zone-change replacement
- [x] Commander damage (21+ combat damage = loss)
- [x] TUI display (fancy: bottom-left overlay, simple: zone summary)

### Planeswalker Loyalty - COMPLETE
- [x] Loyalty:N field, starting counters, +/-N abilities, 0-loyalty death
- [x] Not used as mana source
- [x] Full lifecycle verified: cast -> abilities -> death -> command -> re-cast with tax

### Heuristic AI - COMPLETE
- [x] Evaluates CastFromCommand, always casts planeswalkers

### Token System - COMPLETE
- [x] is_token field on Card, TokenCreaturesYouControl selector
- [x] Intangible Virtue buffs tokens (+1/+1 verified in combat: 70x 2dmg hits)

### Bugs Fixed (10)
1. Token ownership always Player 1 (c2df44a9) - CRITICAL
2. Token script pre-loading (c1016144)
3. Fixed controller cast matching (c1016144)
4. Planeswalker tapped for mana (4f134568) - CRITICAL
5. Loyalty costs parsed as mana (6f4e41c1)
6. No starting loyalty counters (6f4e41c1)
7. Heuristic AI ignores CastFromCommand (cd5509c1)
8. Heuristic AI never casts planeswalkers (cd5509c1)
9. Token creature selectors unimplemented (d66cddde)
10. CI formatting (multiple)

### Testing
- 634+ lib tests, 13 e2e tests, 30 random seeds stable (0 crashes)
- Heuristic AI: 105-turn mirror game, both AIs cast Chandra
- Benchmarks: 60K+ games/sec, no regression

### Known Remaining Issues
- ModalChoice not resolved for CastFromExile path (Boros Charm from exile skips mode selection)
  Root cause: CastFromExile handler in priority.rs lacks mode selection logic
  This is pre-existing and affects all Charm spells cast from exile, not commander-specific
- Player choice for zone-change replacement (currently automatic)
- Chandra +1 conditional damage chain complex (Dig->Play->conditional Damage)
