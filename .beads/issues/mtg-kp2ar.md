---
title: reconstruct_tapped_states misses 3 unlogged tap sites -> route taps through one TapCard chokepoint
status: open
priority: 3
issue_type: bug
created_at: 2026-06-05T17:36:16.653609485+00:00
updated_at: 2026-06-05T17:36:16.653609485+00:00
---

# Description

slot04 desync-review 2026-06-05. reconstruct_tapped_states rebuilds tapped from logged TapCard entries, but 3 tap sites do NOT log TapCard: global ETB-tapped (state.rs:1489), returns-tapped (state.rs:3565), Cost::Untap (actions/mod.rs:9262). A permanent tapped via these and re-materialized on rewind loses its tapped state (not exercised by robots deck). Fix: route ALL tap state-changes through a single TapCard-logging chokepoint so reconstruction is complete. Related: mtg-o99ow.
