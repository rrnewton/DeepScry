#!/usr/bin/env python3
"""
Network Fuzz Test - Find bugs by testing various configurations

This is a BUG FINDING script, NOT a regression test.
It runs for extended periods to discover new bugs through randomized testing.

Tests the network implementation with different:
- Controller types (heuristic, random, zero)
- Seeds
- Deck combinations
- Player orderings
- Native-to-native or WASM-to-native (--wasm mode)

Reports on exit (or Ctrl-C):
- Pass/fail rates per configuration
- Error categorization by last ERROR lines in logs
- Determinism testing (re-running failures)
- Reproducer commands for debugging

WASM mode (--wasm):
- Starts native server + native opponent client
- Starts Python web server for WASM files
- Uses Playwright to run browser-based WASM client
- Requires: npm install playwright (in web/ directory)
"""

import subprocess
import os
import sys
import tempfile
import shutil
import re
import signal
import time
import json
from dataclasses import dataclass, field
from typing import Optional, List, Dict, Tuple
from collections import defaultdict
from concurrent.futures import ThreadPoolExecutor, as_completed
import random

# Configuration
WORKSPACE_ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
MTG_BIN = os.path.join(WORKSPACE_ROOT, "target/release/mtg")

# Decks to test
DECKS = [
    os.path.join(WORKSPACE_ROOT, "decks/booster_draft/avatar/ryan_avatar_draft.dck"),
    os.path.join(WORKSPACE_ROOT, "decks/booster_draft/avatar/gabriel_avatar_draft.dck"),
]

# Controller types to test
CONTROLLERS = ["heuristic", "random", "zero"]

# Seeds to test
SEEDS = [1, 2, 3, 5, 7, 11, 13, 17, 42, 100]

# Global for graceful shutdown
shutdown_requested = False
results_collected: List = []
error_buckets_collected: Dict = defaultdict(list)

@dataclass
class TestConfig:
    """Test configuration"""
    seed: int
    controller_p1: str
    controller_p2: str
    deck1: str
    deck2: str
    seed_p1: int = 3
    seed_p2: int = 3

    def __str__(self):
        return f"seed={self.seed} p1={self.controller_p1} p2={self.controller_p2}"

    def reproducer_command(self) -> str:
        """Generate a single-script reproducer command for this configuration"""
        return f"./tests/network_vs_local_equivalence_e2e.sh {self.seed} {self.controller_p1} {self.controller_p2}"

@dataclass
class TestResult:
    """Result of a single test run"""
    config: TestConfig
    passed: bool
    duration: float
    error_signature: Optional[str] = None
    server_errors: List[str] = field(default_factory=list)
    client1_errors: List[str] = field(default_factory=list)
    client2_errors: List[str] = field(default_factory=list)
    output_dir: Optional[str] = None

def extract_error_signature(log_path: str) -> List[str]:
    """Extract last few ERROR lines from a log file"""
    errors = []
    if os.path.exists(log_path):
        with open(log_path, 'r') as f:
            for line in f:
                if 'ERROR' in line.upper() or 'PANIC' in line.upper():
                    # Clean ANSI codes and timestamps
                    clean = re.sub(r'\x1b\[[0-9;]*m', '', line)
                    clean = re.sub(r'^\[.*?\] ', '', clean)
                    errors.append(clean.strip())
    return errors[-3:] if errors else []

def make_error_signature(server_errors: List[str], client1_errors: List[str], client2_errors: List[str]) -> str:
    """Create a signature from errors for bucketing"""
    all_errors = server_errors + client1_errors + client2_errors
    if not all_errors:
        return "unknown"

    # Take the most specific error (usually the first one that caused the cascade)
    for error in all_errors:
        if 'unexpected OpponentChoice' in error:
            return "unexpected_opponent_choice"
        if 'action_count mismatch' in error:
            return "action_count_mismatch"
        if 'Connection reset' in error:
            return "connection_reset"
        if 'REVEAL VALIDATION FAILED' in error:
            return "reveal_validation"
        if 'Entity not found' in error:
            return "entity_not_found"
        if 'Creature must be on battlefield' in error:
            return "creature_not_on_battlefield"
        if 'Invalid game action' in error:
            return "invalid_game_action"
        if 'panic' in error.lower():
            return "panic"

    # Fallback: use first error line truncated
    return all_errors[0][:50] if all_errors else "unknown"

