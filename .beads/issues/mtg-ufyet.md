---
title: 'Web deck editor: add card art to the card-details pane'
status: open
priority: 3
issue_type: task
created_at: 2026-06-10T01:46:49.947284845+00:00
updated_at: 2026-06-10T01:46:49.947284845+00:00
---

# Description

The web deck builder (web/deck_editor.html) has a text-only card-details pane (name, mana cost, type line, P/T, oracle text) added alongside mtg-682. It still lacks card ART, with a "Text-only for now (images later)" placeholder in renderCardDetails.

## Target behavior

When a card is selected (click on a catalog row or a deck-list row), the card-details pane shows the card's art image in addition to the existing text fields.

## Art source (WASM-free, DRY)

The deck editor is deliberately WASM-free (two lightweight JSON fetches; no card DB). The Scryfall-CDN path in tui_game.html requires the WASM tui_card_cdn_url() + the binary card-lookup.bin table, so it is NOT usable here. Use the same WASM-free fallback the launcher pages (launcher.html / solo_launcher.html) expose: Gatherer, whose URL is computed purely from the card name:
  https://gatherer.wizards.com/Handlers/Image.ashx?name=<name>&type=card

The image is loaded best-effort: a broken/unavailable image (older-card-only Gatherer coverage) is hidden gracefully (onerror) so the text details still render. No new tracked image assets are added.

## Tests
Extend web/test_deck_editor.js section 4b to assert the pane renders an <img> with a Gatherer src for the clicked card.
