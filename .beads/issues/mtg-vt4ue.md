---
title: 'RandomController: improve targeting logic and mana color selection'
status: open
priority: 4
issue_type: task
created_at: 2026-06-13T16:24:26.215211776+00:00
updated_at: 2026-06-13T16:24:26.215211776+00:00
---

# Description

Two known gaps in RandomController (game/random_controller.rs):\n1. choose_targets(): selects targets randomly without regard to spell requirements (e.g. damage spells prefer opponent creatures, draw spells have no preference). Low priority since random controller is for testing only, not human play.\n2. choose_mana_sources_to_pay(): uses a simple greedy CMC-based approach without optimizing for mana colors. Shuffle + take is fine for randomized testing; a smarter approach might use the ManaEngine resolver.\nBoth are acceptable for a randomized stress-test controller. Only fix if random games start failing due to unplayable situations caused by bad target/mana choices.
