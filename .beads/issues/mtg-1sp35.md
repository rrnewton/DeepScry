---
title: 'NETARCH cleanup: remove dead eager OpponentChoice message + dedup-against-eager shim (audit A1/E1)'
status: open
priority: 2
issue_type: task
depends_on:
  mtg-o99ow: related
created_at: 2026-06-05T14:08:02.029564003+00:00
updated_at: 2026-06-05T14:08:02.029564003+00:00
---

# Description

From ai_docs/transient/NETARCH_CLEANUP_AUDIT_20260605.md Q2/§A1. ServerMessage::OpponentChoice (protocol.rs:816-862) has ZERO senders — the mid-game eager send was deleted (server.rs:3063-3079); opponent decisions now flow only via BufferedFact::Choice in ChoiceRequest.buffer. DELETE the dead receive path: native decode + WS-reader arm (client.rs:199-208, 2553-2580), wasm handler (wasm/network/client.rs:912) + its synthetic test feed (:2313), and the dedup-against-eager rationale/keep-first logic in push_opponent_choice (client.rs:1062-1099). RISK: touches client replay path that slot03-deepac2 (deep-AC desync fix) is editing — SEQUENCE AFTER that fix lands, on a clean base. Evidence it is safe: grep finds 0 ServerMessage::OpponentChoice construct/send sites. Validate with full make validate (network e2e).
