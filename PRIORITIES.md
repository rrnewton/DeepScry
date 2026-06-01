# Current Priorities — Lobby / Web Rationalization + Infra

_Pointer index only. **Minibeads (`mb`) is the canonical source of truth.**_
_Snapshot 2026-05-31; live deploy `9d125ae2`. Reorg in progress: not pushing/deploying._

## P1 — Lobby UX (do this first; design before code)
- **mtg-khy7x** — Lobby UX redesign: write a storyboard + reconcile client/server
  state, kill the duplicate deck-pickers and double create-game click. Blocks the
  other lobby issues below (they implement against the agreed flow).

## P2 — Web/lobby correctness & architecture
- **mtg-tnsk7** — Strict layering: network + player-controller logic SHARED across
  TUI/GUI (only renderer differs). Gates native-GUI network work.
- **mtg-1vwpd** — One common query-parameter dispatch interface for
  `native_game.html` + `tui_game.html` (via `lobby_launcher.js`).
- **mtg-dw9j3** — Lobby game liveness/heartbeat: drop stale waiting games when the
  host browser/WS dies.
- **mtg-33fmb** — Content-addressing hole: `cards.bin` + harness pages still fetch
  fixed-name data bins.

## P3 — Infra / tooling
- **mtg-gy77e** — Standardized + flexible test-binary strategy (env signal for a
  prebuilt `mtg` binary + feature-flag verification). Supersedes mtg-sto4q.
- **mtg-nisrk** — Cloudflare cache purge: wire a purge command + creds (one-time
  card-image purge still pending — needs a CF token).

## Context — recently shipped (live @9d125ae2)
- Cache-skew multiplayer fix — **mtg-wwzw8** (closed).
- Lobby Phase 1 deck-picker + waiting room — **mtg-465** (closed).
- Phase 2 native-GUI network + shared launcher + TUI "Custom Network Game" — **mtg-187**.
- **mtg-612** (content-address decks/tokens) — done; remaining hole tracked by mtg-33fmb → close.
- **mtg-sto4q** — superseded by mtg-gy77e. **mtg-4a1f5** image-gate regression. **mtg-wvn3d** validate-precheck self-flag.

## Separate workstream — deck compatibility (paused)
- Goal **mtg-pph0s** / tracker **mtg-34**. Decks done+merged: jeskai, trolldisk,
  robots, rogue (mtg-387 29/31 @ integration `50612dba`, not deployed).
- `worktrees/compat-finish-monoblack` is **paused** with uncommitted Icy
  Manipulator WIP (mtg-511); mono-black/thedeck sweeps not finished.
- Hard blockers: **mtg-pqtqm** (deterministic dexterity-flip model) ↔ **mtg-389**
  (Chaos Orb) ↔ **mtg-503** (Falling Star). Follow-ups: mtg-400 (Animate Dead),
  mtg-c4p8f (Sylvan Library), mtg-489/mtg-152 (Chain Lightning copy).

## Held for user — do NOT auto-merge / deploy / push
- PR #11 **mtg-614** (wasm rewind/replay), **mtg-589**, **mtg-599**.
- integration is 1 commit (`50612dba`, rogue) ahead of live main `9d125ae2`;
  engine-compat changes staged, not deployed — batch-deploy decision deferred to user.
