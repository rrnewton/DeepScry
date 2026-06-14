#!/usr/bin/env node
// Reusable UI visual-review harness — see <RepoRoot>/CLAUDE.md "UI Development:
// mandatory visual review". It loads a page/widget in headless Chromium,
// captures a screenshot, AND collects console errors + failed requests (404s on
// images/assets) so a UI change can be (a) eyeballed for layout / hierarchy /
// overflow / broken-asset regressions and (b) gated on "no new console errors
// or missing assets". This is the reusable STARTING POINT; the specific checks
// (what to shoot, what "looks wrong" means) are per-change.
//
// Requires: a running server (e.g. `mtg server-web`) and node_modules/playwright
// (run from web/). Point Playwright at the real browser cache in worktrees:
//   PLAYWRIGHT_BROWSERS_PATH=/home/<user>/.cache/ms-playwright
//
// Usage:
//   PLAYWRIGHT_BROWSERS_PATH=~/.cache/ms-playwright node web/ui_review.js \
//     --url http://127.0.0.1:8080/ [--signin <name>] [--selector "text=Foo"] \
//     [--out debug/shot.png] [--full]
//
// Screenshots go to the gitignored debug/ dir (never commit screenshots).
// Exits non-zero if any console error or failed/404 asset request occurred, so
// it can also be used as a gate (e.g. in an e2e wrapper).
const { chromium } = require('playwright');
function arg(name, def) {
  const i = process.argv.indexOf('--' + name);
  if (i < 0) return def;
  const v = process.argv[i + 1];
  return (v && !v.startsWith('--')) ? v : true;
}
(async () => {
  const url = arg('url');
  if (!url) { console.error('--url required'); process.exit(2); }
  const signin = arg('signin');
  const selector = arg('selector');
  const out = arg('out', 'debug/ui_review.png');
  const full = arg('full', false);

  const browser = await chromium.launch();
  const page = await browser.newPage({ viewport: { width: 1440, height: 950 } });
  const consoleErrors = [];
  const failedRequests = [];
  page.on('console', m => { if (m.type() === 'error') consoleErrors.push(m.text()); });
  page.on('requestfailed', r => failedRequests.push(`${r.url()} (${r.failure() && r.failure().errorText})`));
  page.on('response', r => { if (r.status() >= 400) failedRequests.push(`${r.url()} [${r.status()}]`); });

  await page.goto(url, { waitUntil: 'domcontentloaded' });
  await page.waitForTimeout(1500);
  if (signin) {
    try {
      await page.locator('input[type="text"], input:not([type=checkbox]):not([type=radio])').first().fill(String(signin));
      await page.getByRole('button', { name: /continue|sign in|enter/i }).first().click();
      await page.waitForTimeout(2500);
    } catch (e) { console.log('SIGNIN_ERR ' + e.message); }
  }
  let target = page;
  if (selector) {
    try { const el = page.locator(String(selector)).first(); await el.scrollIntoViewIfNeeded(); target = el; }
    catch (e) { console.log('SELECTOR_ERR ' + e.message); }
  }
  await target.screenshot({ path: out, fullPage: !!full && target === page });
  console.log('SCREENSHOT ' + out);
  console.log(`CONSOLE_ERRORS ${consoleErrors.length} ` + JSON.stringify(consoleErrors.slice(0, 12)));
  console.log(`FAILED_REQUESTS ${failedRequests.length} ` + JSON.stringify(failedRequests.slice(0, 20)));
  await browser.close();
  process.exit((consoleErrors.length || failedRequests.length) ? 1 : 0);
})();
