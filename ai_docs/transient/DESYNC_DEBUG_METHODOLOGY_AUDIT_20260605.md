This is a synthesis task — I have all four survey findings in hand and need to produce a decision-ready report. No code exploration is needed; the findings are grounded with file:line citations. Let me write the report directly.

PLAIN-LANGUAGE: Below, "desync" means the server and the player's browser (client) disagreeing about the game state; "point of first divergence" is the exact first moment they disagree; "action_count" is a counter of how many game-state changes have happened (a position marker in the change-log); "view hash" is a single number that fingerprints the visible game state.

---

# Desync-Divergence Debugging: Methodology + Tooling Audit

## 1. CURRENT METHODOLOGY — how a desync is debugged today

A desync is the server and a player's browser (the "client" / WASM shadow) disagreeing about game state. Today's flow has a sharp, well-automated front half and a tribal, hand-cranked back half.

**What's codified and automatic (the front half):**

- **Detection is automatic, immediate, and fatal.** At every choice the player submits, the server synchronously checks two things in the coordinator task: (1) the client's `action_count` (its position in the change-log) matches the server's, and (2) the client's view-hash (a single-number fingerprint of visible state) matches the server's. Either mismatch terminates the game instantly with a `FatalError` to both sides — no retry, no tolerance (`server.rs:2548,2562,2740,2754`; principle in `NETWORK_ARCHITECTURE.md:22-62`).
- **The point of first divergence is found automatically — NOT by manual bisect.** Because the hash check runs per choice (keyed by a strictly-monotonic `choice_seq`), the engine catches the divergence at the *first* choice where client state ≠ server state. There is no bisection step to locate "when" it first went wrong at the choice granularity — the first-failing choice is the divergence point, reported the instant it happens.
- **A first-cut field diff is codified.** On mismatch the server emits three layers: (1) a FATAL log line with both hex hashes + `choice_seq` + `action_count` + player ID; (2) a structured `DebugSyncInfo` dump with a per-field DIFFERENCES block (life, hand/library/graveyard sizes, battlefield count, plus per-card `(id, tapped, controller)` and per-player graveyard IDs); (3) an `SRV_P1_RECV` telemetry line (`server.rs:251-384`; `protocol.rs:1394-1612`). The mtg-668 "class-A" path (a category of bug where coarse sizes all match but the hash still differs) even pinpoints the exact diverging card and field: `card 12345: server(tapped,ctrl)=(true,1) client=(false,0)`.

**What's tribal and hand-cranked (the back half — locating root cause *within* the diverging choice):**

Once you know *which choice* and *which field* diverged, finding *why* is re-derived by hand every time:

- **action_count is deliberately NOT in the hash** (legitimate client/server drift in the replica model — mtg-668/mtg-744), so it's validated on a separate track. When sizes all match but hashes differ, agents fall back to **dumping both sides' undo-logs (the change-log) and manually diffing the action *sequences*** to find which actions the client skipped. There is no tool that aligns the two logs and names the first diverging action — agents `diff` by hand or write a one-off script in `debug/`.
- **The hardest cases** (mtg-668 seed-2 Timetwister) took *hours*, not because detection was slow, but because the browser's `seq↔hash` bookkeeping was misaligned and the resolution required manual undo-log sequence diffing (`mtg-728.md:226-296`).
- **The end-to-end workflow itself is tribal.** The byte-pin → bisect → both-sides action_count-stamped trace → classify-field → root-cause methodology is **NOT codified** as a skill or runbook. It lives as post-facto narrative in beads issues (mtg-677, mtg-725, mtg-752) and scattered ad-hoc scripts. No `.claude/skills/desync-debugging/` exists.

**Honest bottom line:** *Divergence localization at the choice/field level is excellent and automatic. Root-causing within the diverging choice is manual, undocumented, and re-invented per bug.*

## 2. TOOLING INVENTORY — what exists today

