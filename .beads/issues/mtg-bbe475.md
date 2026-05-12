---
title: 'Research: Web GUI architecture and how to drive it with Playwright'
status: closed
priority: 0
issue_type: task
created_at: 2026-04-04T01:50:09.420741185+00:00
updated_at: 2026-05-12T13:58:40.952454984+00:00
closed_at: 2026-05-12T13:58:40.952454894+00:00
---

# Description

Files: src/ (web GUI code), wasm/ (WASM build), index.html or similar

Action:
1. Find the web GUI source code and how it renders game state
2. Understand how the WASM module communicates with the JS frontend
3. Identify key DOM elements/selectors for game actions (play card, activate ability, pass, etc.)
4. Check how game state is displayed (hand, battlefield, graveyard, stack)
5. Assess Playwright screenshot capabilities and how to pass images to Claude

Why: Need to know the web GUI structure to write Playwright automation.

Verify: Report with DOM structure, key selectors, and a proposed Playwright interaction flow.
