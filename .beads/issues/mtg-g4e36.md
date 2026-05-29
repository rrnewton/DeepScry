---
title: flakiness_stress.py wasm_e2e runner doesn't build WASM (100% false fails)
status: open
priority: 2
issue_type: bug
created_at: 2026-05-29T09:19:07.606573884+00:00
updated_at: 2026-05-29T09:19:07.606573884+00:00
---

# Description

flakiness_stress.py wasm_e2e runner doesn't build WASM → 100% false failures

Discovered during the mtg-604 overnight baseline (2026-05-29, integration eff46373).

`scripts/flakiness_stress.py stress-all` recorded all 8 curated wasm_e2e tests
(test_fancy_tui, test_human_input, test_click_and_log, test_font_size_layout,
test_card_size_stability, test_battlefield_layout, test_tapped_rotation,
test_graveyard_overlay) as 100% fail / true-nondeterministic. These are FALSE
positives, not product flakes:

ROOT CAUSE: the wasm_e2e KIND_RUNNER (`_wasm_cmd`) just runs `node <stem>.js`
in web/ with NO wasm build step. It relies on web/pkg/ being current. After the
crate rename mtg-forge-rs → mtg-engine, the on-disk web/pkg/ was stale
(mtg_forge_rs_bg.wasm), so the page requests /pkg/mtg_engine.js → 404 → WASM
never loads → `waitForSelector('#launcher.show')` times out → 100% fail.

`make validate` passes these same tests because validate-wasm-e2e-step rebuilds
wasm first (`make wasm`: wasm-pack build + rm -rf web/pkg + cp mtg-engine/pkg
web/pkg) before running them. web/pkg is gitignored.

FIX OPTIONS:
1. flakiness_stress.py should run `make wasm-dev` (or `make wasm`) once before
   stressing any wasm_e2e name (mirror validate's prep), OR
2. mark wasm_e2e kind as requiring a build-prep precondition the harness asserts
   (fail loudly "run make wasm first" instead of silently 100%-failing), OR
3. drop wasm_e2e from the curated stress_all_names() subset (they need browser
   + built artifacts; not a clean isolated-stress target).

Until fixed, wasm_e2e flakiness rows are only meaningful if web/pkg is freshly
built for the SHA under test.
