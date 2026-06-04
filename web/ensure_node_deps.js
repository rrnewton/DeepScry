#!/usr/bin/env node
// Provision web/ node dependencies for the browser e2e suite — OFFLINE-FIRST,
// with NO silent skipping. (mtg-717 follow-on: unblock locked-down hosts where
// `npm install` is forbidden, WITHOUT introducing a coverage-hiding auto-skip.)
//
// Policy (mirrors CLAUDE.md "NEVER gracefully skip; hard-fail on missing
// prerequisites" + mtg-716 "validate must NOT fetch a browser at runtime"):
//
//   1. If the JS deps (playwright) are already requireable AND its chromium
//      binary is present → USE THEM AS-IS, no npm install. This is the offline
//      path: pre-stage web/node_modules + the playwright chromium cache once
//      (copy from a machine where provisioning succeeded) and the full e2e
//      suite RUNS with no network — full coverage, no skip.
//   2. Else, if playwright is not yet requireable, attempt `npm install` with
//      ALL output SURFACED (never swallowed — a locked-down host's
//      "direct installs not allowed" exit MUST be visible).
//   3. Re-verify. If deps are STILL unavailable, HARD-FAIL (exit 1) with an
//      actionable message. We NEVER auto-skip the tests: a silently-skipped
//      e2e looks identical to a passing one, so coverage holes hide. To run
//      validate deliberately WITHOUT the browser e2e, pass the explicit
//      `scripts/validate_run.py --no-wasm-e2e` (or `--no-network`) flag, which
//      disables them visibly and reports it in the run summary.
//
// NOTE: we do NOT run `npx playwright install chromium` here — fetching a
// browser binary at validate time breaks hermeticity (mtg-716). The chromium
// binary is provisioned once by `make setup`; if it is missing we hard-fail and
// tell the user to provision it, rather than silently downloading mid-run.
const { execFileSync } = require('child_process');
const fs = require('fs');

function playwrightRequireable() {
    try { require.resolve('playwright'); return true; } catch (e) { return false; }
}

function chromiumPresent() {
    try {
        const p = require('playwright').chromium.executablePath();
        return !!p && fs.existsSync(p);
    } catch (e) { return false; }
}

const npm = process.env.NPM || 'npm';

// 1. Offline path: everything already present → use it, no install.
if (playwrightRequireable() && chromiumPresent()) {
    console.log('node deps present (vendored/offline OK) — skipping npm install');
    process.exit(0);
}

// 2. JS deps missing → attempt an install, surfacing all output.
if (!playwrightRequireable()) {
    console.log(`node deps absent — running \`${npm} install\` (output shown; not swallowed)`);
    try {
        execFileSync(npm, ['install'], { stdio: 'inherit' });
    } catch (e) {
        console.error(`\n\`${npm} install\` failed (exit ${e.status != null ? e.status : '?'}).`);
        // fall through to the hard-fail below
    }
}

// 3. Re-verify and hard-fail with an actionable message if still unusable.
const haveJs = playwrightRequireable();
const haveChromium = haveJs && chromiumPresent();
if (haveJs && haveChromium) {
    console.log('node deps provisioned');
    process.exit(0);
}

const lines = ['', 'ERROR: browser e2e dependencies are NOT available (validate will NOT silently skip these tests).', ''];
if (!haveJs) {
    lines.push('  Missing: the playwright npm package (web/node_modules) — and `npm install` could not provide it (this host may forbid npm installs).');
} else {
    lines.push('  Missing: the playwright CHROMIUM browser binary. validate does NOT fetch browsers at runtime (hermeticity, mtg-716).');
}
lines.push('');
lines.push('  Pick ONE (no auto-skip path exists):');
lines.push('   (A) Provision OFFLINE (preferred — tests RUN, full coverage):');
lines.push('         pre-stage web/node_modules + the playwright chromium cache on this host');
lines.push('         (copy from a machine where `cd web && npm install && npx playwright install chromium` succeeded).');
lines.push('   (B) Provision ONLINE:  cd web && npm install && npx playwright install chromium   (or: make setup)');
lines.push('   (C) Deliberately DISABLE the browser e2e (explicit + reported in the run summary):');
lines.push('         scripts/validate_run.py --no-wasm-e2e      (and/or --no-network for the networked browser suite).');
lines.push('');
console.error(lines.join('\n'));
process.exit(1);
