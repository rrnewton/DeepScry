---
title: 'Bug-report dialog: precheck WS connection before typing + show connection error clearly'
status: open
priority: 3
issue_type: bug
created_at: 2026-05-28T18:50:44.166581084+00:00
updated_at: 2026-05-28T18:50:44.166581084+00:00
---

# Description

Bug-report button UX: pressing Submit failed with "Bug report submission requires an active network WebSocket connection." Two fixes:
1. CHECK the WebSocket connection state BEFORE the user types their bug report (on dialog open), so they don't waste effort writing a report that can't be submitted.
2. Show that connection error CLEARLY on the dialog itself, persistently, irrespective of whether Submit is pressed (e.g. a disabled Submit + an inline "not connected — bug reports need an active server connection" banner).
Touches the bug-report dialog in web/tui_game.html (+ native_game.html once the button is added there) + web/network.js connection-state. Related: mtg-tan84 (bug-report pipeline). Note: the file-from-VM Stage 0 work (mtg-tan84) changes how reports are filed — coordinate so the precheck reflects the actual submit path.
