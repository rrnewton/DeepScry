---
title: 'Bug: setARN set-origin matching + Mode$ Always state-trigger + CantBeCast/CantPlayLand statics unimplemented'
status: open
priority: 2
issue_type: task
created_at: 2026-05-31T01:49:00.357649793+00:00
updated_at: 2026-05-31T01:49:00.357649793+00:00
---

# Description

City in a Bottle (mtg-491, wave-16 robots sideboard, mtg-559) needs THREE unsupported constructs; the card resolves but does nothing:

1. set-origin matching `Permanent.!token+setARN+Other` / `Card.setARN` — no concept of a card's originally-printed set (ARN = Arabian Nights) exists in the engine; the predicate never matches.
2. `T:Mode$ Always` state-triggered ability (with IsPresent$ condition) — no Always/state-trigger firing site.
3. `S:Mode$ CantBeCast` and `S:Mode$ CantPlayLand` cast-prohibition statics — unimplemented.

Result: the 'sacrifice all ARN permanents' trigger never fires (verified: Library of Alexandria, an ARN card, survives), and the cast/play prohibition is not enforced.

Niche sideboard hoser; large feature surface (set-origin tracking touches the card DB/loader; state-triggers and cast-prohibition statics are broad engine features). Defer; not blocking robots-deck playability (City in a Bottle is rarely relevant).