| Tool | Status | Gate | What it gives you |
|---|---|---|---|
| `compute_view_hash()` folded u64 hash | Scripted | always (network mode) | Single fingerprint of ~11-14 visible fields (`state_hash.rs:396-480`) |
| Per-choice hash + action_count validation | Scripted | `--network-debug` | Automatic, immediate, fatal first-divergence detection (`server.rs:2548-2584`) |
| `DebugSyncInfo` + `::diff()` | Scripted | `--network-debug` | Structured per-field both-sides diff → `Vec<String>` like `hand_sizes: [7,8] vs [7,7]` (`protocol.rs:1394-1612,1520-1581`) |
| `log_state_differences()` / `log_state_hash_mismatch()` | Scripted | on mismatch | Formatted box: hashes, per-field DIFFERS flags, per-card battlefield detail, graveyard IDs (`server.rs:251-384`) |
| `WASM_HASH_DEBUG` / `WASM_CARD_DETAIL` | Scripted | `--network-debug` | Per-choice client-side hash + per-card `(id,tapped,ctrl)` (`local_controller.rs:224-260`) |
| `WASM_FULL_UNDO_DUMP` (60 / 120 on mismatch) | Scripted | `--network-debug` | Tail of client change-log; 120 + differential on action_count mismatch (`local_controller.rs:269-301`) |
| `SERVER_FULL_UNDO_DUMP` | Scripted | `--network-debug` **+** `MTG_NET_FULL_UNDO_DUMP=1` | Tail of 120 server actions (opt-in) (`controller.rs:320-326`) |
| `WASM_SUBMIT` wire snapshot | Scripted | `--network-debug` | Ground-truth of what client *sent* (`client.rs:1931`) |
| E2E dump aggregation | Scripted | auto on failure | Regex-extracts both undo dumps + mismatch + card-detail to `debug/netarch-undo-dumps/` (`test_network_gui_e2e.js:385-434`) |
| Stale-binary guard (WASM) | Scripted | Makefile only | `rm -rf pkg/` before wasm rebuild (mtg-475) — but NOT enforced pre-test |
| `desync_canary.sh` broad-seed sweep | Scripted | `make validate-desync-canary` (opt-in) | 6 seeds × 3 controllers green corpus + KNOWN_RED tier |
| **Targeted `println!`/TRACE field probes** | **Ad-hoc** | by hand | Pinpoint which line *writes* a field (0% reuse — re-added every bug) |
| **Undo-log sequence alignment/diff** | **Ad-hoc** | by hand | Find first diverging action between two logs (0% reuse — no tool exists) |
| **Reserved-ID branch-on-absence validation** | **Ad-hoc** | by hand | Detect when the client skipped an action it should have tracked (0% reuse) |
| **mtime-verify before network test** | **Discipline only** | none | "binary newer than source" — pure memory, no gate |

## 3. GAPS — the painful, repeated friction (each anchored in evidence)

**G1. "Two bugs in one hash" — the folded hash collapses ~14 fields into one number.** When coarse sizes all match but the hash differs, you enter Tier-2/Tier-3 difficulty. The mtg-668 seed-2 case shows this directly: all coarse sizes matched, and pinpointing that cards 49 & 59 had divergent tap status required per-card iteration + binary search (`mtg-728.md:226-296`). Worse, a hash mismatch could in principle reflect *two* simultaneously-diverging fields, and you only learn that after the structured diff runs. Tier-3 (hash differs but all detail fields match, or `--network-debug` was off) drops to a full-JSON manual diff measured in *hours*.

**G2. No automatic first-diverging-*action* finder.** First-diverging-*choice* is automatic; first-diverging-*action within the change-log* is not. Agents dump both undo-logs and `diff` by hand or write throwaway alignment scripts (per-bug, `debug/`, 0% reuse). Evidence: mtg-677 "byte-pin seed-2 divergence" and the mtg-668 resolution both required manual action-sequence diffing because the divergence lived in *which actions the client skipped*, invisible to the field-level hash diff.

**G3. Agents re-add ad-hoc `TRACE`/`println!` probes every divergence.** The scripted dumps show *which* field/choice diverged but not *which code path wrote it*. Task #25 ("Probe + root-pin non-logging tap writer (card 14)") is the archetype: agents hand-code prints into field writers to find the offending line. `format_view_card_detail()` itself only exists *because* it was first invented ad-hoc for mtg-668. There is no both-sides, action_count-stamped, field-level trace mode to turn on instead.