def run_test(config: TestConfig, timeout: int = 120) -> TestResult:
    """Run a single network test"""
    global shutdown_requested

    if shutdown_requested:
        return TestResult(
            config=config,
            passed=False,
            duration=0,
            error_signature="shutdown_requested"
        )

    start_time = time.time()

    # Create temp directory
    output_dir = tempfile.mkdtemp(prefix="network_fuzz_")

    # Random port
    port = random.randint(17800, 27800)

    # Paths
    server_log = os.path.join(output_dir, "server.log")
    client1_log = os.path.join(output_dir, "client1.log")
    client2_log = os.path.join(output_dir, "client2.log")

    try:
        # Start server
        server_proc = subprocess.Popen(
            [MTG_BIN, "server",
             "--port", str(port),
             "--seed", str(config.seed),
             "--network-debug",
             "--verbosity", "minimal",
             "--no-color-logs"],
            stdout=open(server_log, 'w'),
            stderr=subprocess.STDOUT,
            cwd=WORKSPACE_ROOT
        )

        # Wait for server to start
        time.sleep(1.5)

        if server_proc.poll() is not None:
            return TestResult(
                config=config,
                passed=False,
                duration=time.time() - start_time,
                error_signature="server_startup_failed",
                output_dir=output_dir
            )

        # Start client 1
        client1_proc = subprocess.Popen(
            [MTG_BIN, "connect",
             config.deck1,
             "--server", f"localhost:{port}",
             "--controller", config.controller_p1,
             "--seed-player", str(config.seed_p1),
             "--name", "Ryan"],
            stdout=open(client1_log, 'w'),
            stderr=subprocess.STDOUT,
            cwd=WORKSPACE_ROOT
        )

        time.sleep(0.5)

        # Start client 2
        client2_proc = subprocess.Popen(
            [MTG_BIN, "connect",
             config.deck2,
             "--server", f"localhost:{port}",
             "--controller", config.controller_p2,
             "--seed-player", str(config.seed_p2),
             "--name", "Gabriel"],
            stdout=open(client2_log, 'w'),
            stderr=subprocess.STDOUT,
            cwd=WORKSPACE_ROOT
        )

        # Wait for completion
        try:
            server_proc.wait(timeout=timeout)
            client1_proc.wait(timeout=5)
            client2_proc.wait(timeout=5)
        except subprocess.TimeoutExpired:
            server_proc.kill()
            client1_proc.kill()
            client2_proc.kill()
            return TestResult(
                config=config,
                passed=False,
                duration=time.time() - start_time,
                error_signature="timeout",
                output_dir=output_dir
            )

        duration = time.time() - start_time

        # Check results
        server_errors = extract_error_signature(server_log)
        client1_errors = extract_error_signature(client1_log)
        client2_errors = extract_error_signature(client2_log)

        # Determine if passed (no errors and server exited cleanly)
        passed = (server_proc.returncode == 0 and
                  not server_errors and
                  not client1_errors and
                  not client2_errors)

        error_sig = None if passed else make_error_signature(server_errors, client1_errors, client2_errors)

        return TestResult(
            config=config,
            passed=passed,
            duration=duration,
            error_signature=error_sig,
            server_errors=server_errors,
            client1_errors=client1_errors,
            client2_errors=client2_errors,
            output_dir=output_dir
        )

    except Exception as e:
        return TestResult(
            config=config,
            passed=False,
            duration=time.time() - start_time,
            error_signature=f"exception:{str(e)[:30]}",
            output_dir=output_dir
        )

def test_determinism(config: TestConfig, num_runs: int = 3, wasm_mode: bool = False) -> Tuple[int, int]:
    """Test if a configuration fails deterministically"""
    passes = 0
    fails = 0
    for _ in range(num_runs):
        if wasm_mode:
            result = run_wasm_test(config)
        else:
            result = run_test(config)
        if result.passed:
            passes += 1
        else:
            fails += 1
        # Clean up
        if result.output_dir and os.path.exists(result.output_dir):
            shutil.rmtree(result.output_dir, ignore_errors=True)
    return passes, fails


