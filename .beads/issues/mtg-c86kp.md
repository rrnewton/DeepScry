---
title: 'TRACK: Old School 1994 deck ''06 Jeskai Aggro Joseantonioprieto'' — full compatibility'
status: open
priority: 1
issue_type: task
created_at: 2026-05-28T02:03:18.296486765+00:00
updated_at: 2026-05-28T02:03:18.296486765+00:00
---

# Description

TRACK: full compatibility for the 1994 Old School deck '06 Jeskai Aggro Joseantonioprieto'.

Deck file: decks/old_school/06_jeskai_aggro_joseantonioprieto.dck
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
- 4x **Savannah Lions** — mtg-8wa4q
- 4x **Serendib Efreet** — mtg-lajw2
- 3x **Black Vise** — mtg-pzzk8
- 1x **Mox Ruby** — mtg-bhnoj
- 1x **Mox Jet** — mtg-fa9c28
- 1x **Mox Pearl** — mtg-5490k
- 1x **Mox Sapphire** — mtg-rk9og
- 1x **Sol Ring** — mtg-1qlk9
- 1x **Black Lotus** — mtg-55mcj
- 1x **Chaos Orb** — mtg-ad79fd
- 4x **Lightning Bolt** — mtg-1g699
- 3x **Disenchant** — mtg-2ahn3
- 4x **Psionic Blast** — mtg-b5aum
- 1x **Ancestral Recall** — mtg-w0f5s
- 4x **Chain Lightning** — mtg-mfc3y
- 1x **Fireball** — mtg-ozzx3
- 1x **Wheel of Fortune** — mtg-356951
- 1x **Timetwister** — mtg-0uoxy
- 1x **Time Walk** — mtg-52q8u
- 1x **Demonic Tutor** — mtg-w0dfs
- 1x **Mind Twist** — mtg-nrdks
- 3x **City of Brass** — mtg-ef504b
- 2x **Plateau** — mtg-xb5jc
- 4x **Volcanic Island** — mtg-2b937
- 4x **Tundra** — mtg-apqta
- 2x **Plains** — mtg-uah5f
- 1x **Strip Mine** — mtg-36d76b
- 1x **Library of Alexandria** — mtg-nbriu
- 1x **Badlands** — mtg-qf2gx
- 1x **Scrubland** — mtg-vk7wb
- 1x **Mishra's Factory** — mtg-voj6u

### Sideboard
- 3x **Su-Chi** — mtg-wo7v3
- 2x **Red Elemental Blast** — mtg-9jogx
- 2x **Blue Elemental Blast** — mtg-0fuxr
- 1x **Braingeyser** — mtg-mylhk
- 2x **Psychic Purge** — mtg-2bs8e
- 1x **Divine Offering** — mtg-xivz9
- 1x **Karma** — mtg-05ypp
- 1x **Circle of Protection: Red** — mtg-kvafc
- 1x **Disenchant** — mtg-2ahn3
- 1x **Balance** — mtg-uztk2


== Recommended end-to-end reproducer (when all per-card issues are WORKING) ==
./target/release/mtg tui \
  decks/old_school/06_jeskai_aggro_joseantonioprieto.dck \
  decks/old_school/<opponent>.dck \
  --p1=heuristic --p2=heuristic --seed 42 --verbosity 2

== Status ==
DECK STATUS: skeleton — see per-card issues for granular state.
