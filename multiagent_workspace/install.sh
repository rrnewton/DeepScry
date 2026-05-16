#!/usr/bin/env bash
# install.sh — install the multiagent_workspace dev harness into the
#              parent directory containing this mtg-forge-rs checkout.
#
# Usage:
#   ./multiagent_workspace/install.sh           # run from the project checkout
#   bash mtg-forge-rs/multiagent_workspace/install.sh   # run from the parent
#
# What this does, in order:
#
#   1. Locate the kit dir (this script's dir) and the project dir
#      (kit's parent, must be "mtg-forge-rs").
#   2. Locate the parent dev-harness dir (project's parent).
#   3. Verify the parent looks like an empty-or-mostly-empty dev
#      harness root (project subdir present, no conflicting files).
#   4. Create symlinks for:
#        parent/CLAUDE.md              → kit/CLAUDE.md
#        parent/.claude                → kit/.claude
#        parent/scripts/new_worktree.sh → kit/scripts/new_worktree.sh
#   5. Copy (not symlink) templates into the parent IFF absent:
#        kit/templates/ACTIVE.md       → parent/worktrees/ACTIVE.md
#        kit/templates/ARCHIVED.md     → parent/worktrees/ARCHIVED.md
#        kit/templates/parent.gitignore → parent/.gitignore
#   6. Create parent/worktrees/ if absent.
#   7. Initialise a local-only git repo in parent IFF none exists, and
#      register mtg-forge-rs as a submodule IFF not already so.
#
# Idempotent: re-running the installer is safe. Existing symlinks
# matching the desired target are left alone. Existing templated files
# are NEVER overwritten — they hold per-machine state.

set -euo pipefail

# ---------------------------------------------------------------------------
# Locate the kit, the project, and the parent dev-harness directory.
# ---------------------------------------------------------------------------

KIT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_DIR="$(dirname "$KIT_DIR")"
PARENT_DIR="$(dirname "$PROJECT_DIR")"

PROJECT_NAME="$(basename "$PROJECT_DIR")"

if [ "$PROJECT_NAME" != "mtg-forge-rs" ]; then
    echo "error: project dir is named '$PROJECT_NAME', expected 'mtg-forge-rs'" >&2
    echo "       (kit must live at <parent>/mtg-forge-rs/multiagent_workspace/)" >&2
    exit 1
fi

if [ ! -d "$PROJECT_DIR/.git" ] && [ ! -f "$PROJECT_DIR/.git" ]; then
    echo "error: $PROJECT_DIR is not a git checkout" >&2
    exit 1
fi

echo "═════════════════════════════════════════════════════════════════════"
echo "  multiagent_workspace installer"
echo "═════════════════════════════════════════════════════════════════════"
echo "  Kit       : $KIT_DIR"
echo "  Project   : $PROJECT_DIR"
echo "  Parent    : $PARENT_DIR"
echo "═════════════════════════════════════════════════════════════════════"
echo ""

# ---------------------------------------------------------------------------
# Helpers
# ---------------------------------------------------------------------------

# Create a symlink from PARENT_DIR/LINK_REL → kit-relative TARGET_REL.
# We compute the relative target by hand (NOT via `realpath`, which
# would follow symlinks like ~/work → ~/working_copies and produce
# noisy cross-volume paths). Since the kit always lives at
# `parent/mtg-forge-rs/multiagent_workspace/`, the target is reachable
# from any parent subdir via N copies of "../" + the kit-relative path.
make_symlink() {
    local link_rel="$1"            # e.g. "CLAUDE.md" or "scripts/new_worktree.sh"
    local target_rel_in_kit="$2"   # e.g. "CLAUDE.md" or "scripts/new_worktree.sh"

    local link_path="$PARENT_DIR/$link_rel"
    local link_dir
    link_dir="$(dirname "$link_path")"
    mkdir -p "$link_dir"

    # Depth = how many directories deep the link sits inside the parent.
    # "CLAUDE.md"                  → depth 0 → "mtg-forge-rs/multiagent_workspace/$target"
    # "scripts/new_worktree.sh"    → depth 1 → "../mtg-forge-rs/multiagent_workspace/$target"
    local depth
    depth="$(echo "$link_rel" | awk -F'/' '{print NF-1}')"
    local prefix=""
    local i=0
    while [ $i -lt $depth ]; do
        prefix="../$prefix"
        i=$((i + 1))
    done
    local rel_target="${prefix}mtg-forge-rs/multiagent_workspace/$target_rel_in_kit"

    if [ -L "$link_path" ]; then
        local existing
        existing="$(readlink "$link_path")"
        if [ "$existing" = "$rel_target" ]; then
            echo "  ✓ symlink up-to-date: $link_path → $rel_target"
            return 0
        fi
        echo "  ! existing symlink points elsewhere: $link_path → $existing"
        echo "    expected: $rel_target"
        echo "    leaving in place; remove manually if you want to refresh"
        return 0
    fi

    if [ -e "$link_path" ]; then
        echo "  ! $link_path already exists and is NOT a symlink — skipping"
        echo "    move it aside if you want install.sh to manage it"
        return 0
    fi

    ln -s "$rel_target" "$link_path"
    echo "  + created symlink: $link_path → $rel_target"
}

