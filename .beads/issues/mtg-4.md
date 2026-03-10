---
title: 'Gameplay features: TUI, human play, controls'
status: open
priority: 1
issue_type: epic
labels:
- tracking
created_at: 2025-10-26T21:06:34+00:00
updated_at: 2026-01-02T20:06:08.227468534+00:00
---

# Description

Track user-facing gameplay features and interaction improvements.

**TUI (Terminal User Interface):**
- Current: ✅  command with --p1/--p2 agent types (zero/random), --seed for deterministic games
- mtg-25: Interactive TUI controller (--p1=tui) for human play
- mtg-26: Display game state during play (life, hand, battlefield)
- mtg-27: Show available actions to player
- mtg-28: Better formatting and colors in output
- mtg-29: Game state visualization improvements
- mtg-p9svf: Agentplay CLI turn sequence and display bugs (HIGH - blocks usability)
- mtg-el58f: Combat attack action not available during Declare Attackers phase (CRITICAL - blocks combat testing)

**Advanced gameplay mechanics:**
- mtg-30: Stack interaction (responding to spells at instant speed)
- mtg-31: Card draw triggers and replacement effects
- mtg-32: Discard mechanics beyond cleanup step
- mtg-33: Graveyard interactions (flashback, recursion)
- ✅ mtg-34: Token creation (CLOSED - fully implemented)
- mtg-35: +1/+1 and -1/-1 counters on creatures

---
Checked up-to-date as of 2026-03-10_#1898(7de2da0).

**Serialization & Testing:**
- mtg-36: GameState text file format (.pzl files)
- mtg-37: Load game states from files for testing
- mtg-38: Puzzle mode for testing specific scenarios
- mtg-39: Replay recorded games from file
