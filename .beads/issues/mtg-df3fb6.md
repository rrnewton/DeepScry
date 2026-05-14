---
title: 'Fuzz infrastructure race: deck_submission.json.tmp clobbered when --client wasm/mixed runs in parallel'
status: open
priority: 4
issue_type: task
created_at: 2026-05-14T15:20:26.780362769+00:00
updated_at: 2026-05-14T15:20:26.780362769+00:00
---

# Description

## Summary

`bug_finding/network_test_lib.py:325` writes `web/data/deck_submission.json.tmp` then `os.replace` to `deck_submission.json`. Multiple WASM client threads (one per parallel test, two per test) all write the SAME paths, so concurrent runs race:

```python
deck_submission_path = os.path.join(WEB_DIR, "data", "deck_submission.json")
deck_submission_path_tmp = deck_submission_path + ".tmp"

with open(deck_submission_path_tmp, 'w') as f:
    json.dump(deck_data, f)
os.replace(deck_submission_path_tmp, deck_submission_path)
```

Observed exception in `bug_finding/network_fuzz_test.py --quick --client wasm --parallel 2`:

```
FileNotFoundError: [Errno 2] No such file or directory:
  '.../web/data/deck_submission.json.tmp' -> '.../web/data/deck_submission.json'
```

Two threads both wrote .tmp; the first replaced it; the second's replace fails because the .tmp it wrote was just consumed.

Worse: even if both calls succeed, they may swap the wrong deck for the wrong player (P1 reads P2's deck). This may explain some of the 'timeout' failures observed in WASM mode where the player has the wrong cards and the AI can never make progress.

## Suggested fix

1. Per-thread temp filename: `deck_submission_<uuid>.json` and pass the file name through the URL to the harness, OR
2. Encode the deck inline in the URL fragment, OR
3. Serialize WASM-mode runs (force --parallel 1 when --client wasm).

## Reproducer

```bash
cd mtg-forge-rs
python3 bug_finding/network_fuzz_test.py --quick --client wasm --parallel 2
```

(Race fires roughly every other run.)

## Discovered by

QA session on `qa-fuzz-testing` @ fe820468, 2026-05-14.
