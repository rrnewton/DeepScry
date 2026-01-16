# Bug Finding Scripts

This directory contains scripts for **randomized stress testing** to discover new bugs.
These are **not** regression tests and are not meant to be run as part of `make validate`.

## Purpose

Bug finding scripts are designed to:
- Run for extended periods (minutes to hours)
- Test with randomized inputs (seeds, decks, controllers, configurations)
- Identify crashes, panics, and unexpected behavior
- Provide reproducible commands for debugging

## Requirements for Bug Finding Scripts

All scripts in this directory should:

1. **Run for a configurable duration** - Either a fixed batch size or run forever until interrupted
2. **Handle Ctrl-C gracefully** - Print a summary when interrupted (SIGINT)
3. **Print a summary on exit** containing:
   - Total inputs tested
   - Pass/fail counts
   - Categorized failure types
   - Reproducer commands for each unique failure
4. **Save failure logs** to temporary directories for debugging

## Available Scripts

### `network_fuzz_test.py`

Randomized testing of network game synchronization with different:
- Random seeds
- Controller types (heuristic, random, zero)
- Player configurations

```bash
# Quick test (10 configs, parallel execution)
python3 bug_finding/network_fuzz_test.py --quick

# Extended test (100 configs)
python3 bug_finding/network_fuzz_test.py --configs 100

# Run until interrupted
python3 bug_finding/network_fuzz_test.py --infinite
```

## Running Bug Finding Sessions

For burn-in testing before releases or when looking for new bugs:

```bash
# Run network fuzz testing for ~1 hour
python3 bug_finding/network_fuzz_test.py --configs 500 --parallel 4

# Or run indefinitely until Ctrl-C
python3 bug_finding/network_fuzz_test.py --infinite --parallel 4
```

## Relationship to Regression Tests

- **`tests/`**: Deterministic regression tests run by `make validate` and CI
- **`bug_finding/`**: Randomized exploratory testing for discovering new bugs

When a bug finding script discovers a reproducible failure:
1. Add a minimal reproducer to `tests/` as a regression test
2. Fix the bug
3. The regression test prevents the bug from returning
