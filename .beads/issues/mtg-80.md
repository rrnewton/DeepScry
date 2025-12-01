---
title: Improve enchantment evaluation in GameStateEvaluator
status: closed
priority: 4
issue_type: task
created_at: 2025-10-26T21:06:34+00:00
updated_at: 2025-12-01T21:51:09.019878590+00:00
---

# Description

Properly evaluate enchantments based on what they're enchanting.

Reference: GameStateEvaluator.java:224-228

Implementation (2025-12-01):
- Added evaluate_enchantment() method in game_state_evaluator.rs
- Auras attached to creatures now evaluate based on their effects:
  - ModifyPT static abilities: +15 per power, +10 per toughness  
  - GrantKeyword static abilities: valued based on keyword type
- Global enchantments valued at 20 + 15*CMC as baseline
- Fallback for unparsed auras: 15*CMC

Follows Java approach of only counting the enchantment's effect on what it's enchanting, avoiding double-counting of abilities already present.

Unit tests added:
- test_enchantment_evaluation_pump_aura
- test_enchantment_evaluation_keyword_grant_aura  
- test_enchantment_evaluation_global_enchantment
