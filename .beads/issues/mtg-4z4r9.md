---
title: 'Networked HUMAN/TUI game desync: client shadow missing a PlayLand (own-drawn land) → FATAL DESYNC + stuck ''Waiting for Server'''
status: open
priority: 2
issue_type: task
labels:
- network
- desync
created_at: 2026-06-02T14:24:51.850768051+00:00
updated_at: 2026-06-02T14:24:51.850768051+00:00
---

# Description

USER LIVE REPRO 2026-06-02 (deepscry.net cb9b15d9), MIXED GUI(P1)/TUI(P2) networked game:
- P1 (native GUI) took turn 1 fine. Turn 2 started, P2 (TUI, human) drew a card, but the TUI action menu stuck on "Waiting for Server" — P2 could not take its turn.
- Console: `FATAL DESYNC: Local abilities (2) != server abilities (3). Local: [PlayLand { card_id: 73 }, PlayLand { card_id: 78 }], Server: [PlayLand { card_id: 72 }, PlayLand { card_id: 73 }, PlayLand { card_id: 78 }]`
- I.e. the TUI client shadow is MISSING PlayLand{72} that the server offers — the shadow's hand/abilities diverge from the authoritative server at P2's turn-2 priority. Likely card 72 is P2's own freshly-drawn land that the shadow didn't materialize/register as playable (a draw/hand-state or re-entry/rewind issue on the shadow).

KEY: this is the HUMAN/TUI path (fancy_tui::run_network_mode_human_v2), NOT the AI path netarch (mtg-610) unified + gated. The netarch gate (rogerbrand/robots42) is AI-vs-AI mirrors, so netarch going green there does NOT exercise/guarantee this human-path case. The reveal-history-buffer / strict-rewind work must be verified on the HUMAN path too (a human P2 drawing + playing on turn 2), and this specific own-drawn-land-missing desync reproduced + fixed.

NEXT: reproduce in a test (mixed GUI/TUI or at least a human-controller networked game advancing to turn 2 with a draw + land play); root-cause why the shadow lacks PlayLand{72}; confirm the netarch reveal-buffer/rewind covers the human path. Relates mtg-610 (netarch rewind/replay), mtg-679 (human-controller desync). compute_view_hash is the arbiter; this is a real divergence (the abilities differ), not a benign masking artifact.
