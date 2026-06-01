# Old School 93/94 Decks

Representative decklists for the **Old School 93/94** Magic: The Gathering format —
a *modern* community format played with cards from the 1993–94 sets (Alpha/Beta/
Unlimited, Arabian Nights, Antiquities, Legends, The Dark, plus the usual b/r list).
**These are NOT 1994 decks or a 1994 championship** — the format itself did not exist
in 1994; it is played today at events like [n00bcon](http://www.n00bcon.com/) (the
"93/94 World Championship", Gothenburg) and regional leagues.

## Provenance

Manually downloaded from **[mtgdecks.net → Old-school](https://mtgdecks.net/Old-school)**,
which aggregates tournament decklists submitted by players. The player names in the
filenames are the deck pilots. From the mtgdecks Old-school listings, several of these
pilots (e.g. **Jose Antonio Prieto** — `06_jeskai_aggro`) are competitors in the
**Catalan/Spanish Old School scene**: the [Liga Catalana d'Old School (LCOS)](https://oldschool.cat/eng)
and [Bazaar of Barna](https://bazaarofbarna.com/) tournaments in Barcelona, with
results dated **2024** (e.g. Prieto Top-8'd LCOS Sept 2024 with a "Disco Troll" list).
So these are **mid-2020s community-tournament Old School lists**, archetype-
representative, not a single official championship.

Per-deck source pages live under their archetype on mtgdecks:
[robots](https://mtgdecks.net/Old-school/robots),
[troll-disk](https://mtgdecks.net/Old-school/troll-disk),
[mono-black](https://mtgdecks.net/Old-school/mono-black),
[rogue](https://mtgdecks.net/Old-school/rogue). The exact per-deck URL +
event/date for each of our six was not all individually pinned (mtgdecks
rate-limits scraping — see mtg-scrape issue), but one is confirmed as a worked
example: the Prieto Disco-Troll list is
[Top-8, Liga Catalana Old School, 29-Sep-2024](https://mtgdecks.net/Old-school/disco-troll-decklist-by-jose-antonio-prieto-2255002).
The safest collection label is **"Old-School Decks"**, not
"Championship <year>". (A separate effort, mtg-nmbr1, tracks building real,
properly-sourced official Championship-by-year collections for public launch.)

## Deck list

| File | Archetype | Pilot |
|---|---|---|
| `01_rogue_rogerbrand.dck` | Rogue / multicolor | Roger Brand |
| `02_thedeck_peterschnidrig.dck` | "The Deck" (UWx control) | Peter Schnidrig |
| `03_robots_jesseisbak.dck` | Artifact aggro ("Robots") | Jesse Isbak |
| `05_mono_black_rogerbrand.dck` | Mono-Black control | Roger Brand |
| `06_jeskai_aggro_joseantonioprieto.dck` | Jeskai aggro | Jose Antonio Prieto (LCOS / Bazaar of Barna, ~2024) |
| `06_troll_disk_daniellebrunazzo.dck` | Troll Disk combo | Danielle Brunazzo |

## Engine compatibility

Per-card and per-deck engine compatibility for these decks is **tracked in beads**
(minibeads), not here — see the deck-tracker issues (rogue mtg-387, thedeck mtg-413,
robots mtg-559, mono-black mtg-560, jeskai mtg-561, troll-disk mtg-562) and the
overall goal **mtg-pph0s** (full play-tested support for all 1994 old-school decks).
This README intentionally carries **no** point-in-time "what works / what's blocked"
status — that drifts; consult beads for current state.

## See also

- `docs/DCK_FORMAT.md` — `.dck` file format spec
- `decks/old_school2/` — synthetic/AI-generated archetype lists used as engine test
  fixtures (NOT real tournament decks)
