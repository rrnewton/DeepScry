---
title: 'Experiment: ALL-DEBUG validate wall-clock vs release baseline'
status: closed
priority: 3
issue_type: task
created_at: 2026-06-04T11:38:11.437857362+00:00
updated_at: 2026-06-05T13:28:34.991198282+00:00
---

# Description

mtg-717 follow-on (user-approved). HYPOTHESIS: building mtg in DEBUG instead of release may lower TOTAL validate wall-clock — the release build is the long pole AND the source of the unit.nextest->build.mtg-release coupling; debug compiles much faster but runs slower, so the net is UNKNOWN and must be measured.

== RESULT 2026-06-05_#2968(b242cbfb) (slot04-alldebug; DONE) ==
DECISION: KEEP RELEASE as the default `make validate` build. (Optional opt-in debug fast-mode for LOCAL iteration only — never CI/default.)

Capture: experiments/validate_alldebug_20260605/ (README + metadata.json + delta_table.md + make_delta_table.py + both validate logs). Both runs: 33 passed, 0 failed, 0 skipped.
Logs: release validate_logs/validate_b242cbfb4b1f750d52ab0c890a7e4cf9944f2979.log ; debug validate_logs/validate_d4a03d95703e09233e684feb0b0d5304cb2db363_dirty.log.

KNOB USED (single point, branch exp-alldebug-validate, commit 1207fa91, MEASURE-ONLY): changed the build.mtg-release step in scripts/validate.py to `cargo build --bin mtg --features network && cp -f target/debug/mtg target/release/mtg`. Downstream steps resolve target/release/mtg via MTG_REUSE_PREBUILT, so staging the debug build there exercises the debug binary everywhere — no per-step env override needed. Stale-binary check passed: staged target/release/mtg = 414,575,440B (= target/debug/mtg) vs release 490,088,088B.

NUMBERS:
- Robust win: build.mtg-release compile 107s -> 16s (-91s, -85%, debug skips optimization).
- Robust cost: debug binary is slower at runtime on binary-bound steps — network.fuzz 7s->31s (+343%), network.equiv-* +4..6s each, determ.commander 1s->4s.
- Raw wall-clock 811s -> 604s (-207s) and serial-sum 2104s -> 1238s are CONFOUNDED: the two back-to-back runs saw UNEQUAL concurrent load (run1 neighbour slot02 contended far heavier than run2 neighbour slot05). Proof: binary-INDEPENDENT steps that never touch the binary "improved" 2.5x — examples.run 447->189, lint.clippy 392->156, wasm.bundle 137->19 (~638s of the serial drop, none debug-attributable). So the -207s is NOT a debug win.

WHY KEEP RELEASE:
1. No proven wall-clock win. build.mtg-release is not the lone critical path — wall-clock is dominated by binary-independent steps (examples.run, lint.clippy, wasm.bundle) and the serialized network.* group (503s release / 510s debug serial — unchanged, because debug runtime cost cancels the earlier-start gain). Debug-attributable wall-clock upper bound ~90s (serialized network group starts ~91s earlier), partly eaten by slower per-step runtime.
2. CI parity + tests the SHIPPED artifact. CI/deploy ship the RELEASE binary; a desync-always-fatal project must run its determinism/e2e gates against the same optimized codegen — a release-only miscompile/optimization-induced nondeterminism would slip past a debug-only validate.
3. Debug runtime cost lands exactly on the serialized network.* long-pole.

BETTER LEVER for real wall-clock cuts (follow-up, out of scope): attack examples.run, lint.clippy, and the serialized network.* group (parallelize examples, raise the "net" resource capacity if ports allow, cache clippy). The native-binary build profile is not the bottleneck.

CAVEAT: not run on a fully idle box (1 concurrent validate per run, asymmetric). A clean re-measure (quiesce all other slots, or average >=3 pairs) would tighten the wall-clock figure, but would not change the KEEP decision (driven by CI-parity + shipped-artifact principles, independent of the confounded numbers).
