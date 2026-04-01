---
title: Commander format support
status: open
priority: 1
issue_type: feature
created_at: 2026-04-01T02:43:55.841215877+00:00
updated_at: 2026-04-01T06:44:16.021557978+00:00
---

# Description

## Commander Format Support

Tracking issue for Commander (EDH) format support. Deck: decks/commander/chandra_tokens.dck

## Implementation Status (2026-04-01_#2036(74b6ec2c))

### Core Infrastructure - COMPLETE
- [x] [Commander] deck section, per-player command zone, commander tracking
- [x] Auto-detect commander (40 life), cast from command zone
- [x] Commander tax ({2} per cast), zone-change replacement (graveyard/exile -> command)
- [x] Commander damage (21+ combat damage = loss)
- [x] TUI display (fancy: bottom-left overlay, simple: zone summary)

### Planeswalker Loyalty - COMPLETE (iteration 3-4)
- [x] Parse Loyalty:N field from card scripts
- [x] Cost::AddLoyalty / Cost::SubLoyalty variants
- [x] Starting loyalty counters on ETB (Chandra enters with 4)
- [x] Loyalty ability activation (+1, -3, -7 costs)
- [x] 0-loyalty death (state-based action, MTG CR 704.5i)
- [x] Planeswalker mana source exclusion (not tapped for mana)
- [x] Full lifecycle: cast -> use abilities -> die -> command zone -> re-cast with tax

### Heuristic AI - COMPLETE
- [x] Evaluates CastFromCommand alongside CastSpell
- [x] Always casts planeswalkers (ongoing value)
- [x] Both AIs now cast Chandra from command zone

### Bugs Fixed (across all iterations)
1. Token ownership always Player 1 (c2df44a9) - CRITICAL
2. Token script pre-loading for A:/T: lines (c1016144)
3. Fixed controller "cast" matching for command zone (c1016144)
4. Planeswalker tapped for mana (4f134568) - CRITICAL
5. Loyalty costs parsed as mana costs (6f4e41c1)
6. No starting loyalty counters (6f4e41c1)
7. Heuristic AI ignores CastFromCommand (cd5509c1)
8. Heuristic AI never casts planeswalkers (cd5509c1)
9. CI formatting + clippy (0c18b5e1, d0e0d98f)

### Testing
- 634+ lib tests, 13 e2e tests (commander_e2e.sh)
- 20 random games complete (13-31 turns), 20 more verified stable
- Heuristic AI: both players cast Chandra, full loyalty lifecycle
- Benchmarks: no regression (60K+ games/sec)

### Known Remaining Work
- [ ] Player choice for zone-change replacement
- [ ] Chandra +1 exile-and-cast not fully resolving (Dig+Play+Damage chain)
- [ ] Modal spell targeting (Boros Charm targets creatures, pre-existing)
