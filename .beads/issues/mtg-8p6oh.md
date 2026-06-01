---
title: 'Championship deck FETCHING protocol prototype: decks/championship/<year>/ + README/provenance, fetch every-5-yr champ decks (NO wiring/code changes)'
status: open
priority: 3
issue_type: task
created_at: 2026-06-01T13:49:18.450338457+00:00
updated_at: 2026-06-01T13:49:18.450338457+00:00
---

# Description

USER (2026-06-01): a background agent figures out JUST the championship-deck fetching protocol. Create decks/championship/<year>/ dirs (e.g. decks/championship/1995/) each with a README documenting provenance + source links. Fetch the every-5-years championship decks: 1995, 2000, 2005, 2010, 2015, 2020, 2025. DO NOT wire them up / change any code / touch collection-loading — data + docs only this pass.

SCOPE: pure data acquisition. For each target year, find the World Championship (or best-available championship) finalist decklists for that year and save as .dck files under decks/championship/<year>/ with a per-year README: event name, date, location, source URL(s), and per-deck pilot + placement. PARSE the .dck into our format (mirror decks/old_school/*.dck — [metadata]/[Main]/[Sideboard]; see docs/DCK_FORMAT.md + DeckLoader). Card names must match cardsfolder naming (the agent can sanity-check a deck LOADS with  deck-parse if trivial, but NO code changes / NO collection wiring / NO make-validate needed).

SOURCES + PROTOCOL (intel from this session):
- Official MTG World Championship decklists exist for many years; WotC even sold 'World Championship Decks' products (1997-2004). Wizards/Wikipedia/mtgtop8/mtgdecks/tcdecks carry them. Pick the most authoritative per year + CITE it.
- IMPORTANT mtgdecks.net fetch protocol (we proved this): bare HTTP = 403; Playwright (real chromium, web/node_modules) gets through but RATE-LIMITS to ~the first request per fresh browser.newContext() then 403s the session — so do ONE goto per fresh context with ~8s gaps. A deck-DETAIL page renders fully incl event+date in body text. Reuse/adapt debug/scrape_mtgdecks_oldschool.js. Be POLITE (long gaps, fresh context per request) — do NOT get the IP banned (it'd hurt other work). WebSearch first to FIND the right per-deck/per-year URLs, then Playwright-fetch the specific pages.
- Note the naming caveat: MTG World Championship started 1994 (Zak Dolan). '1995' = the 1995 Worlds, etc. Be honest about event-year vs card-pool-year in the README (don't conflate with the modern Old School format).

DELIVERABLE: decks/championship/<year>/ for as many of 1995/2000/2005/2010/2015/2020/2025 as found, each w/ .dck files + README (provenance+links+pilots+placements), committed to a worktree branch. Report which years/decks were found, source URLs, and any that couldn't be located. NO code, NO wiring, NO collection changes — that's mtg-nmbr1's later phase. This issue = the fetching protocol + the raw decks only.
Part of / precursor to mtg-nmbr1 (official collections).
