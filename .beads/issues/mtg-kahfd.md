---
title: 'TRACK: Lobby+server-protocol redesign to deployed prototype (AFK build 2026-05-31)'
status: closed
priority: 1
issue_type: task
created_at: 2026-06-01T00:34:25.997406631+00:00
updated_at: 2026-06-01T12:34:23.571669256+00:00
---

# Description

TRACK (CLOSED-AS-FAILED): the 2026-05-31 AFK build attempt of the lobby+server-protocol+web redesign.

OUTCOME (2026-06-01, user review): the build merged + deployed @18b2941d but FAILED the top-line goal — it was gated on 'make validate green + skeptic diff-read', NEITHER of which exercises the human play journey, so untested-broken UI shipped: no page split (index.html reworked in place), renderer selector left on the lobby defaulting to TUI, double launcher, native render froze after first land, reload corrupts state, and the netarch finish (guards) only partially landed. Server-protocol CODE (Register/heartbeat/deck+ready/reconnect tokens/bug-report-infallible/--help) exists and may be salvageable, but NONE of it is play-test-verified.

SUPERSEDED BY mtg-35z3s (the REDO, done right): 4-page architecture + native-default + ONE launcher, gated on an END-TO-END PLAYED-GAME acceptance test built FIRST. Process fix: measure against the played-game test + diff intent-vs-result against the spec, not against green CI. Closing this tracker; all forward work under mtg-35z3s. Sub-issues that were wrongly marked done are reopened/qualified: mtg-dw9j3 (reopened), mtg-tnsk7 (qualified), mtg-khy7x (rewritten), mtg-1vwpd (qualified).
