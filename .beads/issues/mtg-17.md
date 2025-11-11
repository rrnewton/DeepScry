---
title: Equipment artifacts (need equip abilities)
status: open
priority: 3
issue_type: feature
created_at: 2025-10-26T21:06:34+00:00
updated_at: 2025-11-11T01:07:33.980742161+00:00
---

# Description

Implement Equipment artifacts with equip abilities.

## Current Status (2025-11-11 #906)

**✅ BASIC EQUIPMENT COMPLETE** - See mtg-98df7d for full implementation details.

**What Works**:
- ✅ Equip keyword and cost parsing (K:Equip:X)
- ✅ Attachment to creatures (bidirectional references)
- ✅ Equipment granting power/toughness bonuses to equipped creature
- ✅ Unequipping when creature leaves battlefield
- ✅ Equip activated ability generation with correct cost
- ✅ Target validation (creatures you control)
- ✅ Sorcery-speed timing restriction
- ✅ CR 613 layer system for buff calculation
- ✅ Works with real cards from cardsfolder

**Test Coverage**:
- 13 Equipment tests across unit and integration
- E2E test validates full workflow
- Real card test validates Bonesplitter and Accorder's Shield
- All tests passing

**Remaining Work** (Advanced Features):
- [ ] **Keyword Granting**: Static abilities that grant keywords (Vigilance, Flying, etc.)
  - Currently parsed but not applied
  - Blocked by mtg-20 (general static abilities system)
  - Example: Accorder's Shield grants Vigilance
- [ ] **Reconfigure**: Kamigawa Neon Dynasty mechanic
  - Equipment can become creatures
  - Requires additional rules implementation
- [ ] **Living Weapon**: Equipment that creates tokens
  - Example: Batterskull
  - Requires ETB trigger support
- [ ] **Auto-attach**: Some Equipment attach when entering battlefield
  - Requires special-case handling
- [ ] **Move Equipment**: Abilities that move Equipment between creatures
  - Example: Brass Squire
  - Requires ability activation through game loop

**Priority Assessment**:
- Basic Equipment (P/T bonuses) is COMPLETE and working
- Keyword granting is most important next step but blocked by mtg-20
- Other advanced features can wait

**Recommendation**:
- Keep this issue open to track advanced Equipment features
- Next: Implement general static abilities system (mtg-20)
- Then: Keyword granting for Equipment
