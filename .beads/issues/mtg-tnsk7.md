---
title: 'Strict layering: network + player-controller logic must be SHARED across TUI and GUI (no per-UI duplication)'
status: open
priority: 2
issue_type: task
created_at: 2026-05-31T20:13:58.072105756+00:00
updated_at: 2026-05-31T21:37:21.289953390+00:00
---

# Description

USER architectural requirement + CONFIRMED ROOT-CAUSE EVIDENCE (2026-05-31). Rules: engine knows no UI; shared UI logic knows no renderer; networking + player-controller logic is the SAME code for TUI and GUI — only the renderer differs (docs/NETWORK_ARCHITECTURE.md).

CONFIRMED BUG (explains user's 'native GUI checkbox does nothing; native_game.*.html URL but still the TUI'): web/native_game.html's NETWORK path (lines ~1763-1850) deliberately calls launch_network_game — which is the TUI/ratzilla renderer — and even creates a #ratzilla-terminal <div> on the fly. The native CARD renderer (launch_game_session) is LOCAL-ONLY; there is NO native card renderer for network games at all. So Phase 2's 'native GUI network client' (mtg-187) is a FACADE: for network play both TUI and Native land on the TUI renderer. This is the layering violation to fix: extract ONE shared network-client + player-controller path; the renderer (TUI ratzilla vs native card DOM) must be the only thing that swaps. Currently network play is welded to the ratzilla renderer.

Likely correct shape: a renderer-agnostic network game driver (consumes choice requests / state from the WS, drives a controller) with a pluggable view (ratzilla TUI OR native card DOM). Implement against the mtg-khy7x storyboard + mtg-1vwpd param contract.
