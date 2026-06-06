#!/usr/bin/env node
/**
 * Regression test for mtg-747: an AURA attached to a LAND must (a1) render as
 * its own card in the battlefield (not be skipped) and (a2-consumed) surface as
 * an attachment badge on its host land.
 *
 * BUG: commit b2b38afa over-broadened the equipment-skip in renderBattlefield to
 * `if (card.attached_to != null) continue`, which ALSO swallowed attached auras
 * → the "Enchantments" section header rendered but its card was skipped (empty
 * section); and gui_view_model.rs only computed attachments for is_creature()
 * hosts, so an enchanted LAND never reported its aura.
 *
 * This test drives the REAL `renderBattlefield` (exposed on window.__mtg) with a
 * synthetic player — an aura attached to a land — and asserts the rendered DOM:
 *   - the aura tile RENDERS (the a1 fix: only EQUIPMENT attached to a host is
 *     skipped, not auras). Without the fix the aura tile would be absent.
 *   - the host land shows the attachment badge naming the aura (the GUI
 *     consuming the attachments the a2 fix now produces).
 * The Rust side (gui_view_model.rs PRODUCING the attachment for a land host) is
 * covered by the unit test `aura_on_land_is_surfaced_as_attachment`.
 */
const { chromium } = require('playwright');
const { spawn } = require('child_process');
const path = require('path');
const { getRandomPorts } = require('./test_network_utils');
const { firstBuiltinDeck, localGameUrl } = require('./game_boot_params');

const projectRoot = path.join(__dirname, '..');

function log(msg) {
    const ts = new Date().toISOString().substring(11, 23);
    console.log(`[${ts}] ${msg}`);
}

(async () => {
    let httpServer, browser;
    const { httpPort: HTTP_PORT } = await getRandomPorts();
    let failures = [];

    function check(name, ok, detail) {
        if (ok) {
            log(`PASS: ${name} — ${detail}`);
        } else {
            log(`FAIL: ${name} — ${detail}`);
            failures.push(`${name}: ${detail}`);
        }
    }

    try {
        httpServer = spawn('python3', ['-m', 'http.server', HTTP_PORT.toString()], {
            cwd: path.join(projectRoot, 'web'),
            stdio: ['ignore', 'pipe', 'pipe'],
        });
        await new Promise(r => setTimeout(r, 1500));

        browser = await chromium.launch({ headless: true, args: ['--no-sandbox'] });
        const page = await browser.newPage();
        await page.setViewportSize({ width: 1280, height: 720 });

        const browserErrors = [];
        page.on('pageerror', err => browserErrors.push(err.message));
        page.on('console', msg => {
            if (msg.type() === 'error') browserErrors.push(`console.error: ${msg.text()}`);
        });

        // Boot a game just to get window.__mtg + the battlefield DOM containers;
        // the assertion drives renderBattlefield with SYNTHETIC data so it is
        // deterministic and independent of the AI ever casting an aura.
        const base = `http://localhost:${HTTP_PORT}`;
        const deck = await firstBuiltinDeck(base);
        await page.goto(localGameUrl(base, 'native_game.html', {
            deck, p1: 'heuristic', p2: 'heuristic', seed: 42,
        }), { waitUntil: 'networkidle', timeout: 30000 });
        await page.waitForSelector('#game-area.show', { state: 'attached', timeout: 30000 });
        await page.waitForTimeout(1500);

        check('window.__mtg.renderBattlefield is exposed',
              await page.evaluate(() => typeof window.__mtg?.renderBattlefield === 'function'),
              'present');

        // Render a synthetic battlefield: an aura attached to a land. The
        // render + DOM read happen in ONE synchronous evaluate so the game's own
        // updateUI tick cannot overwrite the container mid-assertion.
        const result = await page.evaluate(() => {
            const aura = {
                card_id: 201, name: 'Friendly Neighborhood', mana_cost: 'G',
                type_line: 'Enchantment — Aura', category: 'enchantment',
                css_classes: ['card', 'enchantment'], is_tapped: false, summoning_sick: false,
                power: null, toughness: null, formatted_pt: null, colors: 'G',
                is_token: false, damage: 0, attached_to: 200, attachments: [],
                is_valid_choice: false, is_selected: false,
            };
            const land = {
                card_id: 200, name: 'Forest', mana_cost: '',
                type_line: 'Basic Land — Forest', category: 'land',
                css_classes: ['card', 'land'], is_tapped: false, summoning_sick: false,
                power: null, toughness: null, formatted_pt: null, colors: '',
                is_token: false, damage: 0, attached_to: null,
                attachments: [{ card_id: 201, name: 'Friendly Neighborhood', category: 'enchantment', css_classes: ['card', 'enchantment'] }],
                is_valid_choice: false, is_selected: false,
            };
            const player = {
                battlefield_sections: [
                    { label: 'Lands', category: 'land', cards: [land] },
                    { label: 'Enchantments', category: 'enchantment', cards: [aura] },
                ],
            };
            window.__mtg.renderBattlefield(player, 'player-field-cards', { card_height_px: 120 });

            const grid = document.getElementById('player-field-cards');
            const auraTile = grid.querySelector('.card[data-card-id="201"]');
            const landTile = grid.querySelector('.card[data-card-id="200"]');
            const badge = landTile ? landTile.querySelector('.card-equip-badge') : null;
            const sectionLabels = [...grid.querySelectorAll('.bf-section-label')].map(e => e.textContent);
            return {
                auraRendered: !!auraTile,
                auraName: auraTile ? (auraTile.querySelector('.card-name')?.textContent || '') : '',
                landRendered: !!landTile,
                badgeText: badge ? badge.textContent : null,
                landHasEquippedClass: landTile ? landTile.classList.contains('equipped') : false,
                sectionLabels,
            };
        });

        // (a1) the aura tile must render — NOT be skipped as if it were equipment.
        check('aura tile RENDERS on the battlefield (a1: auras no longer skipped)',
              result.auraRendered,
              `auraRendered=${result.auraRendered}, name="${result.auraName}"`);
        check('aura appears under an Enchantments section',
              result.sectionLabels.some(l => /Enchant/i.test(l)),
              `sectionLabels=${JSON.stringify(result.sectionLabels)}`);

        // (a2 consumed) the host land surfaces the attachment as a badge.
        check('host land shows the attachment badge naming the aura',
              !!result.badgeText && /Friendly Neighborhood/.test(result.badgeText),
              `badge=${JSON.stringify(result.badgeText)}`);
        check('host land is marked .equipped (has-attachment styling)',
              result.landHasEquippedClass && result.landRendered,
              `equipped=${result.landHasEquippedClass}, landRendered=${result.landRendered}`);

        const nonImage404 = browserErrors.filter(e =>
            !(e.includes('Failed to load resource') && e.includes('404'))
        );
        check('no non-image browser errors / WASM panics',
              nonImage404.length === 0,
              nonImage404.length === 0 ? `clean (${browserErrors.length} image-404s ignored)` : nonImage404.slice(0, 3).join(' | '));

    } finally {
        if (browser) await browser.close();
        if (httpServer) httpServer.kill();
    }

    if (failures.length === 0) {
        log('=== ALL TESTS PASSED ===');
        process.exit(0);
    } else {
        log(`=== FAILURES (${failures.length}) ===`);
        failures.forEach(f => log(`  - ${f}`));
        process.exit(1);
    }
})();
