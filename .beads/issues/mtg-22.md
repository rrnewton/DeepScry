---
title: Target validation (legal targets)
status: open
priority: 3
issue_type: feature
created_at: 2025-10-26T21:06:34+00:00
updated_at: 2025-11-30T14:11:46.714696331+00:00
---

# Description

Implement proper target validation:
- ✅ Check if target is legal based on ValidTgts$ parameter for basic types (commit ad081e1+)
  - spell_targets_land: "target land" only allows lands (e.g., Sinkhole)
  - spell_targets_creature: "target creature", "target nonartifact" only allow creatures (e.g., Terror)
  - spell_targets_any: "any target" allows creatures (e.g., Lightning Bolt)
  - spell_targets_player: "target player", "target opponent"
- ✅ Verify target hasn't been hexproof/shroud protected (was already implemented)
- ⬜ Handle "target creature or player" vs "target creature" distinctions (partial - needs work for player targets)
- ⬜ Protection from color/type checking (not yet implemented)
- ⬜ Terror-style "nonblack", "nonartifact" restrictions (not yet enforced at spell level)

**2025-11-30 Update (commit 988):**
Fixed bug where Sinkhole could target creatures instead of only lands. Added CardCache fields (spell_targets_land, spell_targets_creature, spell_targets_player, spell_targets_any) that are parsed from oracle text and used in get_valid_targets_for_spell().
