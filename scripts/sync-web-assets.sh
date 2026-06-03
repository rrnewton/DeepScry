#!/usr/bin/env bash
# Populate the (gitignored) web/ icon + logo slots from the `assets` submodule.
#
# The `assets` submodule (deepscry-assets) is wired as `update = none`, so it is
# NOT checked out by default — normal clones, agent worktrees, CI, and deploys
# never pay for the multi-MB design masters. This script does the explicit,
# on-demand checkout and copies the small web derivatives into web/ under the
# stable names that web/index.html + web/site.webmanifest reference.
#
# The web/ targets are gitignored on purpose (mtg-forge-rs NEVER commits
# images); the served filenames are content-addressed (CAS-hashed) by the web
# build at deploy time (mtg-k935c). Run this before `make validate` (so the e2e
# web tests see the icons) and before `deploy-cloud.sh deploy`.
set -euo pipefail

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
repo="$(dirname "$here")"
cd "$repo"

echo "→ checking out inactive 'assets' submodule on demand"
git submodule update --init --checkout assets

src="$repo/assets"
dst="$repo/web"
[ -d "$src/emblem" ] || { echo "ERROR: $src/emblem missing after submodule checkout" >&2; exit 1; }

# Copy table: <source-in-submodule> <dest-in-web>. Dest names are the stable
# references used by web/index.html and web/site.webmanifest.
copy() {
    local s="$src/$1" d="$dst/$2"
    [ -f "$s" ] || { echo "ERROR: missing asset $s" >&2; exit 1; }
    cp -f "$s" "$d"
    echo "   $1 -> web/$2"
}

copy logos/deepscry_logo_512.webp deepscry_logo.webp   # hero banner
copy emblem/favicon.ico           favicon.ico          # classic favicon
copy emblem/emblem_32.png         favicon-32.png       # 32px PNG favicon
copy emblem/emblem_180.png        apple-touch-icon.png # iOS home-screen
copy emblem/emblem_192.png        icon-192.png         # PWA manifest
copy emblem/emblem_512.png        icon-512.png         # PWA manifest
copy emblem/emblem_64.webp        emblem-64.webp       # footer/section accent

echo "✓ web assets synced from assets submodule"
