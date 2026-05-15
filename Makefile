# MTG Forge Rust - Development Makefile
#
# Quick reference for common development tasks
.PHONY: help build test validate clean run check fmt fmt-check clippy clippy-wasm doc docs examples full-benchmark bench-snapshot bench-logging coverage coverage-full validate-coverage-step validate-fmt-step profile callgrindprofile perfprofile heapprofile dhatprofile count setup-claude claude-github claude-beads happy code-dups bench wasm wasm-export wasm-serve wasm-dev play-web-local-dev wasm-test wasm-test-fancy wasm-test-fancy-dev wasm-test-human wasm-test-game-gui-rebuild wasm-test-game-gui-playtest wasm-e2e wasm-e2e-dev wasm-e2e-network wasm-e2e-network-human play-web play-web-pvp play-web-local build-network validate-network-e2e-step validate-impl-no-network validate-impl-sequential-no-network validate-parallel-steps-no-network

# Configuration variables
# NODE: Node.js binary (Playwright requires Node 18+)
# Auto-detect: prefer node18 wrapper, fall back to claude_code's bundled node, then system node
NODE := $(shell which node18 2>/dev/null || (test -x /usr/local/bin/claude_code/node && echo /usr/local/bin/claude_code/node) || which node 2>/dev/null)
# NPM: prefer the OS-managed /usr/bin/npm. Some Meta devservers ship a wrapper
# at /usr/local/bin/npm that prints a "direct installs not allowed" notice and
# exits 1, which breaks make-validate's `npm install --silent` calls. The
# real npm shipped with the OS nodejs package is at /usr/bin/npm.
NPM := $(shell test -x /usr/bin/npm && echo /usr/bin/npm || which npm 2>/dev/null)
# PORT: web server port (use: make PORT=7999 play-web-local-dev)
PORT ?= 8080
# SERVER_PORT: MTG game server port (use: make play-web SERVER_PORT=9999)
SERVER_PORT ?= 17771
# CONTROLLER: AI controller for play-web (random, heuristic, zero)
CONTROLLER ?= heuristic

# Default target - show available commands
help:
	@echo "MTG Forge Rust - Available Commands:"
	@echo ""
	@echo "  make build          - Build the project (cargo build)"
	@echo "  make test           - Run unit tests (cargo test)"
	@echo "  make validate       - Full pre-commit validation (tests + examples + lint)"
	@echo "  make examples       - Run all examples"
	@echo "  make full-benchmark - Run all performance benchmarks (slow)"
	@echo "  make bench-snapshot    - Run snapshot benchmark only"
	@echo "  make bench-logging     - Run stdout logging benchmark only"
	@echo "  make profile           - Profile game execution with flamegraph (CPU time)"
	@echo "  make callgrindprofile  - Profile with Valgrind Callgrind (works in containers)"
	@echo "  make perfprofile       - Profile with perf (requires host/privileges)"
	@echo "  make heapprofile       - Profile allocations with heaptrack"
	@echo "  make dhatprofile       - Profile allocations with dhat-rs (recommended)"
	@echo "  make coverage          - Run tests with coverage, generate HTML report"
	@echo "  make coverage-full     - Coverage for tests + examples (slower)"
	@echo "  make clean             - Clean build artifacts (cargo clean)"
	@echo "  make run            - Run the main binary (cargo run)"
	@echo "  make check          - Fast compilation check (cargo check)"
	@echo "  make fmt            - Format code (cargo fmt)"
	@echo "  make clippy         - Run linter (cargo clippy)"
	@echo "  make doc            - Generate documentation and open in browser"
	@echo "  make docs           - Generate documentation (no browser)"
	@echo "  make wasm           - Build WebAssembly module for browser"
	@echo "  make wasm-dev       - Build WASM (dev mode, fast)"
	@echo "  make play-web       - Play web GUI game vs AI (launches server + AI + web server)"
	@echo "                        Override: DECK=decks/foo.dck CONTROLLER=random PORT=8080"
	@echo "  make play-web-pvp   - Two-player PvP: two browser tabs connect to same server"
	@echo "  make play-web-local - Build WASM (network) and start local web server (no AI)"
	@echo "  make play-web-local-dev - Same as play-web-local but with dev build (fast)"
	@echo "  make wasm-serve     - Build WASM (non-network) and start local web server"
	@echo "  make wasm-test-fancy - Run Playwright e2e test with screenshots"
	@echo ""

# Build the project
build:
	@echo "=== Building project ==="
	cargo build

# Build release version
build-release:
	@echo "=== Building release ==="
	cargo build --release