def run_wasm_test(config: TestConfig, timeout: int = 180) -> TestResult:
    """Run a single WASM network test using Playwright"""
    global shutdown_requested

    if shutdown_requested:
        return TestResult(
            config=config,
            passed=False,
            duration=0,
            error_signature="shutdown_requested"
        )

    start_time = time.time()

    # Create temp directory
    output_dir = tempfile.mkdtemp(prefix="wasm_fuzz_")

    # Random ports
    game_port = random.randint(17800, 27800)
    web_port = random.randint(28000, 38000)
    password = "fuzztest"

    # Paths
    server_log = os.path.join(output_dir, "server.log")
    client_log = os.path.join(output_dir, "native_client.log")
    browser_log = os.path.join(output_dir, "browser.log")
    playwright_script = os.path.join(output_dir, "test_script.js")

    server_proc = None
    client_proc = None
    web_proc = None

    try:
        # Start game server
        server_proc = subprocess.Popen(
            [MTG_BIN, "server",
             "--port", str(game_port),
             "--password", password,
             "--seed", str(config.seed),
             "--network-debug",
             "--verbosity", "minimal",
             "--no-color-logs"],
            stdout=open(server_log, 'w'),
            stderr=subprocess.STDOUT,
            cwd=WORKSPACE_ROOT
        )

        # Wait for server
        time.sleep(1.5)

        if server_proc.poll() is not None:
            return TestResult(
                config=config,
                passed=False,
                duration=time.time() - start_time,
                error_signature="server_startup_failed",
                output_dir=output_dir
            )

        # Start web server
        web_dir = os.path.join(WORKSPACE_ROOT, "web")
        web_proc = subprocess.Popen(
            ["python3", "-m", "http.server", str(web_port)],
            stdout=subprocess.DEVNULL,
            stderr=subprocess.DEVNULL,
            cwd=web_dir
        )
        time.sleep(1.0)

        # Start native client (opponent)
        client_proc = subprocess.Popen(
            [MTG_BIN, "connect",
             config.deck2,  # Native client uses deck2
             "--server", f"localhost:{game_port}",
             "--password", password,
             "--controller", config.controller_p2,
             "--seed-player", str(config.seed_p2),
             "--name", "NativeOpponent"],
            stdout=open(client_log, 'w'),
            stderr=subprocess.STDOUT,
            cwd=WORKSPACE_ROOT
        )

        time.sleep(1.0)

        # Create Playwright test script
        playwright_code = f'''
const {{ chromium }} = require('playwright');
const fs = require('fs');

const CONFIG = {{
    gamePort: {game_port},
    webPort: {web_port},
    password: "{password}",
    controller: "{config.controller_p1}",
    deck: "{os.path.basename(config.deck1).replace('.dck', '')}",
    timeout: {timeout * 1000},
    browserLog: "{browser_log}"
}};

async function waitForServer(port, maxAttempts = 30) {{
    const WebSocket = require('ws');
    for (let i = 0; i < maxAttempts; i++) {{
        try {{
            const ws = new WebSocket(`ws://localhost:${{port}}`);
            await new Promise((resolve, reject) => {{
                ws.on('open', () => {{ ws.close(); resolve(); }});
                ws.on('error', reject);
                setTimeout(() => reject(new Error('timeout')), 1000);
            }});
            return true;
        }} catch (e) {{
            await new Promise(r => setTimeout(r, 500));
        }}
    }}
    return false;
}}

async function runTest() {{
    const logs = [];
    const log = (msg) => {{
        const entry = `[${{new Date().toISOString()}}] ${{msg}}`;
        logs.push(entry);
        console.log(entry);
    }};

    let browser = null;
    try {{
        // Wait for game server
        log('Waiting for game server...');
        const ready = await waitForServer(CONFIG.gamePort);
        if (!ready) throw new Error('Game server not ready');
        log('Game server ready');

        // Launch browser
        log('Launching browser...');
        browser = await chromium.launch({{
            headless: true,
            args: ['--no-sandbox', '--enable-unsafe-swiftshader']
        }});
        const page = await browser.newPage();

        // Collect console messages
        page.on('console', msg => {{
            logs.push(`[Browser] ${{msg.text()}}`);
        }});

        page.on('pageerror', err => {{
            logs.push(`[PageError] ${{err.message}}`);
        }});

        // Navigate to fancy TUI
        log('Loading page...');
        await page.goto(`http://localhost:${{CONFIG.webPort}}/fancy.html`, {{
            waitUntil: 'networkidle',
            timeout: 30000
        }});

        // Wait for WASM init
        await page.waitForSelector('#launcher.show', {{ state: 'visible', timeout: 30000 }});
        log('WASM loaded');

        // Select network mode
        await page.selectOption('#game-mode', 'network');
        await page.evaluate(() => {{
            document.getElementById('game-mode').dispatchEvent(new Event('change', {{ bubbles: true }}));
        }});
        await new Promise(r => setTimeout(r, 500));

        // Select controller
        await page.selectOption('#p1-controller', CONFIG.controller);
        await new Promise(r => setTimeout(r, 500));

        // Fill network settings
        await page.fill('#server-url', `ws://localhost:${{CONFIG.gamePort}}`);
        await page.fill('#server-password', CONFIG.password);
        await page.fill('#player-name', 'WasmFuzz');

        // Select deck (first one that matches or first available)
        const deckOptions = await page.$$eval('#deck-select option', opts =>
            opts.map(o => ({{ value: o.value, text: o.textContent }}))
        );
        log(`Available decks: ${{deckOptions.length}}`);

        // Launch game
        log('Starting game...');
        await page.click('#btn-launch');

        // Wait for terminal
        try {{
            await page.waitForSelector('#ratzilla-terminal', {{ state: 'visible', timeout: 30000 }});
            log('Game terminal visible');
        }} catch (e) {{
            log('Terminal not visible after 30s');
        }}

        // Wait for game to complete
        let gameOver = false;
        let choiceCount = 0;
        const startWait = Date.now();
        const maxWait = CONFIG.timeout;

        while (!gameOver && (Date.now() - startWait) < maxWait) {{
            // Check for game over in logs
            for (const logLine of logs.slice(-50)) {{
                if (logLine.includes('"type":"game_ended"') ||
                    logLine.includes('Game Over') ||
                    logLine.includes('wins!')) {{
                    gameOver = true;
                    break;
                }}
                const match = logLine.match(/"choice_seq":(\\d+)/);
                if (match) choiceCount = Math.max(choiceCount, parseInt(match[1]));
            }}

            if (gameOver) break;

            // Check terminal
            const termText = await page.evaluate(() => {{
                const t = document.getElementById('ratzilla-terminal');
                return t ? t.textContent : '';
            }});
            if (termText.includes('Game Over') || termText.includes('wins!') || termText.includes('defeated')) {{
                gameOver = true;
                break;
            }}

            await new Promise(r => setTimeout(r, 2000));
        }}

        // Check for errors
        const hasDesync = logs.some(l =>
            l.includes('desync') || l.includes('DESYNC') ||
            l.includes('action_count mismatch')
        );

        const hasError = logs.some(l =>
            l.includes('[PageError]') || l.includes('PANIC') ||
            l.includes('panic')
        );

        // Result
        const result = {{
            gameOver,
            choiceCount,
            hasDesync,
            hasError,
            duration: (Date.now() - startWait) / 1000
        }};

        log(`Result: gameOver=${{gameOver}}, choices=${{choiceCount}}, desync=${{hasDesync}}, error=${{hasError}}`);

        // Write logs
        fs.writeFileSync(CONFIG.browserLog, logs.join('\\n'));

        // Exit code
        if (hasDesync || hasError) {{
            process.exit(2);  // Error
        }} else if (gameOver) {{
            process.exit(0);  // Success
        }} else if (choiceCount > 10) {{
            process.exit(0);  // Partial success (made progress)
        }} else {{
            process.exit(1);  // Timeout/no progress
        }}

    }} catch (err) {{
        log(`Error: ${{err.message}}`);
        fs.writeFileSync(CONFIG.browserLog, logs.join('\\n'));
        process.exit(2);
    }} finally {{
        if (browser) await browser.close();
    }}
}}

runTest();
'''

        # Write Playwright script
        with open(playwright_script, 'w') as f:
            f.write(playwright_code)

        # Run Playwright with NODE_PATH set to find modules in web/node_modules
        web_dir = os.path.join(WORKSPACE_ROOT, "web")
        env = os.environ.copy()
        env["NODE_PATH"] = os.path.join(web_dir, "node_modules")

        playwright_proc = subprocess.run(
            ["node", playwright_script],
            cwd=web_dir,
            timeout=timeout + 30,
            capture_output=True,
            text=True,
            env=env
        )

        duration = time.time() - start_time

        # Analyze results
        server_errors = extract_error_signature(server_log)
        client_errors = extract_error_signature(client_log)
        browser_errors = extract_error_signature(browser_log)

        # Check exit code
        if playwright_proc.returncode == 0:
            passed = True
            error_sig = None
        elif playwright_proc.returncode == 2:
            passed = False
            error_sig = make_error_signature(server_errors, client_errors, browser_errors)
            if not error_sig or error_sig == "unknown":
                error_sig = "wasm_error"
        else:
            passed = False
            error_sig = make_error_signature(server_errors, client_errors, browser_errors)
            if not error_sig or error_sig == "unknown":
                error_sig = "wasm_timeout_or_no_progress"

        return TestResult(
            config=config,
            passed=passed,
            duration=duration,
            error_signature=error_sig,
            server_errors=server_errors,
            client1_errors=browser_errors,
            client2_errors=client_errors,
            output_dir=output_dir
        )

    except subprocess.TimeoutExpired:
        return TestResult(
            config=config,
            passed=False,
            duration=time.time() - start_time,
            error_signature="wasm_timeout",
            output_dir=output_dir
        )
    except Exception as e:
        return TestResult(
            config=config,
            passed=False,
            duration=time.time() - start_time,
            error_signature=f"wasm_exception:{str(e)[:30]}",
            output_dir=output_dir
        )
    finally:
        # Cleanup processes
        if server_proc:
            server_proc.kill()
            server_proc.wait()
        if client_proc:
            client_proc.kill()
            client_proc.wait()
        if web_proc:
            web_proc.kill()
            web_proc.wait()

