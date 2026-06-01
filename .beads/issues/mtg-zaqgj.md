---
title: 'Lobby Phase 1 skeptic follow-ups (4 non-blocking): name-leak, WAIT_FOR_JOINER timeout, SetReady-not-gating comment, double WaitingRoomUpdate'
status: open
priority: 3
issue_type: bug
created_at: 2026-06-01T01:37:27.652781529+00:00
updated_at: 2026-06-01T01:37:27.652781529+00:00
---

# Description

Adversarial skeptic review of lobby-server-protocol @98d3b43c (merged) flagged 4 real-but-non-blocking CONCERNs. None affect determinism/hidden-info/security; merged MERGE-OK. Fix as cleanup before/with Phase 3:
1. DOUBLE-REGISTER NAME LEAK (server.rs ~697): if a connection sends Register twice with different names, the FIRST name is never released (only registered_name=last is cleaned on disconnect) → first name reserved until server restart. Fix: release prior registered_name before overwriting, or reject second Register.
2. WAIT_FOR_JOINER TIMEOUT RESTARTS (server.rs:1151): tokio::time::timeout(WAIT_FOR_JOINER, &mut handoff_rx) is created fresh INSIDE the select! loop, so every update_rx.changed()/WS-read iteration restarts the 30-min deadline → a creator sending frequent msgs (Ping/SetDeck) holds a waiting-games slot indefinitely (DoS). Fix: pin a tokio::time::sleep(deadline) OUTSIDE the loop, select on it.
3. SETREADY DOES NOT GATE GAME START + WRONG COMMENT (server.rs:1500 vs 1265-1268; protocol.rs SetReady doc): run_join_flow handoff_tx.send fires on JOIN, not on both-ready; both_ready@1264 is diagnostic-only. protocol.rs SetReady docstring promises both-ready gating that isn't implemented. The comment @1265-1268 ('joiner's SetReady already triggered the handoff') is FACTUALLY WRONG — a Phase-3 trap. Fix: correct the comment + docstring now; implement real both-ready gating in Phase 2/3 (the new lobby.html waiting room depends on this for the 'both Ready → start' UX). HIGH-RISK trap for Phase 3 — fix comment before then.
4. DOUBLE WaitingRoomUpdate TO CREATOR (server.rs:1222+1231, 1251+1263): creator's SetDeck/SetReady both tx.send(snap) on watch AND directly send_message(snap); next select! update_rx.changed() resends → creator gets each self-update twice (joiner unaffected). Cosmetic; fix by not direct-sending when the watch channel will deliver it.
Source: skeptic agent a3cfec4d, verdict MERGE-OK.
