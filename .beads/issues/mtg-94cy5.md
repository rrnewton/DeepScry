---
title: 'Class mechanic: use dedicated class_level field instead of CounterType::Level'
status: open
priority: 3
issue_type: task
created_at: 2026-06-06T07:42:16.170783371+00:00
updated_at: 2026-06-06T07:42:16.170783371+00:00
---

# Description

The Class mechanic implementation (fix-stormchasers-talent) uses CounterType::Level to track the class designation. Per CR 716.4, level counters (used by leveler cards, CR 702.87) do NOT interact with Class cards, and class levels do NOT interact with leveler cards. Using CounterType::Level is technically incorrect because:

1. Proliferate (CR 701.34c) adds one of each counter already on a permanent — this would incorrectly advance a Class level
2. Remove-counter effects could incorrectly de-level a Class card
3. Level-counter-granting effects (like the old Berserk trigger on Dragonlord Silumgar) could incorrectly advance Class levels

The fix: add a dedicated `class_level: u8` field to `core::Card` (analogous to Java Forge's `host.getClassLevel() / setClassLevel()`), and update:
- `execute_class_level_up` to read/write `card.class_level` instead of `CounterType::Level` counters
- `Card::get_counter` guard to not return `class_level` as a counter
- Serialization / undo / network delta code for the new field
- The ETB trigger in card.rs that sets initial class_level=1 (replace PutCounter with a direct field set)

For now, the CounterType::Level approach is a pragmatic workaround that works correctly for all current card scripts (no Proliferate or Level-counter interactions exist in the Izzet Lessons decks). The issue is filed for completeness. See TODO(mtg-TODO) in execute_class_level_up.