def generate_configs(num_configs: int = 50) -> List[TestConfig]:
    """Generate diverse test configurations"""
    configs = []

    # Test all controller combinations
    for c1 in CONTROLLERS:
        for c2 in CONTROLLERS:
            for seed in SEEDS[:5]:  # First 5 seeds for each combo
                configs.append(TestConfig(
                    seed=seed,
                    controller_p1=c1,
                    controller_p2=c2,
                    deck1=DECKS[0],
                    deck2=DECKS[1]
                ))

    # Add some random configs
    while len(configs) < num_configs:
        configs.append(TestConfig(
            seed=random.randint(1, 1000),
            controller_p1=random.choice(CONTROLLERS),
            controller_p2=random.choice(CONTROLLERS),
            deck1=random.choice(DECKS),
            deck2=random.choice(DECKS)
        ))

    return configs[:num_configs]

def print_summary(results: List[TestResult], error_buckets: Dict[str, List[TestResult]],
                  determinism_runs: int = 3, interrupted: bool = False, wasm_mode: bool = False):
    """Print the test summary"""
    print()
    if interrupted:
        print("=" * 50)
        print("INTERRUPTED - Printing summary of completed tests")
        print("=" * 50)
    print()

    mode_str = "WASM" if wasm_mode else "Native"

    # Summary
    if not results:
        print("No tests completed.")
        return

    passed = sum(1 for r in results if r.passed)
    failed = len(results) - passed

    print(f"=== Summary ({mode_str} mode) ===")
    print(f"Total:  {len(results)}")
    print(f"Passed: {passed} ({100*passed/len(results):.1f}%)")
    print(f"Failed: {failed} ({100*failed/len(results):.1f}%)")
    print()

    # Error breakdown
    if error_buckets:
        print("=== Error Categories ===")
        for error_sig, error_results in sorted(error_buckets.items(), key=lambda x: -len(x[1])):
            print(f"\n{error_sig}: {len(error_results)} occurrences")
            # Show example config
            ex = error_results[0]
            print(f"  Example: {ex.config}")
            if ex.server_errors:
                print(f"  Server: {ex.server_errors[0][:80]}...")
            if ex.client1_errors:
                label = "Browser" if wasm_mode else "Client1"
                print(f"  {label}: {ex.client1_errors[0][:80]}...")
            if ex.client2_errors:
                label = "NativeOpp" if wasm_mode else "Client2"
                print(f"  {label}: {ex.client2_errors[0][:80]}...")

        # Test determinism of failures (skip if interrupted)
        if not interrupted:
            print()
            print("=== Determinism Test ===")
            for error_sig, error_results in error_buckets.items():
                config = error_results[0].config
                passes, fails = test_determinism(config, determinism_runs, wasm_mode=wasm_mode)
                det = "DETERMINISTIC" if fails == determinism_runs else f"FLAKY ({passes}/{determinism_runs} pass)"
                print(f"{error_sig}: {det}")

    print()
    print("=== Controller Matrix ===")
    # Show pass rate by controller combination
    matrix = defaultdict(lambda: {"passed": 0, "total": 0})
    for r in results:
        key = f"{r.config.controller_p1} vs {r.config.controller_p2}"
        matrix[key]["total"] += 1
        if r.passed:
            matrix[key]["passed"] += 1

    for combo, stats in sorted(matrix.items()):
        pct = 100 * stats["passed"] / stats["total"] if stats["total"] > 0 else 0
        print(f"  {combo}: {stats['passed']}/{stats['total']} ({pct:.0f}%)")

    # Reproducer commands
    if error_buckets:
        print()
        print("=== Reproducer Commands ===")
        for error_sig, error_results in error_buckets.items():
            ex = error_results[0]
            print(f"\n--- {error_sig} ---")
            print(ex.config.reproducer_command())

    # Keep failure logs
    if error_buckets:
        print()
        print("=== Failure Logs ===")
        for error_sig, error_results in error_buckets.items():
            if error_results[0].output_dir and os.path.exists(error_results[0].output_dir):
                print(f"{error_sig}: {error_results[0].output_dir}")

