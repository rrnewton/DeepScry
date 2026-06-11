"""Regression tests for scripts/check_beads_dup_keys.py (mtg-742 guard).

A 3-way TEXT merge of two branches that each `mb update`d the same tracker
issue leaves a DUPLICATE top-level `updated_at:` line in the YAML frontmatter.
That ambiguous YAML breaks `mb list` / `mb show` for the WHOLE .beads dir; it
recurred four times in one night during release ceremonies. The checker (wired
into `make validate` as lint.beads-dupkey, and CI via that step) must HARD-FAIL
on such a file, must NOT false-positive on legal repeated *list items* or nested
keys, and `--repair` must collapse a dup to the LATEST occurrence (newest
timestamp).

These tests load the checker by path (same idiom as test_validate_flags.py) and
exercise it via its public functions + the `main()` exit code.
"""
import importlib.util
from pathlib import Path

_SPEC = importlib.util.spec_from_file_location(
    "check_beads_dup_keys",
    Path(__file__).resolve().parent.parent / "scripts" / "check_beads_dup_keys.py",
)
guard = importlib.util.module_from_spec(_SPEC)
_SPEC.loader.exec_module(guard)


# --- the corruption shape that actually bit us (mtg-742) -------------------
_DUP_UPDATED_AT = (
    "---\n"
    "title: Durable deck storage\n"
    "status: in_progress\n"
    "created_at: 2026-06-03T21:13:12.163158889+00:00\n"
    "updated_at: 2026-06-10T05:00:00.000000000+00:00\n"
    "updated_at: 2026-06-11T05:27:20.020257026+00:00\n"
    "---\n"
    "\n"
    "# Description\n"
    "body text\n"
)

_CLEAN = (
    "---\n"
    "title: Durable deck storage\n"
    "status: in_progress\n"
    "created_at: 2026-06-03T21:13:12.163158889+00:00\n"
    "updated_at: 2026-06-11T05:27:20.020257026+00:00\n"
    "---\n"
    "\n"
    "# Description\n"
    "body text\n"
)

# labels: is a top-level key whose VALUE is a block list — the repeated `- web`
# items and the repeated label value must NOT be flagged as duplicate keys.
_LABELS_LIST = (
    "---\n"
    "title: x\n"
    "labels:\n"
    "- design\n"
    "- web\n"
    "- design\n"
    "status: open\n"
    "updated_at: 2026-06-11T00:00:00Z\n"
    "---\n"
    "body\n"
)


def test_detects_duplicate_updated_at():
    dups = guard.find_duplicate_keys(_DUP_UPDATED_AT)
    assert dups == {"updated_at": 2}, dups


def test_clean_file_has_no_dups():
    assert guard.find_duplicate_keys(_CLEAN) == {}


def test_labels_block_list_not_flagged():
    # Repeated list items / a multi-value labels block must not count as a
    # duplicate top-level key.
    assert guard.find_duplicate_keys(_LABELS_LIST) == {}


def test_no_frontmatter_is_clean():
    assert guard.find_duplicate_keys("# just a heading\nno frontmatter\n") == {}


def test_repair_keeps_latest_updated_at():
    fixed = guard.repair_text(_DUP_UPDATED_AT)
    # Exactly one updated_at line, and it is the NEWEST timestamp.
    updated_lines = [ln for ln in fixed.splitlines() if ln.startswith("updated_at:")]
    assert len(updated_lines) == 1, updated_lines
    assert "2026-06-11T05:27:20" in updated_lines[0]
    assert "2026-06-10T05:00:00" not in fixed
    # Repair must be idempotent and produce a clean file.
    assert guard.find_duplicate_keys(fixed) == {}
    # Other keys + body survive untouched.
    assert "title: Durable deck storage" in fixed
    assert "created_at: 2026-06-03T21:13:12.163158889+00:00" in fixed
    assert "# Description" in fixed


def test_main_exit_code_fails_on_dup(tmp_path):
    bad = tmp_path / "mtg-dup.md"
    bad.write_text(_DUP_UPDATED_AT, encoding="utf-8")
    assert guard.main([str(bad)]) == 1


def test_main_exit_code_passes_on_clean(tmp_path):
    good = tmp_path / "mtg-clean.md"
    good.write_text(_CLEAN, encoding="utf-8")
    assert guard.main([str(good)]) == 0


def test_main_repair_then_passes(tmp_path):
    f = tmp_path / "mtg-fix.md"
    f.write_text(_DUP_UPDATED_AT, encoding="utf-8")
    assert guard.main(["--repair", str(f)]) == 0
    # File is now clean on disk.
    assert guard.main([str(f)]) == 0
    assert "2026-06-11T05:27:20" in f.read_text(encoding="utf-8")


def test_directory_arg_scans_md_files(tmp_path):
    (tmp_path / "ok.md").write_text(_CLEAN, encoding="utf-8")
    (tmp_path / "bad.md").write_text(_DUP_UPDATED_AT, encoding="utf-8")
    assert guard.main([str(tmp_path)]) == 1


def test_real_beads_dir_is_clean():
    # The committed .beads/issues tree must never carry a latent dup — this is
    # the same check `make validate` runs, pinned as a unit test so a bad merge
    # is caught even by a bare `pytest agentplay/`.
    beads = Path(__file__).resolve().parent.parent / ".beads" / "issues"
    if not beads.is_dir():
        return  # fresh checkout without beads — nothing to assert
    assert guard.main([str(beads)]) == 0
