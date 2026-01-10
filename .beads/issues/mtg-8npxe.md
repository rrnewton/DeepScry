---
title: WASM e2e tests fail with bincode deserialization errors
status: open
priority: 3
issue_type: task
created_at: 2026-01-10T20:07:31.518207180+00:00
updated_at: 2026-01-10T20:07:31.518207180+00:00
---

# Description

## Summary

After merging origin/main (commit cbc11bf), the WASM e2e tests fail with bincode deserialization errors:

```
Browser [error]: Failed to launch TUI: Failed to deserialize cards: tag for enum is not valid, found 18
```

## Background

The merge from origin/main introduced:
1. `script_name: Option<String>` field to `CardDefinition` struct
2. Documentation changes for `# Errors` clippy lint

## Symptoms

- Native tests pass (809 tests)
- WASM compilation succeeds
- Data export succeeds
- But WASM deserialization fails in browser

The error suggests enum tag mismatches between native-exported data and WASM deserialization.

## Investigation Done

- Rebuilt both release binary and WASM from scratch multiple times
- Verified same source code, same commit (d74b320)
- Checked for feature-gated struct fields (none found)
- Confirmed serde/bincode versions match between targets
- No conditional compilation affecting serializable types

## Workaround

Skip WASM e2e tests temporarily until root cause is identified.

## Potential Causes

1. Bincode format incompatibility between native and WASM targets
2. Hidden dependency version differences
3. wasm-bindgen serialization differences
4. Unknown serde/bincode interaction with Option<String> at end of struct

## Related Commits

- cbc11bf: Merge that introduced the changes
- d74b320: Current HEAD with doc fixes
