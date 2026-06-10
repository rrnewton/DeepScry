#!/usr/bin/env node
/**
 * E2E for the login choices (mtg-742) and the Custom Deck URL loader.
 *
 * Hermetic: serves web/ over a local HTTP server and intercepts the auth +
 * deck-URL fetches so no real OAuth provider or external host is touched.
 *
 * Part 1 — Landing page sign-in choices (index.html):
 *   - With both providers configured (/auth/status), the GitHub + Google
 *     buttons appear, plus the always-present "Ephemeral account" section
 *     with its can't-save-to-cloud WARNING.
 *   - Clicking "Sign in with GitHub" navigates to /auth/login/github.
 *
 * Part 2 — Custom Deck URL (launcher.html):
 *   - A valid .dck URL loads, lands in Custom Decks, and is selected.
 *   - A CORS-blocked fetch shows a specific cross-origin error.
 *   - A non-deck body shows an invalid-.dck error.
 *
 * Usage: node web/test_login_and_deck_url.js
 */

'use strict';

const { chromium } = require('playwright');
const { spawn } = require('child_process');
const path = require('path');

const WEB_SRC = __dirname;

function log(msg) {
  const ts = new Date().toISOString().substring(11, 23);
  console.log(`[${ts}] ${msg}`);
}
const failures = [];
function check(cond, msg) {
  if (cond) log('  ✓ ' + msg);
  else {
    log('  ✗ FAIL: ' + msg);
    failures.push(msg);
  }
}

