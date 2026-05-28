---
title: Rename decks/old_school/06_jeskai_aggro → 04 (mis-numbered duplicate 06)
status: open
priority: 4
issue_type: task
created_at: 2026-05-28T15:29:46.520267059+00:00
updated_at: 2026-05-28T15:29:46.520267059+00:00
---

# Description

LOW PRIORITY / backlog. Fix mis-numbering in decks/old_school/: both the jeskai and troll-disk decks are numbered "06". Rename the jeskai one to "04" (which is skipped):

  decks/old_school/06_jeskai_aggro_joseantonioprieto.dck  -> 04_jeskai_aggro_joseantonioprieto.dck
  decks/old_school/06_jeskai_aggro_joseantonioprieto.txt  -> 04_jeskai_aggro_joseantonioprieto.txt   (sidecar)

Annoying because the name is referenced widely. Reference surface (grep 2026-05-28 on integration, excl target/forge-java/.git):

CODE / SCRIPTS / DOCS (small, easy):
- mtg-benchmarks/benches/game_benchmark.rs (~L426): deck1_path string "decks/old_school/06_jeskai_aggro_joseantonioprieto.dck". NOTE the benchmark NAME is "jeskai_trolldisk" (not tied to the 06 number) so the perf-history CSVs are keyed on the name, NOT the path -> historical benchmark data is UNAFFECTED by the rename (experiment_results/ had 0 path refs).
- scripts/plot_performance_interactive.py (~L597): a label/path pair referencing the .dck.
- decks/old_school/README.md (~L22): "5. 06_jeskai_aggro_... - Jeskai aggro by Jose Antonio Prieto".
- deck_list (generated file, L15): regenerate with `make deck_list` after the rename rather than hand-editing.

BEADS (~76 references — the bulk): the jeskai deck tracker (mtg-561) plus its per-card issues cite the deck file path / "06 Jeskai". Update with a careful sed across .beads/issues/ for "06_jeskai_aggro_joseantonioprieto" -> "04_jeskai_aggro_joseantonioprieto" (and prose "06 Jeskai" -> "04 Jeskai" where it denotes the deck number). DO THIS IN A QUIESCENT BEADS WINDOW (no in-flight feature branch editing those issues), same discipline as the numeric-ID renumber, to avoid merge conflicts.

WASM / card packs: the per-set bins (mtg-6fsjb) build from cardsfolder/, not deck files. Deck enumeration for the WASM build / `mtg export-wasm` / deck_list uses globs over decks/old_school/*.dck, so a filename change is picked up automatically. grep found 0 refs under web/. Verify no hardcoded "06_jeskai" sneaks into a generated web data file after rebuild.

STEPS:
1. git mv the .dck + .txt to 04_.
2. Update game_benchmark.rs, plot_performance_interactive.py, decks/old_school/README.md.
3. `make deck_list` to regenerate.
4. Quiescent-window sed across .beads/issues/ for the path + "06 Jeskai" deck-number prose; verify mtg-561 + per-card issues read correctly.
5. make validate green; rebuild wasm; grep web/ generated data for stray "06_jeskai".
6. Optionally re-run the benchmark (name unchanged, so it just continues the same series).

Priority: low. Risk: low blast radius in code (4 spots) but tedious in beads (~76). Do opportunistically, ideally bundled with a beads-renumber quiescent window.
