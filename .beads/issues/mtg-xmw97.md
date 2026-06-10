---
title: Intermittent robots42 seed-3 failure — AI Lightning Bolt mana-tap infinite loop (network-equiv flake recurrence)
status: open
priority: 2
issue_type: task
created_at: 2026-06-10T02:36:41.491425574+00:00
updated_at: 2026-06-10T02:37:15.262103874+00:00
---

# Description

## Summary

The `network.robots42` validate step (state-sync regression for the 1994 Old School "03 Robots Jesseisbak" deck, mtg-559) **failed intermittently** on integration CI, then **passed on a re-run of the same commit**. This is a flake — but NOT a CI-load/browser-startup timing flake. The symptom is a reproducible-looking AI mana-tap **infinite loop** within the failing run, which points at an intermittent engine/AI logic bug, not test-harness flakiness.

## Evidence (2026-06-10)

- **Commit `b3281a33`** (a WEB-ONLY change — `web/deck_editor.html` + test + beads, zero engine/native/wasm code) failed `network-equiv` on its first CI run (run 27248463211): `robots42` FAIL at **seed=3**, 0s, exit 1. Seeds 7/19/42 passed.
- The **identical engine code** at the parent `c348e3f8` had **PASSED** `robots42` in its own CI run (27247403740) — so a web-only delta provably did not cause this; the engine behavior is intermittent across runs.
- **`gh run rerun 27248463211 --failed`** re-ran only `network-equiv` → **PASS** (`✓ PASS robots42 (0s)`). Overall run then green; `b3281a33` deployed-eligible.

## Failure signature (seed=3)

A robot repeatedly tries to cast Lightning Bolt, taps a **Fellwar Stone for `{G}`**, then fails to pay the `{R}` cost:

```
[GAMELOG Turn2 M1] Zero2 casts Lightning Bolt ... → targeting Zero1
[GAMELOG Turn2 M1] Tap Fellwar Stone for {G}
Error casting spell: Invalid game action: Failed to pay mana cost:
  Insufficient total mana to pay R. Have: 0W 0U 0B 0R 1G 0C (regular) + combat
... (repeats) ...
Error: InvalidAction("Priority round exceeded max actions (1000), possible infinite loop")
```

So the AI taps a mana rock for the WRONG color (`{G}` instead of `{R}`), can't pay `{R}`, and retries the same illegal cast until the 1000-action priority-loop guard trips. Why this only manifests on **some runs at seed=3** (and not others / not at the same SHA's earlier run) is the open question — candidates: nondeterminism in the AI's mana-source color selection, an ordering/iteration nondeterminism in available-mana enumeration, or a Fellwar Stone color-availability edge (Fellwar Stone produces colors among its controllers' lands — if that set is computed from a nondeterministically-ordered source, the AI may pick {G} when {R} was needed).

## Why this matters (not "just a flake")

- The 1000-action guard turns a logic bug into a hard test failure rather than a hang — good. But an AI that loops on an unpayable cast is a real determinism/AI-correctness defect that could surface in live AI games, not only in CI.
- Recurrence of the network-equiv flake family previously tracked (and closed) under mtg-256 ("Flaky network equivalence test ~50% failure"). That issue was marked RESOLVED 2026-03-09; this is a distinct, narrower recurrence localized to robots42 seed-3 + the Fellwar Stone mana-tap path.

## Next steps

1. Reproduce seed=3 locally with a fixed seed and capture whether the AI's mana-source color choice is nondeterministic across runs (run the robots42 e2e at seed 3 N times).
2. If nondeterministic: find the unordered collection driving Fellwar Stone / mana-source selection and impose a deterministic order (no-collect/no-clone DRY rules apply).
3. If deterministic-but-wrong: fix the AI mana-payment color selection so it taps `{R}` for a `{R}` cost.

## References

- mtg-559 — robots42 / "03 Robots Jesseisbak" deck compat tracker (this is that deck's e2e).
- mtg-256 (closed) — prior network-equivalence flake family this recurs from.
- CI runs: 27248463211 (b3281a33, first FAIL then rerun PASS), 27247403740 (c348e3f8, PASS on identical engine code).
