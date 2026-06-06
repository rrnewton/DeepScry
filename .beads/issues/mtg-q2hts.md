---
title: 'Web GUI: gate ungated console.log / WASM log::info prints behind DEBUG (log spam without trace mode)'
status: open
priority: 3
issue_type: bug
created_at: 2026-06-06T00:14:05.545952963+00:00
updated_at: 2026-06-06T00:14:05.545952963+00:00
---

# Description

The web game GUI prints noisy status/init logs to the browser console even with DEBUG/trace OFF (user-reported on the deployed site, 2026-06-06). Sources: ~14 ungated console.log() calls in web/native_game.html (e.g. '[Network] Network features available', '[mtg-722] card-lookup table: N entries', 'MTG Forge Native GUI initialized: N decks', '[Network] Connecting to ...', '[Network] State: ...') that should go through the existing debugLog() gate (isDebugMode()); AND ~49 ungated log::info!/println! in mtg-engine/src/wasm/ that surface to the browser console. FIX: route the JS status prints through debugLog() (keep genuine warnings/errors at console.warn/error), and lower the WASM per-action/status log::info! to log::debug!/trace! (or gate behind the debug flag) so the default web log is quiet. Keep errors + desync-fatal messages loud. Part of the 'minimal, internally-consistent, well-documented' web cleanup. Related: mtg-rxacr (debug-gating), the networking-code cruft audit.
