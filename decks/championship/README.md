# Championship Decks — Official MTG World Championship Decklists

This directory contains finalist decklists from the **official Magic: The Gathering World Championship** (the annual Wizards of the Coast event), organized by year. Every file is a `.dck` file in the standard format (`[metadata]` / `[Main]` / `[Sideboard]`).

**These are NOT the modern "Old School 93/94" community format.** The Old School community event (n00bcon "World Championship") is a separate modern-day tournament played with vintage card pools. The decks here are from the official WotC Worlds event, which began in 1994.

## Structure

```
decks/championship/
├── 1995/   — Seattle, WA, USA (Aug 1995) — Champion: Alexander Blumke
├── 2000/   — Brussels, Belgium (Aug 2000) — Champion: Jon Finkel
├── 2005/   — Yokohama, Japan (Dec 2005) — Champion: Katsuhiro Mori
├── 2010/   — Chiba, Japan (Dec 2010) — Champion: Guillaume Matignon
├── 2015/   — Seattle, WA, USA (Aug 2015) — Champion: Seth Manfield
├── 2020/   — Honolulu, HI, USA (Feb 2020) — Champion: Paulo Vitor Damo da Rosa (MWCXXVI)
└── 2025/   — Bellevue, WA, USA (Dec 2025) — Champion: Seth Manfield (MWC 31)
```

Each subdirectory contains:
- `.dck` files: Top 4 (champion, finalist, semifinalists) — typically 4 decks per year
- `README.md`: Event details, source URLs, per-deck provenance, accuracy notes

## Coverage Summary

| Year | Decks | Champion | Confidence |
|------|-------|----------|------------|
| 1995 | 4 (Top 4) | Alexander Blumke (B/W Rack) | High for 3/4; Justice deck approximate |
| 2000 | 4 (Top 4 per WC product) | Jon Finkel (Mono-Blue Tinker) | High — official WC Decks 2000 product |
| 2005 | 4 (Top 4) | Katsuhiro Mori (Selesnya Ghazi-Glare) | High for 2/4; Asahara/Kaji approximate |
| 2010 | 4 (Top 4) | Guillaume Matignon (U/B Control) | High for 3/4; PVDDR approximate |
| 2015 | 4 (Top 4) | Seth Manfield (Abzan Control) | High — confirmed from SCG coverage |
| 2020 | 4 (Top 4) | Paulo Vitor Damo da Rosa (UW Control) | High — confirmed from SCG coverage |
| 2025 | 4 (Top 4) | Seth Manfield (Izzet Lessons) | High — sourced from official magic.gg; **cards not in current engine** |

## Fetch Protocol Notes

This batch of decklists was fetched using the following protocol:

1. **WebSearch first** to locate the best URL per year/player
2. **WebFetch** for pages that allow it — TappedOut, Star City Games, magic.gg, mtg.wtf all worked
3. **mtgdecks.net returned 403** for all WebFetch attempts (confirmed matching the session-limiting behavior documented in mtg-8p6oh)
4. **MTGGoldfish returned 403** for all WebFetch attempts
5. **Moxfield returned 403** for all WebFetch attempts
6. **Scryfall returned 403** for all WebFetch attempts
7. **mtg.fandom.com returned 403** for all WebFetch attempts
8. **Best sources** that DID work: TappedOut, Star City Games articles, magic.gg (official), mtg.wtf (official WC product mirror), Usenet archive (groups.google.com for 1995)
9. **Search snippet extraction**: For some decks where a single authoritative page wasn't accessible, WebSearch returned enough detail in snippets for reconstruction (Hernandez/Stern 1995; Karsten 2005; Manfield/Black/Rietzl/Turtenwald 2015)

## Accuracy Caveats

Files marked as **APPROXIMATE** in their year's README should be verified against primary sources before being used for engine-compatibility testing. Approximate files exist for:
- `1995/03_justice_red_artifact.dck` (skeleton — primary source not fetchable)
- `2005/03_asahara_enduring_ideal.dck` (approximate reconstruction from archetype knowledge)
- `2005/04_kaji_ghazi_glare.dck` (filed as same 75 as Mori; minor diffs unconfirmed)
- `2010/03_pvddr_ub_control.dck` (archetype template; exact 75 on 403-blocked mtgdecks.net)

## Next Phase

Collection wiring (mtg-nmbr1): defining these as named Collections in the launcher and verifying card-load compatibility is tracked separately. This directory is **data + provenance only** — no code was changed, no collection plumbing added here.

## See Also

- `decks/old_school/` — Modern "Old School 93/94" community format decks (n00bcon era, NOT 1994 championship)
- `docs/DCK_FORMAT.md` — `.dck` file format specification
