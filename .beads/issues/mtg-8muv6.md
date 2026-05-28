---
title: 'TRACK: Old School 1994 deck ''06 Troll Disk Daniellebrunazzo'' — full compatibility'
status: open
priority: 1
issue_type: task
created_at: 2026-05-28T02:03:18.297860060+00:00
updated_at: 2026-05-28T02:03:18.297860060+00:00
---

# Description

TRACK: full compatibility for the 1994 Old School deck '06 Troll Disk Daniellebrunazzo'.

Deck file: decks/old_school/06_troll_disk_daniellebrunazzo.dck
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
- 2x **Ironclaw Orcs** — mtg-uhs0s
- 4x **Savannah Lions** — mtg-8wa4q
- 3x **Serendib Efreet** — mtg-lajw2
- 1x **Mox Pearl** — mtg-5490k
- 1x **Mox Jet** — mtg-fa9c28
- 1x **Mox Sapphire** — mtg-rk9og
- 1x **Mox Ruby** — mtg-bhnoj
- 1x **Sol Ring** — mtg-1qlk9
- 1x **Black Lotus** — mtg-55mcj
- 1x **Chaos Orb** — mtg-ad79fd
- 1x **Ancestral Recall** — mtg-w0f5s
- 1x **Counterspell** — mtg-3pjtj
- 1x **Mana Drain** — mtg-xljen
- 3x **Disenchant** — mtg-2ahn3
- 4x **Psionic Blast** — mtg-b5aum
- 4x **Lightning Bolt** — mtg-1g699
- 1x **Wheel of Fortune** — mtg-356951
- 1x **Braingeyser** — mtg-mylhk
- 1x **Timetwister** — mtg-0uoxy
- 1x **Time Walk** — mtg-52q8u
- 1x **Mind Twist** — mtg-nrdks
- 1x **Demonic Tutor** — mtg-w0dfs
- 4x **Chain Lightning** — mtg-mfc3y
- 2x **Plateau** — mtg-xb5jc
- 2x **Badlands** — mtg-qf2gx
- 4x **Tundra** — mtg-apqta
- 4x **Mishra's Factory** — mtg-voj6u
- 3x **Volcanic Island** — mtg-2b937
- 3x **City of Brass** — mtg-ef504b
- 1x **Library of Alexandria** — mtg-nbriu
- 1x **Strip Mine** — mtg-36d76b

### Sideboard
- 1x **Counterspell** — mtg-3pjtj
- 1x **Balance** — mtg-uztk2
- 1x **Spirit Link** — mtg-25w4a
- 1x **Armageddon** — mtg-2d6ks
- 1x **Divine Offering** — mtg-xivz9
- 1x **Red Elemental Blast** — mtg-9jogx
- 1x **Falling Star** — mtg-2jk5q
- 1x **Fireball** — mtg-ozzx3
- 3x **Copy Artifact** — mtg-arabq
- 1x **Control Magic** — mtg-bpu41
- 1x **Blue Elemental Blast** — mtg-0fuxr
- 2x **Su-Chi** — mtg-wo7v3


== Recommended end-to-end reproducer (when all per-card issues are WORKING) ==
./target/release/mtg tui \
  decks/old_school/06_troll_disk_daniellebrunazzo.dck \
  decks/old_school/<opponent>.dck \
  --p1=heuristic --p2=heuristic --seed 42 --verbosity 2

== Status ==
DECK STATUS: skeleton — see per-card issues for granular state.
