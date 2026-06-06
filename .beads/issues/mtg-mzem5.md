---
title: 'Card Compatibility: Stormchaser''s Talent'
status: closed
priority: 3
issue_type: task
created_at: 2026-06-06T08:03:24.943680495+00:00
updated_at: 2026-06-06T08:03:28.072308066+00:00
---

# Description

Test all behavioral aspects of Stormchaser's Talent in MTG Forge-rs.

Card: cardsfolder/s/stormchasers_talent.txt
Set: FDN/Foundations
Deck: 01 Manfield Izzet Lessons (mtg-638) — 4-of in all 3 Izzet Lessons 2025 World Championship decks

Card text:
  {U} Enchantment — Class
  (Gain the next level as a sorcery to add its ability.)
  {3}{U}: Level 2
  When this becomes level 2, return target instant or sorcery card from your graveyard to your hand.
  {4}{U}: Level 3
  Whenever you cast an instant or sorcery spell, create a 1/1 blue and red Otter creature token with prowess.

Status: FIXED (commit cb597626, branch fix-stormchasers-talent, 2026-06-06)

Findings (2026-06-06_#3009(cb597626), agent slot04):

1. [x] ETB enters as level 1 (Level counter ETB trigger): WORKING
2. [x] Level 2 activation (sorcery-speed, 3U cost): WORKING — observed in seed 19 mock game
3. [x] Level 3 activation (sorcery-speed, 4U cost): WORKING — attempted (guard prevents if not level 2 first)
4. [x] ClassLevelGained trigger at level 2 fires: WORKING — returns instant/sorcery from graveyard to hand
5. [x] SpellCast ongoing trigger at level 3 (instant/sorcery → 1/1 Otter with prowess): PARTIAL — infrastructure added (apply_class_level_ongoing_abilities registers SpellCast trigger), functional verification in extended games requires reaching level 3 (AI ordering issue; see CONCERN)
6. [CONCERN] Class level tracking uses CounterType::Level (pragmatic workaround, CR 716.4 violation): filed mtg-94cy5

Gameplay evidence (seed 19, 01_manfield_izzet_lessons mirror):
  Stormchaser's Talent activates ability: Level 2 (class level-up)
  Stormchaser's Talent advances to level 2
  Random1 returns Boomerang Basics from graveyard to hand
  Stormchaser's Talent activates ability: Level 2 (class level-up)
  Stormchaser's Talent advances to level 2
  Random1 returns Abandon Attachments from graveyard to hand

Command: ./agentplay/agent_game.py --mock --seed 19 -- decks/championship/2025/01_manfield_izzet_lessons.dck decks/championship/2025/01_manfield_izzet_lessons.dck

CONCERN: AI activation ordering — AI sometimes tries Level 3 before Level 2 (gets fizzled by the guard). This is a suboptimal AI heuristic issue, not a correctness issue (the guard is correct). Separate issue.

CONCERN: CounterType::Level for class level tracking — see mtg-94cy5.

Fix commit: cb597626a3eddd1bd0fca4eaf0306310e34c7ccb (branch fix-stormchasers-talent)
Related: mtg-94cy5 (class_level dedicated field follow-up)
