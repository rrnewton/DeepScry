---
title: 'Web: consolidate ?allow_local_img_load into ?advanced_options=true (img + multiplayer seed field); rename ''Random AI'' -> ''Random'''
status: open
priority: 3
issue_type: feature
created_at: 2026-06-05T13:52:32.690333177+00:00
updated_at: 2026-06-05T13:52:32.690333177+00:00
---

# Description

Consolidate ?allow_local_img_load into a single ?advanced_options=true; rename 'Random AI' -> 'Random'.

REPORTED (user playtest 2026-06-05):

(1) ADVANCED OPTIONS PARAM: replace the ?allow_local_img_load=true param with one ?advanced_options=true that enables BOTH:
  (a) the 'load images from DeepScry server' checkbox, AND
  (b) a SEED input field for MULTIPLAYER games, on the CREATOR side.
Rationale: for testing we want to run the SAME random/random game multiple times and verify an identical outcome. Seed control is normally solo-only ON PURPOSE - a multiplayer creator who could set the seed could pre-test a known winning hand - so the multiplayer-seed field must stay gated behind ?advanced_options=true (testing only), never default.

(2) RENAME: the 'Random' controller is labelled 'Random AI' but it is NOT AI. Rename to just 'Random'. Literal 'Random AI' at web/solo_launcher.html:172,180 and web/launcher.html:225.

Existing hooks: allow_local_img_load handling at web/game_boot_params.js:88, web/lobby_launcher.js:29,103. Related: mtg-477 (long-term image-licensing posture, currently gated behind allow_local_img_load), mtg-663 ('Local' label missing).