(async () => {
  let browser = null;
  let httpServer = null;
  const PORT = 19000 + Math.floor(Math.random() * 1000);
  try {
    httpServer = spawn('python3', ['-m', 'http.server', String(PORT)], {
      cwd: WEB_SRC,
      stdio: ['ignore', 'pipe', 'pipe'],
    });
    httpServer.stderr.on('data', () => {});
    await new Promise((r) => setTimeout(r, 1200));
    log('HTTP server on ' + PORT);

    browser = await chromium.launch({ headless: true, args: ['--no-sandbox'] });

    // ── Part 1: landing-page sign-in choices ───────────────────────────────
    {
      const ctx = await browser.newContext();
      const page = await ctx.newPage();
      page.on('pageerror', (e) => failures.push('pageerror(index): ' + e.message));
      // Both providers configured, not logged in.
      await page.route('**/auth/status', (route) =>
        route.fulfill({
          status: 200,
          contentType: 'application/json',
          body: JSON.stringify({
            providers: { github: true, google: true },
            oauth_enabled: true,
            logged_in: false,
            user_id: null,
          }),
        })
      );
      await page.goto(`http://127.0.0.1:${PORT}/index.html`);
      await page.waitForFunction(
        () => document.getElementById('oauth-choices') && document.getElementById('oauth-choices').style.display !== 'none',
        { timeout: 5000 }
      );
      const ghVisible = await page.isVisible('#btn-login-github');
      const ggVisible = await page.isVisible('#btn-login-google');
      check(ghVisible, 'GitHub sign-in button shown when provider configured');
      check(ggVisible, 'Google sign-in button shown when provider configured');
      const warnText = (await page.textContent('#ephemeral-warning')) || '';
      check(/cannot save decks to the cloud/i.test(warnText), 'Ephemeral warning states decks cannot be saved to the cloud');
      const hasEphemeralHeading = await page.isVisible('text=Ephemeral account');
      check(hasEphemeralHeading, '"Ephemeral account" option is present');

      // Clicking GitHub navigates to /auth/login/github.
      await page.route('**/auth/login/github', (route) => route.fulfill({ status: 200, body: 'redirect-stub' }));
      await Promise.all([
        page.waitForRequest('**/auth/login/github', { timeout: 5000 }),
        page.click('#btn-login-github'),
      ]);
      check(true, 'Clicking "Sign in with GitHub" hits /auth/login/github');
      await ctx.close();
    }

    // ── Part 2: Custom Deck URL loader (launcher.html) ─────────────────────
    {
      const ctx = await browser.newContext();
      const page = await ctx.newPage();
      page.on('pageerror', (e) => failures.push('pageerror(launcher): ' + e.message));
      // Avoid auth noise on the launcher page.
      await page.route('**/auth/status', (route) =>
        route.fulfill({ status: 200, contentType: 'application/json', body: JSON.stringify({ oauth_enabled: false }) })
      );
      // launcher.html needs the set index; stub a minimal one so it boots.
      await page.route('**/data/sets/index.json', (route) =>
        route.fulfill({ status: 200, contentType: 'application/json', body: JSON.stringify({ deck_names: [], deck_contents: {} }) })
      );

      const GOOD = 'https://raw.example.com/good.dck';
      const CORS = 'https://blocked.example.com/blocked.dck';
      const BAD = 'https://raw.example.com/notadeck.dck';
      await page.route(GOOD, (route) =>
        route.fulfill({
          status: 200,
          contentType: 'text/plain',
          body: '[metadata]\nName=URL Test Deck\n\n[Main]\n4 Lightning Bolt\n20 Mountain\n',
        })
      );
      await page.route(CORS, (route) => route.abort('failed')); // simulate CORS rejection
      await page.route(BAD, (route) => route.fulfill({ status: 200, contentType: 'text/plain', body: 'not a deck at all' }));

      await page.goto(`http://127.0.0.1:${PORT}/launcher.html`);
      await page.waitForSelector('#deck-url', { timeout: 5000 });

      // .dck format link points at the self-hosted explainer.
      const href = await page.getAttribute('a:has-text(".dck format")', 'href');
      check(href === 'docs/dck-format.html', '".dck format" links to the self-hosted explainer page');

      // Valid URL → loads into Custom Decks and is selected.
      await page.fill('#deck-url', GOOD);
      await page.click('#btn-load-url');
      await page.waitForFunction(
        () => /Loaded/.test(document.getElementById('deck-url-status').textContent),
        { timeout: 5000 }
      );
      const status1 = await page.textContent('#deck-url-status');
      check(/Loaded "URL Test Deck" \(24 cards\)/.test(status1), 'valid .dck URL loads 24 cards into Custom Decks');
      const collVal = await page.inputValue('#deck-collection');
      const deckVal = await page.inputValue('#deck-select');
      check(collVal === 'custom' && deckVal === 'URL Test Deck', 'loaded URL deck is selected in the picker');
      const stored = await page.evaluate(() => JSON.parse(localStorage.getItem('mtg-forge-custom-decks') || '{}'));
      check(stored['URL Test Deck'] && stored['URL Test Deck'].main_deck.length === 2, 'URL deck persisted under the shared custom-decks key');

      // CORS-blocked URL → specific cross-origin error.
      await page.fill('#deck-url', CORS);
      await page.click('#btn-load-url');
      await page.waitForFunction(
        () => /cross-origin/i.test(document.getElementById('deck-url-status').textContent),
        { timeout: 5000 }
      );
      const status2 = await page.textContent('#deck-url-status');
      check(/cross-origin/i.test(status2), 'CORS-blocked URL shows a specific cross-origin error');

      // Non-deck body → invalid .dck error.
      await page.fill('#deck-url', BAD);
      await page.click('#btn-load-url');
      await page.waitForFunction(
        () => /not a valid \.dck/i.test(document.getElementById('deck-url-status').textContent),
        { timeout: 5000 }
      );
      const status3 = await page.textContent('#deck-url-status');
      check(/not a valid \.dck/i.test(status3), 'non-deck body shows an invalid-.dck error');
      await ctx.close();
    }

    await browser.close();
    browser = null;
    if (httpServer) httpServer.kill();

    if (failures.length) {
      log(`FAILED: ${failures.length} check(s)`);
      process.exit(1);
    }
    log('ALL LOGIN + DECK-URL CHECKS PASSED');
    process.exit(0);
  } catch (err) {
    console.error('TEST ERROR: ' + (err && err.stack ? err.stack : err));
    if (browser) await browser.close().catch(() => {});
    if (httpServer) httpServer.kill();
    process.exit(1);
  }
})();
