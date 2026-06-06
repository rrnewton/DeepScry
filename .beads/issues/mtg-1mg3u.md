---
title: 'Web GUI: promote graveyard to a full pane (Ours | Theirs columns, scrollable); kill opponent-graveyard-floating-in-battlefield'
status: open
priority: 3
issue_type: task
depends_on:
  mtg-i9bux: related
created_at: 2026-06-06T00:44:07.475674221+00:00
updated_at: 2026-06-06T00:44:07.475674221+00:00
---

# Description

Web GUI (native_game.html) graveyard UX (user 2026-06-06): (1) the opponent graveyard still floats inside their battlefield — remove that. (2) Promote the small graveyard widget to a FULL pane (not a subset of the Hand pane): within it, a LEFT column 'Graveyard — Ours' and a RIGHT column 'Theirs', each keeping its count at the top. (3) Instead of the '... 5 more cards' truncation, the WEB GUI graveyard should be a SCROLLABLE widget showing all cards. Part of the impending LAYOUT OVERHAUL (shared GUI/TUI layout incl. card coordinates). Related: mtg-i9bux (battlefield layout redesign).