# Build release binary with network feature (required for server/connect subcommands)
build-network:
	@echo "=== Building release with network support ==="
	cargo build --release --features network

# Run unit tests (including network tests)
# Note: human_input_e2e tests for WASM pattern don't require wasm feature
test:
	@echo "=== Running unit tests ==="
	cargo nextest run --features network

# Fast compilation check (no codegen)
check:
	@echo "=== Running cargo check ==="
	cargo check

# Format code
fmt:
	@echo "=== Formatting code ==="
	cargo fmt --all

# Check formatting without modifying files
fmt-check:
	@echo "=== Checking code formatting ==="
	cargo fmt --all -- --check

# Run clippy linter
# Note: mtg-benchmarks has mutually exclusive features, so we run it separately without --all-features
# Note: mtg-forge-rs network feature requires native deps, so we include it explicitly
# Note: wasm feature is mutually exclusive with native TUI, so we run it separately
clippy:
	@echo "=== Running clippy ==="
	cargo clippy -p mtg-forge-rs --all-targets --all-features --features network -- -D warnings
	cargo clippy -p mtg-forge-rs --all-targets --features wasm,network -- -D warnings
	cargo clippy -p mtg-benchmarks --all-targets -- -D warnings

# Run clippy on WASM target (catches WASM-specific code paths like #[cfg(target_arch = "wasm32")])
clippy-wasm:
	@echo "=== Running clippy on WASM target ==="
	cargo clippy -p mtg-forge-rs --target wasm32-unknown-unknown --no-default-features --features wasm-tui -- -D warnings

# Detect code duplication
code-dups:
	which jscpd || npm install -g jscpd
	jscpd mtg-engine/ mtg-benchmarks/ scripts/ --min-tokens=100
# pmd cpd --minimum-tokens=100 -d src -d tests -l rust
# pmd cpd --minimum-tokens=100 -d scripts -d tests -l python

count:
	@echo "=== Counting lines of code ==="
	cargo install cloc 2>/dev/null || true
	cloc src; cloc scripts; cloc tests

# Run all examples
examples:
	@echo "=== Running examples ==="
	@echo ""
	@./scripts/run_examples.sh

# Comprehensive pre-commit validation with caching
# Runs all tests, examples, and checks
# Caches results based on commit hash to avoid redundant validation
# Use: make validate ARGS=--force to skip cache
# Use: make validate ARGS=--sequential to run sequentially (fail on first error)
# Use: make validate ARGS="--force --sequential" to combine options
# See scripts/validate.sh for implementation details
validate:
	@./scripts/validate.sh $(ARGS)

# Internal target that actually runs validation
# This is called by scripts/validate.sh
# Runs validation steps in parallel using make -j
validate-impl:
	@echo "=== Starting parallel validation ==="
	@echo ""
	@$(MAKE) -j4 validate-parallel-steps
	@echo ""
	@echo "=== All validation steps completed ==="
	@echo ""

# Sequential validation - runs steps one at a time, fails on first error
# This is called by scripts/validate.sh when --sequential flag is used
validate-impl-sequential:
	@echo "=== Starting sequential validation ==="
	@echo ""
	@$(MAKE) validate-fmt-step
	@echo ""
	@$(MAKE) validate-clippy-step
	@echo ""
	@$(MAKE) validate-clippy-wasm-step
	@echo ""
	@$(MAKE) validate-test-step
	@echo ""
	@$(MAKE) validate-examples-step
	@echo ""
	@$(MAKE) validate-agentplay-step
	@echo ""
	@$(MAKE) validate-commander-step
	@echo ""
	@$(MAKE) validate-snapshot-resume-step
	@echo ""
	@$(MAKE) validate-wasm-step
	@echo ""
	@$(MAKE) validate-wasm-e2e-step
	@echo ""
	@$(MAKE) validate-network-e2e-step
	@echo ""
	@echo "=== All validation steps completed ==="
	@echo ""

# No-network variants - skip network E2E test for faster iteration
# Use: make validate ARGS=--no-network
validate-impl-no-network:
	@echo "=== Starting parallel validation (no network) ==="
	@echo ""
	@$(MAKE) -j4 validate-parallel-steps-no-network
	@echo ""
	@echo "=== All validation steps completed ==="
	@echo ""

