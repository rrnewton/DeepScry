---
title: 'TRACK: Old School 1994 deck ''05 Mono Black Rogerbrand'' — full compatibility'
status: open
priority: 1
issue_type: task
created_at: 2026-05-28T02:03:18.295090790+00:00
updated_at: 2026-05-28T02:03:18.295090790+00:00
---

# Description

TRACK: full compatibility for the 1994 Old School deck '05 Mono Black Rogerbrand'.

Deck file: decks/old_school/05_mono_black_rogerbrand.dck
Filed as part of the 1994 Old School playtest skeleton on 2026-05-27_#2334(496405da).

GOAL: every unique card in this deck reaches CARD STATUS: WORKING (per
.claude/skills/compatibility_tracking/SKILL.md) so the deck can be played
end-to-end through `mtg tui` against another 1994 Old School deck without
any silent-drop, unimplemented-effect, or sentinel-log failures.

This is a tracking issue. Per-card verification lives in the linked
'Card Compatibility:' issues. Update those, not this one, when running
playtests. When all per-card issues are WORKING, this issue can be closed
along with a green deck-vs-deck `mtg tui` reproducer logged here.

== Per-card tracking issues ==

### Main deck
- 4x **Black Knight** — mtg-60ca48
- 4x **Hypnotic Specter** — mtg-6mh6e
- 1x **Will-o'-the-Wisp** — mtg-vy10z
- 3x **Juzám Djinn** — mtg-8orbh
- 2x **Sengir Vampire** — mtg-7a1f62
- 2x **Royal Assassin** — mtg-fc8x8
- 1x **Sol Ring** — mtg-1qlk9
- 1x **Mox Jet** — mtg-fa9c28
- 1x **Black Lotus** — mtg-55mcj
- 1x **Chaos Orb** — mtg-ad79fd
- 2x **Icy Manipulator** — mtg-hkcap
- 4x **Dark Ritual** — mtg-i5uqa
- 1x **Mind Twist** — mtg-nrdks
- 1x **Drain Life** — mtg-nuzxj
- 1x **Demonic Tutor** — mtg-w0dfs
- 4x **Sinkhole** — mtg-f90yx
- 2x **Paralyze** — mtg-z9epj
- 1x **Greed** — mtg-q4pcp
- 4x **Underworld Dreams** — mtg-b52vd
- 15x **Swamp** — mtg-53coi
- 1x **Strip Mine** — mtg-36d76b
- 1x **Library of Alexandria** — mtg-nbriu
- 3x **Mishra's Factory** — mtg-voj6u

### Sideboard
- 4x **Su-Chi** — mtg-wo7v3
- 4x **Gloom** — mtg-z0fji
- 3x **Terror** — mtg-ta3r4
- 1x **Sengir Vampire** — mtg-7a1f62
- 2x **Maze of Ith** — mtg-pp05u
- 1x **Paralyze** — mtg-z9epj


== Recommended end-to-end reproducer (when all per-card issues are WORKING) ==
./target/release/mtg tui \
  decks/old_school/05_mono_black_rogerbrand.dck \
  decks/old_school/<opponent>.dck \
  --p1=heuristic --p2=heuristic --seed 42 --verbosity 2

== Status ==
DECK STATUS: skeleton — see per-card issues for granular state.
