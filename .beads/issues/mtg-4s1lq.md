---
title: Commander format support
status: open
priority: 1
issue_type: feature
created_at: 2026-04-01T02:43:55.841215877+00:00
updated_at: 2026-04-01T07:21:55.876142045+00:00
---

# Description

## Commander Format Support

Tracking issue for Commander (EDH) format support. Deck: decks/commander/chandra_tokens.dck

## Implementation Status (2026-04-01_#2043(430124e4))

### Core Infrastructure - COMPLETE
- [x] [Commander] deck section, per-player command zone, commander tracking
- [x] Auto-detect commander (40 life), cast from command zone
- [x] Commander tax ({2} per cast), zone-change replacement
- [x] Commander damage (21+ combat damage = loss)
- [x] TUI display (fancy: bottom-left overlay, simple: zone summary)

### Planeswalker Loyalty - COMPLETE
- [x] Loyalty:N, +/-N costs, starting counters, 0-loyalty death
- [x] Full lifecycle: cast -> abilities -> death -> command zone -> re-cast with tax

### Token System - COMPLETE
- [x] is_token field, TokenCreaturesYouControl selector
- [x] Intangible Virtue +1/+1 to tokens verified in combat

### Heuristic AI - COMPLETE
- [x] Evaluates CastFromCommand, casts planeswalkers
- [x] Prioritizes mana rocks (Sol Ring, Arcane Signet) in early game turns 1-5
- [x] Modal choice for CastFromExile (Charm spells from exile)

### Bugs Fixed (12)
1. Token ownership always Player 1 (c2df44a9)
2. Token script pre-loading (c1016144)
3. Fixed controller cast matching (c1016144)
4. Planeswalker tapped for mana (4f134568)
5. Loyalty costs parsed as mana (6f4e41c1)
6. No starting loyalty counters (6f4e41c1)
7. Heuristic AI ignores CastFromCommand (cd5509c1)
8. Heuristic AI never casts planeswalkers (cd5509c1)
9. Token creature selectors unimplemented (d66cddde)
10. ModalChoice for CastFromExile (b21951b6)
11. Heuristic AI doesn't prioritize mana rocks (430124e4)
12. CI formatting (multiple)

### Testing
- 634+ lib tests, 13 e2e tests, 30+ random seeds stable (0 crashes)
- Zero ModalChoice warnings
- Benchmarks: 60K+ games/sec, no regression
