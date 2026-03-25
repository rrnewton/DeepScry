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

**TUI (Terminal User Interface) - ALL CORE FEATURES COMPLETE:**
- ✅ mtg-25: Interactive TUI controller (--p1=tui) - 1310-line InteractiveController (CLOSED)
- ✅ mtg-26: Display game state (life, hand, battlefield) (CLOSED)
- ✅ mtg-27: Show available actions (numbered list, rich text input) (CLOSED)
- ✅ mtg-28: Better formatting and colors (Fancy TUI, 4369-line renderer) (CLOSED)
- ✅ mtg-29: Game state visualization (battlefield layout, combat, stack) (CLOSED)
- mtg-p9svf: Agentplay CLI turn sequence and display bugs
- mtg-el58f: Combat attack action not available during Declare Attackers phase

**Advanced gameplay mechanics - ALL COMPLETE:**
- ✅ mtg-30: Stack interaction (instants, counterspells, priority) (CLOSED)
- ✅ mtg-31: Card draw triggers and replacement effects (CLOSED)
- ✅ mtg-32: Discard mechanics (DiscardCards effect, AI evaluation) (CLOSED)
- ✅ mtg-33: Graveyard interactions (Flashback, death triggers, ChangeZoneAll) (CLOSED)
- ✅ mtg-34: Token creation (CLOSED)
- ✅ mtg-35: +1/+1 and -1/-1 counters (PutCounter, MultiplyCounter, counter tracking) (CLOSED)

**Serialization & Testing - ALL COMPLETE:**
- ✅ mtg-36: GameState text file format (.pzl files) (CLOSED)
- ✅ mtg-37: Load game states from files (CLOSED)
- ✅ mtg-38: Puzzle mode for testing (CLOSED)
- ✅ mtg-39: Replay recorded games (CLOSED)

---
Checked up-to-date as of 2026-03-25_#1984(71aba549).
