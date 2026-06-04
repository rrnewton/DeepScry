---
title: validate_run.py --dot DAG emit (graphviz)
status: open
priority: 3
issue_type: task
created_at: 2026-06-04T11:38:11.417600189+00:00
updated_at: 2026-06-04T11:38:11.417600189+00:00
---

# Description

mtg-717 follow-on (user-approved). Add a `--dot` flag to scripts/validate_run.py that emits the step DAG from build_registry() as graphviz. Requirements: (1) source = build_registry() (the SAME structure --list reads → drift-free, never a hand-maintained parallel graph); (2) MUST render the implicit 'browser' resource serialization — cap-1 resource steps serialize at runtime with NO dep edge between them, so a pure dep-graph understates real ordering; show it (dashed cluster / annotation / synthetic edges); (3) respect --use-prebuilt / --group / --no-network so the emitted graph matches what WOULD actually run in that mode (e.g. --use-prebuilt drops build.mtg-release + wasm.bundle and their edges). Output .dot to stdout (pipe to `dot -Tsvg`). Verifies the build→jobGroup sharing graph visually.
