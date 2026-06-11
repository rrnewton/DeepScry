#!/usr/bin/env node
/**
 * E2E for the login choices (mtg-742) and the Custom Deck URL loader.
 *
 * Hermetic: serves web/ over a local HTTP server and intercepts the auth +
 * deck-URL fetches so no real OAuth provider or external host is touched.
 *
 * Part 1 — Landing page sign-in choices (index.html), THREE auth states:
 *   1a (oauth disabled): no provider buttons, only the ephemeral path.
 *   1b (oauth enabled, logged OUT): GitHub + Google buttons visible + the
 *      always-present "Ephemeral account" section with its can't-save WARNING;
 *      clicking "Sign in with GitHub" navigates to /auth/login/github.
 *   1c (oauth enabled, logged IN): the logged-in view ("Signed in via GitHub
 *      as <name>") + a real "Log out" button; clicking it POSTs /auth/logout
 *      and returns the page to the signed-out provider-buttons state.
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

    // Helper: open index.html with a given /auth/status payload mocked.
    async function openIndexWith(statusBody) {
      const ctx = await browser.newContext();
      const page = await ctx.newPage();
      page.on('pageerror', (e) => failures.push('pageerror(index): ' + e.message));
      await page.route('**/auth/status', (route) =>
        route.fulfill({ status: 200, contentType: 'application/json', body: JSON.stringify(statusBody) })
      );
      await page.goto(`http://127.0.0.1:${PORT}/index.html`);
      return { ctx, page };
    }

    // ── Part 1a: oauth DISABLED → only ephemeral, no provider buttons ──────
    {
      const { ctx, page } = await openIndexWith({ oauth_enabled: false, logged_in: false });
      // Give the page a beat to (not) reveal the oauth choices.
      await page.waitForSelector('#username', { timeout: 5000 });
      await page.waitForTimeout(300);
      const choicesShown = await page.isVisible('#oauth-choices');
      const signedInShown = await page.isVisible('#oauth-signed-in');
      check(!choicesShown, 'oauth disabled → no GitHub/Google buttons (choices hidden)');
      check(!signedInShown, 'oauth disabled → no logged-in view');
      const hasEphemeral = await page.isVisible('text=Ephemeral account');
      check(hasEphemeral, 'oauth disabled → ephemeral account path still present');
      await ctx.close();
    }

    // ── Part 1b: oauth ENABLED, logged OUT → both buttons; GitHub navigates ─
    {
      const { ctx, page } = await openIndexWith({
        providers: { github: true, google: true },
        oauth_enabled: true,
        logged_in: false,
        provider: null,
        display_name: null,
        user_id: null,
      });
      await page.waitForFunction(
        () => document.getElementById('oauth-choices') && document.getElementById('oauth-choices').style.display !== 'none',
        { timeout: 5000 }
      );
      check(await page.isVisible('#btn-login-github'), 'GitHub sign-in button shown when provider configured');
      check(await page.isVisible('#btn-login-google'), 'Google sign-in button shown when provider configured');
      check(!(await page.isVisible('#oauth-signed-in')), 'logged-out → no logged-in view');
      check(!(await page.isVisible('#oauth-privacy-note')), 'logged-out → OAuth privacy note hidden (no identity to reassure about)');
      const warnText = (await page.textContent('#ephemeral-warning')) || '';
      check(/cannot save decks to the cloud/i.test(warnText), 'Ephemeral warning states decks cannot be saved to the cloud');
      check(await page.isVisible('text=Ephemeral account'), '"Ephemeral account" option is present');

      // Clicking GitHub navigates to /auth/login/github.
      await page.route('**/auth/login/github', (route) => route.fulfill({ status: 200, body: 'redirect-stub' }));
      await Promise.all([
        page.waitForRequest('**/auth/login/github', { timeout: 5000 }),
        page.click('#btn-login-github'),
      ]);
      check(true, 'Clicking "Sign in with GitHub" hits /auth/login/github');
      await ctx.close();
    }

    // ── Part 1c: oauth ENABLED, logged IN → logged-in view + Log out ───────
    {
      const { ctx, page } = await openIndexWith({
        providers: { github: true, google: true },
        oauth_enabled: true,
        logged_in: true,
        provider: 'github',
        display_name: 'octocat',
        suggested_name: 'octocat',
        user_id: 'github-99',
      });
      await page.waitForFunction(
        () => document.getElementById('oauth-signed-in') && document.getElementById('oauth-signed-in').style.display !== 'none',
        { timeout: 5000 }
      );
      check(!(await page.isVisible('#oauth-choices')), 'logged-in → provider buttons hidden');
      check(await page.isVisible('#btn-oauth-logout'), 'logged-in → Log out button visible');
      const signedInText = (await page.textContent('#oauth-signed-in')) || '';
      check(/GitHub/.test(signedInText), 'logged-in view names the provider (GitHub)');
      check(/octocat/.test(signedInText), 'logged-in view shows the display name (octocat)');
      // Username field is auto-populated from suggested_name, still editable.
      const prefill = await page.inputValue('#username');
      check(prefill === 'octocat', 'logged-in → username field pre-filled from suggested_name');
      const editable = await page.evaluate(() => !document.getElementById('username').readOnly && !document.getElementById('username').disabled);
      check(editable, 'logged-in → pre-filled username stays editable');
      check((await page.textContent('#btn-name')) === 'Continue to lobby', 'logged-in → continue button relabeled (not "ephemeral")');
      // Privacy reassurance is shown only on the OAuth path and covers the
      // three user-requested points.
      check(await page.isVisible('#oauth-privacy-note'), 'logged-in → OAuth privacy note shown by the username field');
      const privText = (await page.textContent('#oauth-privacy-note')) || '';
      check(/only the .*username you pick/i.test(privText.replace(/\s+/g, ' ')), 'privacy note: only the chosen username is public');
      check(/email and your GitHub\/Google identity are never revealed/i.test(privText.replace(/\s+/g, ' ')), 'privacy note: email + provider identity never revealed to other players');
      check(/save and load your deck collection/i.test(privText.replace(/\s+/g, ' ')), 'privacy note: sign-in identity is used to save your deck collection');
      const cloudId = await page.evaluate(() => sessionStorage.getItem('mtg.cloudIdentity'));
      check(cloudId === 'github-99', 'logged-in stamps mtg.cloudIdentity for cloud deck save');

      // Clicking Log out POSTs /auth/logout and returns to the signed-out state.
      let logoutMethod = null;
      await page.route('**/auth/logout', (route) => {
        logoutMethod = route.request().method();
        route.fulfill({ status: 200, body: 'logged out' });
      });
      await Promise.all([
        page.waitForRequest((r) => r.url().endsWith('/auth/logout'), { timeout: 5000 }),
        page.click('#btn-oauth-logout'),
      ]);
      check(logoutMethod === 'POST', 'Log out POSTs /auth/logout (not GET)');
      await page.waitForFunction(
        () => document.getElementById('oauth-choices') && document.getElementById('oauth-choices').style.display !== 'none',
        { timeout: 5000 }
      );
      check(await page.isVisible('#btn-login-github'), 'after Log out → provider buttons return (signed-out state)');
      check(!(await page.isVisible('#oauth-signed-in')), 'after Log out → logged-in view hidden');
      const cloudId2 = await page.evaluate(() => sessionStorage.getItem('mtg.cloudIdentity'));
      check(cloudId2 === null, 'after Log out → cloud identity cleared from sessionStorage');
      await ctx.close();
    }

    // ── Part 1d: lobby layout redesign (structural, no WS needed) ──────────
    {
      const { ctx, page } = await openIndexWith({ oauth_enabled: false, logged_in: false });
      // The lobby pane starts hidden (revealed after WS registration); these
      // are structural DOM checks, so wait for ATTACHMENT, not visibility.
      await page.waitForSelector('#create-form', { state: 'attached', timeout: 5000 });
      // Create-a-Game is a WIDE bar with game name + passcode side by side and
      // a small inline Create button.
      const createIsBar = await page.evaluate(() => document.getElementById('create-form').classList.contains('create-bar'));
      check(createIsBar, 'Create-a-Game uses the wide create-bar layout');
      check((await page.textContent('#btn-create')).trim() === 'Create', 'Create button is the small inline "Create"');
      // Refresh List lives inside the Open Games list header, not as a
      // standalone create-form control.
      const refreshInGamesHead = await page.evaluate(() => {
        const btn = document.getElementById('btn-refresh');
        const head = btn && btn.closest('.list-head');
        return !!(head && /Open Games/.test(head.textContent));
      });
      check(refreshInGamesHead, 'Refresh List button is part of the Open Games list header');
      // Games + Players are siblings in the same grid-2 (side by side).
      const sideBySide = await page.evaluate(() => {
        const games = document.getElementById('games-table');
        const players = document.getElementById('players-table');
        const gGrid = games.closest('.grid-2');
        const pGrid = players.closest('.grid-2');
        return !!(gGrid && gGrid === pGrid);
      });
      check(sideBySide, 'Open Games and Logged-in Players are side by side in one grid');
      // Both scroll panes are tall enough for ~8 dense rows (>= 280px min).
      const tallEnough = await page.evaluate(() => {
        const panes = document.querySelectorAll('#pane-lobby .games-scroll');
        return Array.from(panes).every((p) => {
          const min = parseInt(getComputedStyle(p).minHeight, 10) || 0;
          return min >= 280;
        });
      });
      check(tallEnough, 'list panes are tall enough to show ~8 rows (min-height >= 280px)');
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
