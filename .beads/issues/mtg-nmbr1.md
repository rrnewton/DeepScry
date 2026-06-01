---
title: 'Official deck collections for public launch: Championship <year> decks (every-5-years first) + rename test collections behind ?testing=true'
status: open
priority: 2
issue_type: task
depends_on:
  mtg-pph0s: blocks
  mtg-8p6oh: blocks
  mtg-35z3s: blocks
  mtg-cz3vm: blocks
created_at: 2026-06-01T13:01:04.684026617+00:00
updated_at: 2026-06-01T13:49:18.460678659+00:00
---

# Description

USER (2026-06-01): make Deck Collections clean + official before public launch.

PLAN:
1. CHAMPIONSHIP COLLECTIONS BY YEAR: for each year, include the finalist ~4-8 championship decks, named 'Championship <YEAR>' (e.g. 'Championship 1994'). NOTE the provenance caveat we just confirmed: 'Old School 93/94' is a MODERN format (n00bcon World Championship runs in the 2010s-2020s, not 1994) — so 'Championship 1994' should mean the championship decks representative of / legal in that year's card pool, sourced accurately (cite the real event/year per deck; don't mislabel a 2017 n00bcon list as '1994'). Decide naming carefully: 'Championship <YEAR>' should be honest about what the year denotes (pool year vs event year). Real sources: official Wizards World Championship decklists exist for many years (WotC even sold 'World Championship Decks' products 1997-2004); n00bcon for old-school. Get the right finalist lists per year + confirm they LOAD correctly (cards in cardsfolder) before committing.
2. FILL ORDER: first every 5 years — 1995, 2000, 2005, 2010, 2015, 2020, 2025 — each found + committed + loads correctly. THEN recent years 2021-2024. THEN all remaining years.
3. RENAME TEST COLLECTIONS: current 'Booster Draft' + 'Commander' collections → 'Testing - Booster Draft' + 'Testing - Commander'. HIDE them in the launcher UNLESS query param '?testing=true' is present; make it STICKY like allow_local_img_load (mtg-477 pattern — persist in localStorage/sessionStorage so it survives navigation). Our random/private testing decks (booster draft, commander, old_school2 'cooked up from AI' decks) live under the Testing category.

DEPENDS ON (do AFTER current goals): the 4-page lobby/launcher REDO (mtg-35z3s) — the deck collections + ?testing= gate live in the new launcher.html; AND the old-school deck compat goal (mtg-pph0s) — finish making the current decks WORKING before expanding the catalog. So this is post-both-goals.
Touches: launcher.html (collections UI + ?testing= sticky gate), the deck catalog/collection data (decks/ dirs + how collections are defined/exported), deck README provenance. Also fold in the README provenance correction for decks/old_school (it's modern n00bcon-era archetypes from mtgdecks, NOT 1994 championship decks).