**G4. Stale-binary trap is a discipline-only landmine.** e2e/canary/validate run a *prebuilt* binary; editing `.rs` then testing immediately tests the OLD build → false-positive green (this is a recorded repeat offender — bit two agents on the netarch Phase-1 "buffer drives WASM" work; memory: rebuild-binary-before-network-test). The Makefile guards the WASM `pkg/` nuke (mtg-475), but **nothing enforces "binary mtime > source mtime" before a network test fires**. mtime-verify is cited as *manual* discipline across mtg-677/mtg-725/mtg-752.

**G5. A data-field watchpoint misses free+realloc (entity lifecycle blind spot).** mtg-668 class-A root cause was the client *skipping actions* at `branch_on_absence(reserved_id)` sites — i.e., an entity-lifecycle (clear/insert/reuse of IDs) divergence, not a value-write divergence. A watchpoint on "is card X tapped?" never fires when the bug is "card slot was cleared and a different entity reallocated into that ID." There is no entity clear/insert tracer. Also note a structural blind spot: opponent hand *contents* and library *order* are excluded from the hash (hidden info), so the shadow can hold wrong opponent-deck contents and still pass the view hash — a lifecycle/contents divergence that detection cannot even see.

**G6. The whole methodology is tribal — no runbook/skill.** The byte-pin → bisect → both-sides trace → classify → root-cause workflow, plus the trap list (network-debug always-on, mtime-verify, broad-seed canary before declaring green), exists only as scattered beads narrative and ad-hoc scripts. No `.claude/skills/desync-debugging/SKILL.md`. Every new agent re-derives it, and the green-masks-a-coverage-gap trap (a fix that's green only because the gate ran the one clean seed) recurs precisely because the canary step isn't codified into the workflow.

## 4. PRIORITIZED PROPOSALS

Each item: **name — what it does — effort — expected speedup.**

**P1 (a). `state_diff.rs` — structured both-sides per-field state-diff that names the diverging field(s) instantly.**
A real `diff_full_state(server_debug, client_debug, server_undo, client_undo) -> StructuredStateDiff` module that emits a tree pinpointing every diverging field *and* every diverging card/action in one shot — fixing "two bugs in one hash" by enumerating *all* divergences, not stopping at the first. Replaces the current "Battlefield … DIFFERS" + raw-vec dump with:
```
├─ Coarse fields: [all match]
├─ Battlefield detail: card 49 → tapped: server=true client=false
└─ Action sequence: gap at action 863-904 (server: 42 ChangeZoneAll+Shuffle, client: 0)
```
~300 LoC; 80% of the pieces (`DebugSyncInfo`, detail helpers) already exist. Wire into `server.rs:251-293`. **Effort: M. Speedup: Tier-2 cases minutes→seconds; eliminates the "is it one field or two?" guessing.**

**P2 (b). `bisect_first_divergence` — automatic binary-search-on-action_count first-divergence finder.**
Given a reproducing seed/deck/controller, replays the game on both server-model and shadow-model and binary-searches `action_count` to return the *exact first action* where the two undo-logs/hashes diverge — the missing G2 tool. Subsumes the per-bug hand-`diff` of undo dumps. Pairs naturally with P1 (P1 names the field at the divergence point; P2 finds the action). **Effort: M (the native oracle/replay infra already exists). Speedup: turns hours-long manual action-sequence diffs into a single automated run; deterministic, scriptable in CI.**

**P3 (c). `--field-trace` — built-in both-sides, action_count-stamped, field-level trace debug mode.**
A first-class debug mode that, when enabled, emits a stamped line per field-write on *both* server and client: `[ac=864 seq=37] card49.tapped: false→true (writer=resolve_attack_step)`. Kills G3: agents stop hand-adding `println!` into field writers. Built once, gated like `--network-debug`. **Effort: M-L (instrumentation at the field-writer/undo-apply layer). Speedup: removes the ~per-bug ad-hoc TRACE re-add cycle entirely; makes the writer/code-path obvious without recompiling probes.**

**P4 (d). `net-test-guard.sh` — enforce rebuild + mtime-verify before any network test.**
A thin pre-flight helper: rebuild server+WASM, assert binary mtime > newest source mtime, refuse to run otherwise. Wire into `test_network_gui_e2e.js`, `network_desync_reproducer.sh`, `desync_canary.sh`. Closes G4 — converts the stale-binary trap from discipline to gate. **Effort: S. Speedup: eliminates an entire class of false-green that has already burned ≥2 agents; near-zero cost, high recurrence-prevention value.**

**P5 (e). Entity-lifecycle tracer (clear/insert/realloc).**
A trace mode that logs entity-slot lifecycle events — `clear(id=49)`, `insert(id=49, entity=…)`, reserved-ID `branch_on_absence(id=…)` hits — on both sides, so a free+realloc divergence (the actual mtg-668 class-A root cause) is visible where a value-watchpoint is blind (G5). Could ship as a facet of P3's trace mode. Add a "did the shadow skip an action it should have tracked?" assertion. **Effort: M. Speedup: directly targets the hardest, multi-hour class (skipped-action / reserved-ID) that the field diff structurally cannot catch.**

**P6 (f). `.claude/skills/desync-debugging/SKILL.md` — codified runbook.**
A repeatable checklist wrapping the existing docs + the empirical workflow from mtg-677/mtg-725: (1) always `--network-debug`; (2) run P4's rebuild+mtime gate; (3) capture byte-pinned divergence (choice_seq, action_count, hash hex); (4) run P2's bisect + P1's field diff; (5) **before declaring green, run the broad-seed canary** (`make validate-desync-canary`) — not just the pinned seed, to defeat the green-masks-a-coverage-gap trap. Cites `NETWORK_ARCHITECTURE.md:22-62`, `FUZZ_AND_STRESS_TESTING_STRATEGY.md:102-151`, mtg-677. **Effort: S. Speedup: stops every new agent re-deriving the workflow and re-falling-into the traps; force-multiplies P1-P5.**

**Suggested sequence:** P4 (cheap, stops false-greens today) → P1 (biggest localization win) → P2 (automates the manual diff) → P6 (codify, now that the tools exist to cite) → P3 → P5.

## 5. THE ONE THING — build only one this week

**Build P1: the structured both-sides per-field state-diff (`state_diff.rs`).**

It delivers the most root-cause-speed per unit effort because:

- **It attacks the most-recurring, highest-cost friction directly.** Every divergence passes through "the hash differs — now *which field*?" Today that's a 3-tier slog (Tier-2 = per-card binary search by hand; Tier-3 = hours of JSON diffing). P1 collapses it to a single named-field/named-card/named-action readout, and crucially enumerates *all* divergences at once — the literal "two bugs in one hash" fix.
- **Effort is genuinely M, not L, because 80% already exists.** `DebugSyncInfo`, the detail helpers (`view_battlefield_detail`, `view_graveyard_ids`), and the mismatch log entry point are all built; P1 is ~300 LoC of comparator + a tree formatter wired into `server.rs:251-293`. No changes to `undo.rs` or core game logic.
- **It compounds with everything else.** P2's bisect needs something to *report* at the divergence point — that's P1's output. P6's runbook needs a single canonical "read this diff" step — that's P1. P1 is the shared substrate the rest of the roadmap leans on.

The only serious contender is **P4 (the rebuild+mtime gate)** — it's S-effort and stops a recorded false-green class immediately. If the week's goal is *risk reduction*, do P4 first (it's an afternoon). But for *root-cause speed per unit effort* — the question asked — **P1 wins**: it permanently converts the single most-repeated debugging step from a multi-hour manual investigation into a one-line structured readout.

Relevant source anchors for whoever picks this up: `mtg-engine/src/game/state_hash.rs:396-480` (the folded hash), `mtg-engine/src/network/protocol.rs:1394-1612` (`DebugSyncInfo` + `::diff()`), `mtg-engine/src/network/server.rs:251-384` (mismatch logging — the wire-in point), `mtg-engine/src/network/controller.rs:295-331` and `mtg-engine/src/wasm/network/local_controller.rs:212-310` (both-sides undo dumps to feed the action-sequence diff).