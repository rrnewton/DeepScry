---
title: Scrape exact mtgdecks.net source URLs + event/date for each downloaded deck collection (Playwright)
status: open
priority: 3
issue_type: task
created_at: 2026-06-01T13:28:55.356099477+00:00
updated_at: 2026-06-01T13:28:55.356099477+00:00
---

# Description

USER (2026-06-01): record the exact mtgdecks.net source URL (+ event/date) for every deck in collections we fetch, in the collection README.

FINDINGS so far (this session): mtgdecks BLOCKS bare HTTP (403) but Playwright (real chromium) gets through — HOWEVER it rate-limits to ~the FIRST request per fresh browser CONTEXT, then 403s the rest of that session. Working pattern: ONE goto per fresh browser.newContext() with an ~8s gap between contexts (debug/scrape_mtgdecks_oldschool.js is a starting point). A deck-DETAIL page (e.g. .../disco-troll-decklist-by-jose-antonio-prieto-2255002) renders FULLY and exposes the event+date in body text (proven: 'Top8 — Liga Catalana Old School — September 2024 [15 Players] — 29-Sep-2024'). The /search?q= endpoint 404s; the decklists-by-<player> URL falls back to a generic archetype page (no per-player filter). Archetype listing pages work: /Old-school/{robots,troll-disk,mono-black,rogue,...} (paginated; page:2+ gets 403 in-session → use fresh context per page).

TODO: for our 6 decks/old_school decks, find each one's exact deck-detail URL — either (a) match our .dck card lists against the archetype listing pages (fresh-context-per-page crawl, polite delays), or (b) targeted web-search per pilot+archetype. Then write URL + event/date into decks/old_school/README.md. Generalize: any future fetched collection records source URLs. CONFIRMED so far: Jose Antonio Prieto Disco-Troll = https://mtgdecks.net/Old-school/disco-troll-decklist-by-jose-antonio-prieto-2255002 (LCOS Sept 2024 Top8). Be polite (don't get the IP banned — long gaps, fresh context per request). Belongs with mtg-nmbr1 (official collections).
