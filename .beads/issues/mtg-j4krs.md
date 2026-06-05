---
title: 'NETARCH cleanup: spell_ability cross-check — fix stale doc + populate in WASM client so cross-check runs in production (audit D/Q4)'
status: open
priority: 2
issue_type: bug
depends_on:
  mtg-o99ow: related
created_at: 2026-06-05T14:08:02.035904315+00:00
updated_at: 2026-06-05T14:08:02.035904315+00:00
---

# Description

From audit Q4/§D. The user wants SubmitChoice.spell_ability used as a CROSS-CHECK against the index (assert agreement, fatal on mismatch), not as a replacement. GOOD NEWS: the server already does exactly this — index canonical, spell_ability validates, always-on, one equality compare (controller.rs:644-687). TWO DEFECTS: (1) STALE DOC: SubmitChoice.spell_ability comment (protocol.rs:343-347) and ChoiceResponse (controller.rs:116-120) say 'server uses this directly instead of looking up by index' — the OPPOSITE of the code. Fix docs to describe the existing index-canonical + cross-check-assert (always on). DOCS-ONLY, SAFE FIRST WAVE. (2) COVERAGE GAP: the WASM/web client always sends spell_ability: None (wasm/network/client.rs:1915-1922 'WASM client doesn't track spell_ability yet'), so the cross-check is a NO-OP on the deployed web path; only native-vs-native is protected. Native populates it for all priority choices (local_controller.rs:461-484). Thread the chosen SpellAbility through the wasm local controller and populate SubmitChoice.spell_ability so the assert protects production. Keep assert always-on (cheap). RISK: (2) touches wasm controller (slot03-adjacent) — SEQUENCE AFTER deep-AC; (1) safe now.
