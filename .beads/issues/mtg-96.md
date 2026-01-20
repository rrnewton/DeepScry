---
title: 'TODO: search for cardsfolder'
status: closed
priority: 2
issue_type: task
labels:
- human
created_at: 2025-10-27T13:51:48+00:00
updated_at: 2026-01-20T14:17:40.605197480+00:00
---

# Description

## Task

Improve cardsfolder search path to be more robust:
- `./cardsfolder` if it exists
- Go to the directory containing the `mtg` binary, look for `cardsfolder` there
- If not found, go up to the parent directory, repeating the search for `./cardsfolder`
- If we reach the root `/` and don't find it, then error

## Implementation

Enhanced `src/loader/cardsfolder.rs` with new search algorithm:

1. **CARDSFOLDER environment variable** (new) - Allows explicit override
2. **./cardsfolder in CWD** - Quick check for current directory
3. **Binary directory search** (new) - Finds exe location and searches up
4. **Parent directory walk** (new) - Walks up directory tree to root

Also updated `main.rs` to use the centralized `find_cardsfolder()` from the loader module instead of its own simple implementation.

## Benefits

- Works when running binary from any directory
- Environment variable for CI/deployment flexibility
- Comprehensive search covers development and installed scenarios
- Better error messages with search path details