def signal_handler(signum, frame):
    """Handle Ctrl-C gracefully"""
    global shutdown_requested
    if shutdown_requested:
        print("\nForce quit...")
        sys.exit(1)
    print("\n\nShutdown requested... waiting for current tests to complete...")
    shutdown_requested = True

def check_wasm_prerequisites():
    """Check that WASM prerequisites are available"""
    web_dir = os.path.join(WORKSPACE_ROOT, "web")

    # Check Playwright is installed
    try:
        result = subprocess.run(
            ["node", "-e", "require('playwright')"],
            cwd=web_dir,
            capture_output=True,
            timeout=10
        )
        if result.returncode != 0:
            print("ERROR: Playwright not installed in web/ directory")
            print("Run: cd web && npm install playwright")
            return False
    except Exception as e:
        print(f"ERROR: Cannot check Playwright: {e}")
        return False

    # Check ws module
    try:
        result = subprocess.run(
            ["node", "-e", "require('ws')"],
            cwd=web_dir,
            capture_output=True,
            timeout=10
        )
        if result.returncode != 0:
            print("ERROR: ws module not installed in web/ directory")
            print("Run: cd web && npm install ws")
            return False
    except Exception as e:
        print(f"ERROR: Cannot check ws module: {e}")
        return False

    # Check WASM files exist
    wasm_js = os.path.join(web_dir, "pkg/mtg_forge_rs.js")
    if not os.path.exists(wasm_js):
        print(f"ERROR: WASM not built: {wasm_js}")
        print("Run: make wasm-network")
        return False

    # Check for network_init export (wasm-network feature)
    with open(wasm_js, 'r') as f:
        if 'network_init' not in f.read():
            print("ERROR: WASM built without network feature")
            print("Run: make wasm-network")
            return False

    return True