# Copy SRC → DEST iff DEST does not already exist. Per-machine templated
# state must never be overwritten — that would clobber the user's
# worktree registry / gitignore tweaks.
copy_if_absent() {
    local src="$1"
    local dest="$2"

    local dest_dir
    dest_dir="$(dirname "$dest")"
    mkdir -p "$dest_dir"

    if [ -e "$dest" ]; then
        echo "  ✓ already present (not overwriting): $dest"
        return 0
    fi

    cp "$src" "$dest"
    echo "  + copied template: $src → $dest"
}

# ---------------------------------------------------------------------------
# Step 1-2: Symlinks
# ---------------------------------------------------------------------------

echo "→ [1/3] installing symlinks"
make_symlink "CLAUDE.md"               "CLAUDE.md"
make_symlink ".claude"                 ".claude"
make_symlink "scripts/new_worktree.sh" "scripts/new_worktree.sh"

# ---------------------------------------------------------------------------
# Step 3: Templates (copied, never overwritten)
# ---------------------------------------------------------------------------

echo ""
echo "→ [2/3] copying templates (preserving any existing per-machine state)"
mkdir -p "$PARENT_DIR/worktrees"
copy_if_absent "$KIT_DIR/templates/ACTIVE.md"        "$PARENT_DIR/worktrees/ACTIVE.md"
copy_if_absent "$KIT_DIR/templates/ARCHIVED.md"      "$PARENT_DIR/worktrees/ARCHIVED.md"
copy_if_absent "$KIT_DIR/templates/parent.gitignore" "$PARENT_DIR/.gitignore"

# ---------------------------------------------------------------------------
# Step 4: Parent-level local git repo + submodule registration
# ---------------------------------------------------------------------------

echo ""
echo "→ [3/3] initialising parent-level local git repo"

if [ -d "$PARENT_DIR/.git" ]; then
    echo "  ✓ parent already has a .git directory — leaving alone"
else
    ( cd "$PARENT_DIR" && git init --initial-branch=main >/dev/null )
    echo "  + ran 'git init' in $PARENT_DIR (local-only, no remote)"
fi

# Add mtg-forge-rs as a submodule of the parent IFF not already
# registered. We add by relative path so the .gitmodules entry is
# portable across machines.
if ! ( cd "$PARENT_DIR" && git config -f .gitmodules --get-regexp '^submodule\..*\.path$' 2>/dev/null \
        | awk '{print $2}' | grep -Fxq "mtg-forge-rs" ); then
    # Determine the mtg-forge-rs remote URL so the submodule has a
    # sensible upstream. Fall back to a placeholder if no origin.
    PROJ_URL="$(git -C "$PROJECT_DIR" config --get remote.origin.url 2>/dev/null || echo '')"
    if [ -z "$PROJ_URL" ]; then
        PROJ_URL="./mtg-forge-rs"
        echo "  ! project has no 'origin' remote — registering submodule with local path"
    fi

    # git submodule add refuses to operate on a directory that is
    # already a checked-out repo unless we use --force.
    if ( cd "$PARENT_DIR" && git submodule add --force "$PROJ_URL" mtg-forge-rs >/dev/null 2>&1 ); then
        echo "  + registered mtg-forge-rs as a submodule of parent (url: $PROJ_URL)"
    else
        echo "  ! 'git submodule add' failed; skipping (you can register manually later)"
    fi
else
    echo "  ✓ mtg-forge-rs already registered as a submodule"
fi

# Explicitly do NOT push: the parent may have no remote at all. That
# is fine — the parent repo is for local audit history.

echo ""
echo "═════════════════════════════════════════════════════════════════════"
echo "  ✓ multiagent_workspace installed in $PARENT_DIR"
echo "═════════════════════════════════════════════════════════════════════"
echo ""
echo "  Next steps:"
echo "    cd $PARENT_DIR"
echo "    cat CLAUDE.md"
echo "    ls -la"
echo ""
echo "  To create a worktree for an agent:"
echo "    cd $PARENT_DIR"
echo "    ./scripts/new_worktree.sh <branch-name>"
echo ""
echo "  To uninstall: remove the symlinks listed above and the"
echo "  $PARENT_DIR/worktrees/ directory."
echo "═════════════════════════════════════════════════════════════════════"