validate-impl-sequential-no-network:
	@echo "=== Starting sequential validation (no network) ==="
	@echo ""
	@$(MAKE) validate-fmt-step
	@echo ""
	@$(MAKE) validate-clippy-step
	@echo ""
	@$(MAKE) validate-clippy-wasm-step
	@echo ""
	@$(MAKE) validate-test-step
	@echo ""
	@$(MAKE) validate-examples-step
	@echo ""
	@$(MAKE) validate-agentplay-step
	@echo ""
	@$(MAKE) validate-commander-step
	@echo ""
	@$(MAKE) validate-snapshot-resume-step
	@echo ""
	@$(MAKE) validate-wasm-step
	@echo ""
	@$(MAKE) validate-wasm-e2e-step
	@echo ""
	@echo "=== All validation steps completed ==="
	@echo ""

# Parallel validation steps - these will run concurrently when invoked with -j
# WASM build has separate dependencies so it runs in parallel with other steps
.PHONY: validate-parallel-steps validate-parallel-steps-no-network validate-impl-sequential validate-impl-sequential-no-network validate-fmt-step validate-clippy-step validate-clippy-wasm-step validate-test-step validate-examples-step validate-wasm-step validate-wasm-e2e-step validate-network-e2e-step validate-agentplay-step validate-commander-step validate-snapshot-resume-step
validate-parallel-steps: validate-fmt-step validate-clippy-step validate-clippy-wasm-step validate-test-step validate-examples-step validate-agentplay-step validate-commander-step validate-snapshot-resume-step validate-wasm-step validate-wasm-e2e-step validate-network-e2e-step deck_list
validate-parallel-steps-no-network: validate-fmt-step validate-clippy-step validate-clippy-wasm-step validate-test-step validate-examples-step validate-agentplay-step validate-commander-step validate-snapshot-resume-step validate-wasm-step validate-wasm-e2e-step deck_list

# Formatting check - matches the CI `fmt` job in .github/workflows/ci.yml.
# This must be wired into validate so that formatting drift is caught locally
# instead of turning CI red. CI uses nightly rustfmt; we invoke the default
# toolchain here, which has historically agreed with nightly for this repo.
validate-fmt-step:
	@$(MAKE) fmt-check
	@echo "✓ fmt-check completed"

validate-clippy-step:
	@$(MAKE) clippy
	@echo "✓ clippy completed"

validate-clippy-wasm-step:
	@$(MAKE) clippy-wasm
	@echo "✓ clippy-wasm completed"

validate-test-step:
	@$(MAKE) test
	@echo "✓ test completed"

validate-examples-step:
	@$(MAKE) examples
	@echo "✓ examples completed"

validate-agentplay-step:
	@echo "=== Running agentplay tests ==="
	@python3 -m pytest agentplay/ -v
	@python3 agentplay/agent_game.py --mock --seed 42 --max-turns 5 -- decks/simple_bolt.dck decks/simple_bolt.dck; \
		rc=$$?; if [ $$rc -ne 0 ] && [ $$rc -ne 2 ]; then exit $$rc; fi
	@echo "=== Running mode-equivalence orchestrator ==="
	@./scripts/test_mode_equivalence.sh
	@echo "✓ agentplay tests completed"

validate-commander-step:
	@echo "=== Running commander E2E test ==="
	@bash tests/commander_e2e.sh
	@echo "✓ commander E2E completed"

# Snapshot/resume determinism + smoke test for `mtg resume` subcommand.
# See tests/snapshot_resume_e2e.sh for what is covered.
validate-snapshot-resume-step:
	@echo "=== Running snapshot/resume E2E test ==="
	@bash tests/snapshot_resume_e2e.sh
	@echo "✓ snapshot/resume E2E completed"

validate-wasm-step:
	@$(MAKE) wasm-dev
	@echo "✓ wasm-dev build completed"

# WASM e2e tests run after wasm-dev build completes
# This step depends on validate-wasm-step finishing first
validate-wasm-e2e-step: validate-wasm-step
	@echo "=== Running WASM e2e tests ==="
	@cd web && $(NPM) install --silent 2>/dev/null
	@cd web && $(NODE) test_fancy_tui.js && $(NODE) test_human_input.js && $(NODE) test_click_and_log.js && $(NODE) test_font_size_layout.js && $(NODE) test_decouple_step3_launch_game_session.js && $(NODE) test_card_size_stability.js && $(NODE) test_decouple_step6_valid_choices.js && $(NODE) test_tapped_rotation.js && $(NODE) test_graveyard_overlay.js
	@echo "✓ wasm-e2e tests completed"

# Network E2E test: builds native server + WASM client, runs networked games
# Depends on build-network and wasm-network targets
# Runs: baseline single-deck test, multi-deck test (quick), and click+log test
validate-network-e2e-step:
	@echo "=== Building network components ==="
	@$(MAKE) build-network
	@$(MAKE) wasm-network
	@echo "=== Running Network E2E tests ==="
	@cd web && $(NPM) install --silent 2>/dev/null && npx playwright install chromium --with-deps 2>/dev/null || true
	@cd web && node test_network_gui_e2e.js
	@cd web && node test_network_multideck.js --quick
	@cd web && node test_network_click.js
	@echo "✓ network-e2e tests completed"