def main():
    global shutdown_requested, results_collected, error_buckets_collected

    import argparse
    parser = argparse.ArgumentParser(
        description='Network fuzz tester - Bug finding through randomized testing',
        epilog='This is a bug finding tool, not a regression test. Use for discovering new bugs.'
    )
    parser.add_argument('--configs', type=int, default=30, help='Number of configs to test per batch')
    parser.add_argument('--parallel', type=int, default=3, help='Parallel test count')
    parser.add_argument('--determinism-runs', type=int, default=3, help='Runs for determinism test')
    parser.add_argument('--quick', action='store_true', help='Quick mode: fewer configs (10)')
    parser.add_argument('--infinite', action='store_true', help='Run forever until Ctrl-C')
    parser.add_argument('--timeout', type=int, default=120, help='Timeout per test in seconds')
    parser.add_argument('--wasm', action='store_true',
                        help='Test WASM client (browser) against native server/opponent')
    args = parser.parse_args()

    # Set up signal handler for graceful shutdown
    signal.signal(signal.SIGINT, signal_handler)
    signal.signal(signal.SIGTERM, signal_handler)

    if args.quick:
        args.configs = 10

    # WASM mode: force parallelism to 1 (port conflicts)
    if args.wasm:
        if args.parallel != 1:
            print("Note: WASM mode forces --parallel=1 (port conflicts)")
        args.parallel = 1
        args.timeout = max(args.timeout, 180)  # WASM needs more time

    mode_str = "WASM" if args.wasm else "Native"
    print(f"=== Network Fuzz Test ({mode_str} mode) ===")
    print(f"Binary: {MTG_BIN}")
    print(f"Configs per batch: {args.configs}")
    print(f"Parallel: {args.parallel}")
    print(f"Infinite mode: {args.infinite}")
    print(f"Press Ctrl-C to stop and see summary")
    print()

    # Check binary exists
    if not os.path.exists(MTG_BIN):
        print(f"ERROR: Binary not found: {MTG_BIN}")
        print("Run: cargo build --release --features network")
        sys.exit(1)

    # Check WASM prerequisites
    if args.wasm:
        if not check_wasm_prerequisites():
            sys.exit(1)
        print("WASM prerequisites OK")
        print()

    batch_num = 0
    while not shutdown_requested:
        batch_num += 1
        if args.infinite:
            print(f"\n=== Batch {batch_num} ===")

        # Generate configs
        configs = generate_configs(args.configs)
        if not args.infinite:
            print(f"Generated {len(configs)} test configurations")
        print()

        # Run tests
        print("Running tests...")

        # Select test function based on mode
        if args.wasm:
            test_fn = lambda cfg, to=args.timeout: run_wasm_test(cfg, to)
        else:
            test_fn = lambda cfg, to=args.timeout: run_test(cfg, to)

        with ThreadPoolExecutor(max_workers=args.parallel) as executor:
            futures = {executor.submit(test_fn, config): config for config in configs}

            for i, future in enumerate(as_completed(futures)):
                if shutdown_requested:
                    break

                result = future.result()
                results_collected.append(result)

                status = "PASS" if result.passed else f"FAIL ({result.error_signature})"
                print(f"  [{i+1}/{len(configs)}] {result.config}: {status} ({result.duration:.1f}s)")

                if not result.passed:
                    error_buckets_collected[result.error_signature].append(result)

                # Clean up passing tests
                if result.passed and result.output_dir and os.path.exists(result.output_dir):
                    shutil.rmtree(result.output_dir, ignore_errors=True)

        # Exit after one batch unless infinite mode
        if not args.infinite:
            break

    # Print summary
    print_summary(results_collected, error_buckets_collected, args.determinism_runs,
                  interrupted=shutdown_requested, wasm_mode=args.wasm)

    failed = sum(1 for r in results_collected if not r.passed)
    sys.exit(0 if failed == 0 else 1)

if __name__ == "__main__":
    main()
