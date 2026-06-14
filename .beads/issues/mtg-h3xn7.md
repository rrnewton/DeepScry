---
title: 'Card Compatibility: Wild Growth'
status: open
priority: 3
issue_type: task
created_at: 2026-06-14T12:31:35.271531445+00:00
updated_at: 2026-06-14T12:31:35.271531445+00:00
---

# Description

Test all behavioral aspects of Wild Growth in MTG Forge-rs.

Set: LEA
Card script: cardsfolder/w/wild_growth.txt
Oracle: Enchant land. Whenever enchanted land is tapped for mana, its controller adds an additional {G}.

Aspects (one per ability/keyword/cost):

1. [x] Card loads as an Enchantment Aura (Enchant land).
2. [x] Attaches to a land in puzzle state (Forest|Id:N + Wild Growth|AttachedTo:N).
3. [x] The enchanted Forest still taps for its base {G} (a 1-mana {G} spell casts fine).
4. [BROKEN] The 'additional {G}' (T:Mode$ TapsForMana) is NOT available to pay casting costs - see bug issue Created issue: mtg-l9ubr. A single enchanted Forest cannot pay {G}{G}, though Forest {G} + extra {G} should.

Findings (2026-06-14_#3469(2d7639fd1)) - PARTIAL. Loads/attaches/base-tap WORK; the mana-boost trigger does not contribute usable mana during cost payment. NOT puzzle-tested (the WORKING aspect cannot be cleanly puzzle-asserted without a mana assertion, and the headline aspect is broken). Tracked by bug Created issue: mtg-l9ubr.

Decisive control-vs-test reproducer is in Created issue: mtg-l9ubr.
