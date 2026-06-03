#!/usr/bin/env node
// Fail-fast check that Playwright + its chromium binary are provisioned in
// web/node_modules. Extracted from the inline `node -e "..."` that used to live
// in the Makefile's validate-network-e2e-step (mtg-716, mtg-717) so the step
// can be wrapped by scripts/validate_step.sh without shell-quoting gymnastics.
//
// mtg-716 policy: chromium is provisioned ONCE by `make setup` (binary only, no
// `--with-deps`/root). validate must NOT fetch a browser at runtime — that
// breaks hermeticity. So we verify presence via the Playwright API (a
// structured check, not a string grep) and FAIL FAST with an actionable message
// if it is missing, instead of cascading into a confusing
// "Target page/context/browser has been closed".
const fs = require('fs');

let execPath;
try {
    execPath = require('playwright').chromium.executablePath();
} catch (e) {
    console.error('\nERROR: playwright is not installed in web/node_modules.\n' +
        'Run: make setup   (or: cd web && npm install && npx playwright install chromium)\n');
    process.exit(1);
}

if (!fs.existsSync(execPath)) {
    console.error('\nERROR: Playwright chromium is not provisioned (' + execPath + ').\n' +
        'Run: make setup   (or: cd web && npx playwright install chromium)\n');
    process.exit(1);
}

console.log('Playwright chromium present: ' + execPath);
