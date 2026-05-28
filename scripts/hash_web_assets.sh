#!/usr/bin/env bash
# hash_web_assets.sh — content-address the wasm-bindgen pkg pair in a STAGED
# web tree, so the JS-glue ↔ .wasm desync (mtg-475 / mtg-2indh) becomes
# structurally impossible: a content change yields a NEW filename, and old
# filenames keep serving old (self-consistent) bytes. The web server can then
# serve `/pkg/<name>.<hash>.{js,wasm}` as `immutable, max-age=1y` safely even
# behind a CDN that overrides headers, because new bytes can only arrive under
# a new URL. (mtg-571)
#
# This is the deploy-time complement to the exporter's content-addressed
# `<set>.<hash>.bin` files: the exporter owns the data bins (the hashed name
# lives in sets/index.json), this script owns the code bundle (the hashed name
# is rewritten into the HTML that imports it).
#
# WHY a script and not full trunk `rel="rust"` adoption (yet): all four game
# pages do `import init, { ...many named exports... } from './pkg/mtg_forge_rs.js'`
# plus dynamic `import('./pkg/...')`. Trunk's rel="rust" replaces a <link> with
# its OWN injected bootstrap and exposes the module as `window.wasmBindings`;
# it cannot serve hand-authored named-export static imports of a hashed name.
# Migrating all four pages to `window.wasmBindings.*` is a large rewrite gated
# behind a separate task. Until then we keep the committed source HTML on the
# fixed-name path (so `make validate`'s e2e tests are unaffected) and do the
# content-addressing on a DEPLOY-STAGING COPY only — exactly like the existing
# `?v=<sha>` cache-bust sed in deploy-cloud.sh, which this replaces.
#
# STRUCTURED REWRITE, not arbitrary HTML munging (per the project's "No Hacky
# String Operations On Structured Data" rule): we rewrite exactly two
# well-defined injection points that we control:
#   1. the ES import specifier  './pkg/mtg_forge_rs.js'  (static + dynamic)
#   2. the no-arg  init()  call -> init({ module_or_path: '<hashed .wasm>' })
# Point 2 is wasm-bindgen's documented `init({ module_or_path })` override, so
# the generated glue's internal `new URL('mtg_forge_rs_bg.wasm', ...)` default
# is bypassed and we need NOT edit the generated glue at all.
#
# USAGE:
#   scripts/hash_web_assets.sh <web_dir>
# where <web_dir> contains pkg/mtg_forge_rs.js + pkg/mtg_forge_rs_bg.wasm and
# the *.html pages that import them. Operates IN PLACE on <web_dir> — point it
# at a staging copy, never the source tree.

set -euo pipefail

WEB_DIR="${1:?usage: hash_web_assets.sh <web_dir>}"
PKG_DIR="$WEB_DIR/pkg"
JS="$PKG_DIR/mtg_forge_rs.js"
WASM="$PKG_DIR/mtg_forge_rs_bg.wasm"

[[ -f "$JS"   ]] || { echo "error: $JS not found (run 'make wasm-network' first)" >&2; exit 1; }
[[ -f "$WASM" ]] || { echo "error: $WASM not found (run 'make wasm-network' first)" >&2; exit 1; }

# 16 hex chars (64 bits) — ample collision margin for a 2-file bundle.
hash_of() { sha256sum "$1" | cut -c1-16; }

# IMPORTANT ordering: the JS glue's internal default reference to the wasm is
# bypassed by the init({module_or_path}) override we inject, so we may hash the
# two files independently. We hash the WASM first (its name goes into the HTML
# init arg), then the JS (whose name goes into the import specifier).
WASM_HASH="$(hash_of "$WASM")"
JS_HASH="$(hash_of "$JS")"

WASM_HASHED="mtg_forge_rs_bg.${WASM_HASH}.wasm"
JS_HASHED="mtg_forge_rs.${JS_HASH}.js"

echo "→ hashing pkg bundle:"
echo "    mtg_forge_rs.js        -> $JS_HASHED"
echo "    mtg_forge_rs_bg.wasm   -> $WASM_HASHED"

# Rename the files. We keep ONLY the hashed names in the staged tree so the
# fixed names are gone (rsync --delete then prunes any old hashed names on the
# VM that the new HTML no longer references — see deploy-cloud.sh GC note).
mv "$JS"   "$PKG_DIR/$JS_HASHED"
mv "$WASM" "$PKG_DIR/$WASM_HASHED"

# Rewrite the HTML pages. Each rewrite is a fixed-token substitution of a
# specifier WE author and control (not a free-form HTML edit):
#   - './pkg/mtg_forge_rs.js'  ->  './pkg/<JS_HASHED>'   (covers both the
#     static `from '...'` import and dynamic `import('...')`)
#   - the bare `init()` call    ->  init({ module_or_path: './pkg/<WASM_HASHED>' })
# We only touch *.html so we never mangle the .js file's own references.
shopt -s nullglob
for f in "$WEB_DIR"/*.html; do
    # 1. import specifier (path-with-leading-dot OR leading-slash form).
    sed -i \
        -e "s|\\./pkg/mtg_forge_rs\\.js|./pkg/${JS_HASHED}|g" \
        -e "s|/pkg/mtg_forge_rs\\.js|/pkg/${JS_HASHED}|g" \
        "$f"
    # 2. no-arg init() -> explicit module_or_path. Matches `await init();`,
    #    `await init()`, and `init();`. We deliberately do NOT touch any
    #    init(<arg>) call that already passes a path (none today, but this
    #    keeps the rewrite idempotent & safe).
    sed -i -E \
        "s@(await )?init\(\)@\1init({ module_or_path: './pkg/${WASM_HASHED}' })@g" \
        "$f"
done

echo "→ rewrote pkg references in $(ls "$WEB_DIR"/*.html | wc -l) HTML page(s)"
echo "✓ pkg bundle content-addressed"
