---
title: 'Standardized + flexible test binary strategy: trusted env signal for prebuilt mtg binary + feature-flag encoding'
status: open
priority: 3
issue_type: task
created_at: 2026-05-31T20:13:58.073266180+00:00
updated_at: 2026-05-31T20:18:40.478142222+00:00
---

# Description

Standardized+flexible test-binary strategy (USER). Run a test INDIVIDUALLY → it cargo build/runs (hermetic, exercises current code). Run under make validate → a TRUSTED env var tells it a fresh mtg binary is already built; it uses that prebuilt binary instead of re-invoking cargo (removes the cargo build-lock serialization that stalls validate — the mtg-sto4q symptom). The handshake also encodes the FEATURE FLAGS the binary was built with, so consumers verify the config and fail loud on mismatch. One shared helper for all e2e shell tests (tests/*.sh) + agentplay + smoke; wire via scripts/validate.sh.
SUPERSEDES mtg-sto4q (closing it). Distinct from mtg-wvn3d (precheck false-positive).
