---
title: 'Card Compatibility: Stock Up'
status: open
priority: 3
issue_type: task
created_at: 2026-06-06T04:35:18.986239891+00:00
updated_at: 2026-06-06T04:35:18.986239891+00:00
---

# Description

Test all behavioral aspects of Stock Up in MTG Forge-rs.

Card: cardsfolder/s/stock_up.txt
Set: ATLA / 2025 Standard
Deck: 02 Shibata, 03 Davis Izzet Lessons (2025 WC)

Card text:
  2U  Sorcery
  Look at the top five cards of your library. Put two of them into your hand and the rest on the bottom of your library in any order.

Findings (2026-06-05_#3008(50175e06)):

1. [x] Parses: cost 2U, Sorcery
2. [x] Looks at top 5 cards (log: "Zero1 looks at the top 5 cards of their library")
3. [x] Puts 2 into hand ("Zero1 puts <card> into Hand" x2)
4. [x] Puts 3 on bottom ("Zero1 puts 3 cards on the bottom of their library")
5. [NOTE] Log says "Stock Up (0) digs 5 card(s) from opponent's library to hand" — incorrect log message (should say own library), but behavior is correct.

Reproducer:
```sh
./target/release/mtg tui --p1 zero --p2 zero --p1-draw "Stock Up;Island;Island;Island;Island;Island;Island" --p2-draw "Island;Island;Island;Island;Island;Island;Island" --seed 42 --verbosity 3 decks/championship/2025/03_davis_izzet_lessons.dck decks/championship/2025/03_davis_izzet_lessons.dck 2>&1 | grep -E "Stock|top 5|puts|bottom"
```

Expected:
```
Zero1 looks at the top 5 cards of their library
Zero1 puts <card> into Hand
Zero1 puts <card> into Hand
Zero1 puts 3 cards on the bottom of their library
```
Actual: Confirmed working with minor log message issue.

CARD STATUS: WORKING — dig 5, put 2 in hand works correctly (minor log message issue: says "opponent's library")