# Generate documentation and open in browser
doc:
	@echo "=== Generating documentation ==="
	cargo doc --no-deps --open

# Generate documentation without opening browser
docs:
	@echo "=== Generating documentation ==="
	cargo doc --no-deps

# Clean build artifacts
clean:
	@echo "=== Cleaning build artifacts ==="
	cargo clean

# Run the main binary
run:
	@echo "=== Running main binary ==="
	cargo run

# Run with release optimizations
run-release:
	@echo "=== Running release binary ==="
	cargo run --release

# Install development dependencies
setup: install-hooks
	@echo "=== Installing development tools ==="
	rustup component add rustfmt clippy
	rustup target add wasm32-unknown-unknown
	@if ! command -v wasm-pack >/dev/null 2>&1; then \
		echo "Installing wasm-pack..."; \
		cargo install wasm-pack; \
	fi

# Install tracked git hooks into .git/hooks/. Run once after cloning the repo.
# The pre-commit hook runs `cargo fmt --all -- --check` so we never push
# unformatted code that fails CI's `fmt` job.
.PHONY: install-hooks
install-hooks:
	@echo "=== Installing git hooks ==="
	@if [ ! -d .git ]; then \
		echo "Skipping: not a git working tree (no .git directory)"; \
		exit 0; \
	fi
	@for hook in scripts/git-hooks/*; do \
		name=$$(basename $$hook); \
		install -m 0755 "$$hook" ".git/hooks/$$name"; \
		echo "  installed: .git/hooks/$$name"; \
	done

# Show project info
info:
	@echo "Project: MTG Forge Rust"
	@echo "Rust version: $$(rustc --version)"
	@echo "Cargo version: $$(cargo --version)"
	@cargo tree --depth 1

# Benchmarking
# ==============================================================================

plot:
	python3 scripts/plot_performance_interactive.py

# Generate plots for all experiment_results/*/perf_history.csv files
# Skips symlinked directories to avoid redundant processing
plot-all:
	@for csv in experiment_results/*/perf_history.csv; do \
		if [ -f "$$csv" ]; then \
			dir=$$(dirname "$$csv"); \
			if [ -L "$$dir" ]; then \
				echo "=== Skipping symlink $$dir ==="; \
			else \
				echo "=== Generating plot for $$csv ==="; \
				python3 scripts/plot_performance_interactive.py \
					--input "$$csv" \
					--output "$$dir/performance_dashboard.html"; \
			fi \
		fi \
	done

# Run all performance benchmarks and record to CSV (takes a long time)
# This is the OFFICIAL benchmark entrypoint - always use this for tracked results
bench: full-benchmark
full-benchmark:
	@echo "=== Running all benchmarks (results recorded to CSV) ==="
	./scripts/run_benchmark.sh

# Quick benchmark runs (NOT recorded to CSV - for quick testing only)
bench-snapshot:
	@echo "=== Running snapshot benchmark (not recorded to CSV) ==="
	cargo bench --bench game_benchmark snapshot

bench-logging:
	@echo "=== Running stdout logging benchmark (not recorded to CSV) ==="
	cargo bench --bench game_benchmark stdout_logging

# Coverage
# ==============================================================================

# Run tests with coverage instrumentation and generate HTML report
# Requires: cargo install cargo-llvm-cov
# Output: experiment_results/coverage/html/index.html
coverage:
	@./scripts/run_coverage.sh

# Coverage for unit tests + examples (slower, more complete)
coverage-full:
	@./scripts/run_coverage.sh --full

# Standalone coverage step (not wired into validate by default - opt-in)
validate-coverage-step:
	@$(MAKE) coverage
	@echo "coverage completed"

# Profiling
# ==============================================================================

# Profile game execution with flamegraph (CPU time profiling)
# Requires cargo-flamegraph: cargo install flamegraph
profile:
	@echo "=== Profiling game execution with flamegraph (CPU time) ==="
	@echo "This will run 1000 games (seed 42) and generate a flamegraph"
	@echo "Output will be saved to experiment_results/flamegraph.svg"
	@echo ""
	@mkdir -p experiment_results
	@if ! command -v cargo-flamegraph >/dev/null 2>&1; then \
		echo "Error: cargo-flamegraph not found"; \
		echo "Install with: cargo install flamegraph"; \
		exit 1; \
	fi
	cargo flamegraph --bin mtg --output experiment_results/flamegraph.svg -- profile --games 1000 --seed 42
	@echo ""
	@echo "Flamegraph saved to: experiment_results/flamegraph.svg"
	@echo "Open with: firefox experiment_results/flamegraph.svg (or your browser of choice)"

# Profile with Valgrind Callgrind (CPU profiling that works in containers)
# Requires valgrind: apt-get install valgrind (or equivalent)
# This is the recommended CPU profiler for containerized environments
callgrindprofile: build-release
	@echo "=== Valgrind Callgrind CPU Profiling ==="
	@echo ""
	@echo "This profiles CPU instruction counts and call graphs using Callgrind."
	@echo "Callgrind works in containers without special permissions."
	@echo "The rewind_bench binary runs 250 games (reduced due to ~50x slowdown)."
	@echo ""
	@if ! command -v valgrind >/dev/null 2>&1; then \
		echo "Error: valgrind not found"; \
		echo "Install with:"; \
		echo "  Ubuntu/Debian: sudo apt-get install valgrind"; \
		echo "  Fedora: sudo dnf install valgrind"; \
		exit 1; \
	fi
	@mkdir -p experiment_results
	@echo "Running callgrind (this will take 1-2 minutes due to instrumentation overhead)..."
	@echo ""
	@# Run with callgrind, collecting instruction counts and call graphs
	valgrind --tool=callgrind \
		--callgrind-out-file=experiment_results/callgrind.out \
		--dump-instr=yes \
		--collect-jumps=yes \
		--cache-sim=yes \
		target/release/rewind_bench -n 250 -m sequential
	@echo ""
	@echo "=== Profiling complete! Analyzing results... ==="
	@echo ""
	@echo "=== Top 30 CPU Hotspots (by instruction count) ==="
	@echo ""
	@callgrind_annotate --auto=yes --inclusive=yes experiment_results/callgrind.out 2>&1 | head -100
	@echo ""
	@echo "=== Next Steps ==="
	@echo ""
	@echo "For function-level analysis:"
	@echo "  callgrind_annotate --auto=yes experiment_results/callgrind.out | less"
	@echo ""
	@echo "For source-level annotation of a specific file:"
	@echo "  callgrind_annotate --auto=yes experiment_results/callgrind.out mtg-engine/src/game/mana_engine.rs"
	@echo ""
	@echo "For interactive visualization (requires KCachegrind on host):"
	@echo "  kcachegrind experiment_results/callgrind.out"
	@echo ""
	@echo "For call graph analysis:"
	@echo "  callgrind_annotate --tree=both experiment_results/callgrind.out | less"
	@echo ""
	@echo "Data saved to: experiment_results/callgrind.out"

# Profile with Linux perf (CPU + cache performance)
# Requires perf: apt-get install linux-tools-common linux-tools-generic (or equivalent)
# May require elevated permissions. Run with sudo or adjust /proc/sys/kernel/perf_event_paranoid
# NOTE: Use 'make callgrindprofile' for containerized environments (no special permissions needed)
perfprofile: build-release
	@echo "=== Linux perf CPU + Cache Profiling ==="
	@echo ""
	@echo "This profiles CPU hotspots and cache behavior using Linux perf."
	@echo "The rewind_bench binary runs 5000 games to get statistically significant samples."
	@echo ""
	@if ! command -v perf >/dev/null 2>&1; then \
		echo "Error: perf not found"; \
		echo "Install with:"; \
		echo "  Ubuntu/Debian: sudo apt-get install linux-tools-common linux-tools-generic"; \
		echo "  Fedora: sudo dnf install perf"; \
		exit 1; \
	fi
	@mkdir -p experiment_results
	@echo "Attempting to run perf record..."
	@echo ""
	@# Run with call-graph recording
	@(cd experiment_results && perf record -F 997 -g --call-graph dwarf -o perf.data \
		-- ../target/release/rewind_bench -n 5000 -m sequential 2>&1 | tee perf_record.log) || \
	(echo ""; \
	 echo "=== perf profiling failed (likely permission/container issue) ==="; \
	 echo ""; \
	 echo "This is expected in containerized environments."; \
	 echo ""; \
	 echo "Workarounds:"; \
	 echo "  1. Run on host system (not in container)"; \
	 echo "  2. Use 'make profile' for flamegraph profiling instead"; \
	 echo "  3. Use 'make dhatprofile' for allocation profiling"; \
	 echo "  4. Run manually with perf stat (no recording):"; \
	 echo "     perf stat -d target/release/rewind_bench -n 1000 -m sequential"; \
	 echo ""; \
	 echo "For reference, here's what a successful perf profile shows:"; \
	 echo "  - Top CPU hotspots by function name"; \
	 echo "  - Call graph showing which functions call expensive operations"; \
	 echo "  - Cache miss rates (L1/L2/L3)"; \
	 echo "  - Instructions per cycle (IPC)"; \
	 echo ""; \
	 exit 1)
	@echo ""
	@echo "=== Profiling complete! Generating reports... ==="
	@echo ""
	@echo "=== Top 20 CPU Hotspots ==="
	@echo ""
	@(cd experiment_results && perf report --stdio --no-children -n --sort symbol --percent-limit 0.5 | head -50)
	@echo ""
	@echo "=== Next Steps ==="
	@echo ""
	@echo "For interactive analysis:"
	@echo "  cd experiment_results && perf report"
	@echo ""
	@echo "For detailed call graph:"
	@echo "  cd experiment_results && perf report --stdio -g --no-children"
	@echo ""
	@echo "For cache miss details:"
	@echo "  cd experiment_results && perf annotate --stdio"
	@echo ""
	@echo "Data saved to: experiment_results/perf.data"

# Profile allocations with heaptrack
# Requires cargo-heaptrack: cargo install cargo-heaptrack
# Also requires heaptrack: apt-get install heaptrack (or equivalent)
heapprofile:
	@echo "=== Profiling allocations with heaptrack ==="
	@echo "This will run 100 games (seed 42) and generate allocation profile"
	@echo "Output will be saved to experiment_results/heaptrack.profile.*.zst"
	@echo ""
	@mkdir -p experiment_results
	@if ! command -v cargo-heaptrack >/dev/null 2>&1; then \
		echo "Error: cargo-heaptrack not found"; \
		echo "Install with: cargo install cargo-heaptrack"; \
		echo ""; \
		echo "Also requires heaptrack system package:"; \
		echo "  Ubuntu/Debian: sudo apt-get install heaptrack"; \
		echo "  Fedora: sudo dnf install heaptrack"; \
		echo "  Arch: sudo pacman -S heaptrack"; \
		exit 1; \
	fi
	HEAPTRACK_OUTPUT=experiment_results cargo heaptrack --bin mtg --release -- profile --games 100 --seed 42
	@# Move heaptrack files to experiment_results if they were created in root
	@if ls heaptrack.mtg.*.gz 2>/dev/null; then \
		for file in heaptrack.mtg.*.gz; do \
			newname=$$(echo "$$file" | sed 's/heaptrack\.mtg\./heaptrack.profile./'); \
			mv "$$file" "experiment_results/$$newname"; \
		done; \
	fi
	@echo ""
	@echo "=== Profiling complete! Now analyzing results ==="
	@echo ""
	./scripts/analyze_heapprofile.sh
	@echo ""
	@echo "Analysis complete! Check output above for top allocation sites."

# Profile allocations with dhat-rs (Rust-native profiler with full symbol information)
# Generates dhat-heap.json which can be viewed with dh_view.html
# Automatically runs analysis and produces human-readable summary
dhatprofile:
	@echo "=== DHAT Allocation Profiling ==="
	@echo ""
	@echo "This profiles allocation hotspots in the game engine using dhat-rs."
	@echo "The rewind_bench binary runs 100 iterations of rewind+replay to isolate"
	@echo "forward gameplay allocations (excluding initialization overhead)."
	@echo ""
	@mkdir -p experiment_results
	@echo "Running profiler..."
	@cargo bench --bench dhat_profile --no-default-features
	@# Move dhat output to experiment_results
	@if [ -f dhat-heap.json ]; then \
		mv dhat-heap.json experiment_results/dhat-heap.json; \
		echo ""; \
		echo "=== Profiling complete! Analyzing results... ==="; \
		echo ""; \
		python3 scripts/analyze_dhat.py; \
		echo ""; \
		echo "=== Next Steps ==="; \
		echo ""; \
		echo "For interactive analysis:"; \
		echo "  1. Open https://nnethercote.github.io/dh_view/dh_view.html"; \
		echo "  2. Load experiment_results/dhat-heap.json"; \
		echo ""; \
		echo "To create a detailed analysis document:"; \
		echo "  python3 scripts/analyze_dhat.py > experiment_results/dhat_analysis_$$(date +%Y-%m-%d)_#$$(git rev-list --count HEAD).md"; \
	else \
		echo "Error: dhat-heap.json not found"; \
		exit 1; \
	fi

# ==============================================================================

deck_list: full_deck_list.txt
full_deck_list.txt:
	find decks/ forge-java/ -name "*.dck" -type f | sort > $@

# WebAssembly
# ==============================================================================

# Export card database and decks for WASM
# Set MTG_SKIP_WASM_EXPORT=1 to skip this step (useful when data already exists)
# Always run the current source tree's exporter. Reusing an existing release
# binary can silently generate stale decks.bin/cards.bin data that no longer
# matches the freshly built WASM loader.
wasm-export:
	@if [ "$$MTG_SKIP_WASM_EXPORT" = "1" ]; then \
		echo "=== Skipping WASM export (MTG_SKIP_WASM_EXPORT=1) ==="; \
	else \
		echo "=== Exporting card database using current sources ==="; \
		cargo run --bin mtg -- export-wasm; \
		echo "=== Export complete! ==="; \
	fi

# Build WebAssembly module for browser
wasm: wasm-export
	@echo "=== Building WebAssembly module ==="
	@if ! command -v wasm-pack >/dev/null 2>&1; then \
		echo "Installing wasm-pack..."; \
		cargo install wasm-pack; \
	fi
	@cd mtg-engine && wasm-pack build --target web --no-default-features --features wasm-tui
	@rm -rf web/pkg
	@cp -r mtg-engine/pkg web/pkg
	@echo ""
	@echo "=== WASM build complete! ==="
	@echo "Output: web/pkg/"
	@echo "To test: make wasm-serve"

# Web server log file location
WASM_SERVER_LOG := web/server.log

# Deck for the AI opponent in play-web (override with: make play-web DECK=decks/monored.dck)
DECK ?= decks/white_weenie.dck

# Play a web GUI game against a native AI opponent.
# Starts the MTG server, connects an AI client, and launches the web server.
# Usage: make play-web [DECK=decks/foo.dck] [PORT=8080] [SERVER_PORT=17771] [CONTROLLER=heuristic]
play-web: build-network wasm-network
	@./scripts/play-web.sh \
		--port $(PORT) \
		--server-port $(SERVER_PORT) \
		--controller $(CONTROLLER) \
		$(DECK)

# Two-player PvP: launches game server + web server, two browser tabs connect as players.
# Usage: make play-web-pvp [PORT=8080] [SERVER_PORT=17771]
play-web-pvp: build-network wasm-network
	@./scripts/play-web.sh \
		--port $(PORT) \
		--server-port $(SERVER_PORT) \
		--pvp

# Build WASM and start local web server
wasm-serve: wasm-network
	@echo ""
	@echo "=== Starting web server ==="
	@echo "Open http://localhost:$(PORT) in your browser"
	@echo "Log file: $(WASM_SERVER_LOG)"
	@echo "Press Ctrl+C to stop"
	@echo ""
	@cd web && python3 -m http.server $(PORT) 2>&1 | tee server.log

# Quick dev build - skips wasm-opt optimization for faster iteration
wasm-dev: wasm-export
	@echo "=== Building WebAssembly (dev mode - no optimization) ==="
	@if ! command -v wasm-pack >/dev/null 2>&1; then \
		echo "Installing wasm-pack..."; \
		cargo install wasm-pack; \
	fi
	@cd mtg-engine && wasm-pack build --dev --target web --no-default-features --features wasm-network
	@rm -rf web/pkg
	@cp -r mtg-engine/pkg web/pkg
	@echo ""
	@echo "=== WASM dev build complete! ==="

# Quick dev build and serve (local-only, no network/AI opponent)
play-web-local-dev: wasm-dev
	@echo ""
	@echo "=== Starting web server (dev build) ==="
	@echo "Open http://localhost:$(PORT) in your browser"
	@echo "Log file: $(WASM_SERVER_LOG)"
	@echo "Press Ctrl+C to stop"
	@echo ""
	@cd web && python3 -m http.server $(PORT) 2>&1 | tee server.log

# Build WASM with network feature (for browser multiplayer)
wasm-network: wasm-export
	@echo "=== Building WebAssembly with network feature ==="
	@if ! command -v wasm-pack >/dev/null 2>&1; then \
		echo "Installing wasm-pack..."; \
		cargo install wasm-pack; \
	fi
	@cd mtg-engine && wasm-pack build --dev --target web --no-default-features --features wasm-network
	@rm -rf web/pkg
	@cp -r mtg-engine/pkg web/pkg
	@echo ""
	@echo "=== WASM network build complete! ==="

# Build WASM with network feature and start web server (no AI opponent)
play-web-local: wasm-network
	@echo ""
	@echo "=== Starting web server (network build) ==="
	@echo "Open http://localhost:$(PORT)/fancy.html in your browser"
	@echo "Press Ctrl+C to stop"
	@echo ""
	@cd web && python3 -m http.server $(PORT) 2>&1 | tee server.log

# Test WASM module in headless browser (basic API test)
wasm-test: wasm
	@echo "=== Testing WASM in headless browser ==="
	@cd web && $(NPM) install --silent 2>/dev/null && $(NODE) test_wasm.js

# Test fancy TUI in browser with Playwright (e2e screenshot test)
# Launches game, steps through turns, takes screenshots, logs performance
wasm-test-fancy: wasm
	@echo "=== Testing Fancy TUI in browser (Playwright e2e) ==="
	@cd web && $(NPM) install --silent 2>/dev/null
	@cd web && $(NODE) test_fancy_tui.js
	@echo ""
	@echo "Screenshots saved in web/screenshots/"
	@echo "Test results: web/screenshots/test_results.json"

# Quick fancy TUI test using dev build (faster iteration)
wasm-test-fancy-dev: wasm-dev
	@echo "=== Testing Fancy TUI (dev build, Playwright e2e) ==="
	@cd web && $(NPM) install --silent 2>/dev/null
	@cd web && $(NODE) test_fancy_tui.js

# Test human input in browser with Playwright (e2e test)
# Tests human controller by pressing keys and verifying battlefield state
wasm-test-human: wasm-dev
	@echo "=== Testing Human Input (Playwright e2e) ==="
	@cd web && $(NPM) install --silent 2>/dev/null
	@cd web && $(NODE) test_human_input.js
	@echo ""
	@echo "Screenshots saved in web/screenshots/"
	@echo "Test results: web/screenshots/human_test_results.json"

# Test the rebuilt thin-DOM game.html in the browser (Playwright e2e).
# Validates the GuiViewModel migration: view-model shape, status bar text,
# player info bars, turn header logging, hand-sort consistency, sequential
# distinct card selection (the original same-name-collision bug),
# image-first card details, battlefield section labels, auto-run, and exit.
wasm-test-game-gui-rebuild: wasm-dev
	@echo "=== Testing rebuilt game.html (Playwright e2e) ==="
	@cd web && $(NPM) install --silent 2>/dev/null
	@cd web && $(NODE) test_game_gui_rebuild.js
	@echo ""
	@echo "Screenshots saved in web/screenshots/rebuild_*.png"
	@echo "Test results: web/screenshots/game_gui_rebuild_results.json"

# Agent-driven playtest: full games (≥10 turns each) across multiple seeds
# with periodic card click sampling. Reports per-game bug findings into
# web/screenshots/game_gui_playtest_results.json. Companion long-form
# verification for `wasm-test-game-gui-rebuild`.
wasm-test-game-gui-playtest: wasm-dev
	@echo "=== Playtesting rebuilt game.html (multi-game Playwright) ==="
	@cd web && $(NPM) install --silent 2>/dev/null
	@cd web && $(NODE) test_game_gui_playtest.js
	@echo ""
	@echo "Screenshots saved in web/screenshots/playtest_*.png"
	@echo "Test results: web/screenshots/game_gui_playtest_results.json"

# Run all WASM e2e tests (production build)
wasm-e2e: wasm
	@echo "=== Running all WASM e2e tests (production) ==="
	@cd web && $(NPM) install --silent 2>/dev/null
	@cd web && $(NODE) test_fancy_tui.js && $(NODE) test_human_input.js && $(NODE) test_click_and_log.js
	@echo ""
	@echo "All WASM e2e tests passed!"

# Run all WASM e2e tests (dev build for faster iteration)
wasm-e2e-dev: wasm-dev
	@echo "=== Running all WASM e2e tests (dev build) ==="
	@cd web && $(NPM) install --silent 2>/dev/null
	@cd web && $(NODE) test_fancy_tui.js && $(NODE) test_human_input.js && $(NODE) test_click_and_log.js
	@echo ""
	@echo "All WASM e2e tests passed!"

# Run WASM Network GUI E2E test (random controller auto-play)
# NOT part of 'make validate' - requires full network build
wasm-e2e-network: build-network wasm-network
	@echo "=== Running WASM Network GUI E2E test ==="
	@cd web && $(NPM) install --silent 2>/dev/null && npx playwright install chromium --with-deps 2>/dev/null || true
	@cd web && node test_network_gui_e2e.js

# Run WASM Network GUI E2E test (human controller with Playwright key presses)
wasm-e2e-network-human: build-network wasm-network
	@echo "=== Running WASM Network Human E2E test ==="
	@cd web && $(NPM) install --silent 2>/dev/null && npx playwright install chromium --with-deps 2>/dev/null || true
	@cd web && node test_network_gui_e2e.js --human
