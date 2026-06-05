---
title: 'Launcher: ''Back'' from game loses settings (reverts to defaults); joiner reconnect flaky; zombie ready-state'
status: open
priority: 3
issue_type: bug
created_at: 2026-06-05T13:52:32.705814499+00:00
updated_at: 2026-06-05T13:52:32.705814499+00:00
---

# Description

Launcher: 'Back' from game loses settings; joiner reconnect flaky; zombie ready-state.

REPORTED (user playtest 2026-06-05): play a 'julian vs gabriel' avatar booster game, then press 'Back' from within the game to return to the launcher. Three problems:

(a) MOST IMPORTANT - settings not saved: the launcher reverts to defaults (deck back to 'eric_avatar_draft', collection back to 'Booster Draft') even if the last game used something else (e.g. Old School). It should SAVE and restore the last-used selections.

(b) Joiner reconnect flaky: the game-CREATOR reconnects fine, but the game-JOINER usually needs a page refresh to reconnect (inconsistent).

(c) Zombie ready-state: sometimes both sides show 'ready' green boxes but toggling the big green button does nothing and the UI is unresponsive, with NO dev-console warnings/errors. Reloading usually fixes it, but often requires multiple reloads from the joiner.

Related: mtg-694 (lobby/launcher playtest UX fixes), mtg-682 (launcher overhaul tracker), mtg-695 (closed - lobby redo dropped launcher settings; this is a regression of the same class), mtg-692 (in_progress - remove dead launcher-only JS from game pages).
