# MTG Forge Rust - Development Makefile
#
# Quick reference for common development tasks
.PHONY: help build test validate clean run check fmt clippy doc docs examples full-benchmark bench-snapshot bench-logging profile callgrindprofile perfprofile heapprofile dhatprofile count setup-claude claude-github claude-beads happy code-dups

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
	@echo "  make clean             - Clean build artifacts (cargo clean)"
	@echo "  make run            - Run the main binary (cargo run)"
	@echo "  make check          - Fast compilation check (cargo check)"
	@echo "  make fmt            - Format code (cargo fmt)"
	@echo "  make clippy         - Run linter (cargo clippy)"
	@echo "  make doc            - Generate documentation and open in browser"
	@echo "  make docs           - Generate documentation (no browser)"
	@echo ""

# Build the project
build:
	@echo "=== Building project ==="
	cargo build

# Build release version
build-release:
	@echo "=== Building release ==="
	cargo build --release

# Run unit tests
test:
	@echo "=== Running unit tests ==="
	cargo nextest run

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
clippy:
	@echo "=== Running clippy ==="
	cargo clippy -p mtg-forge-rs --all-targets --all-features -- -D warnings
	cargo clippy -p mtg-benchmarks --all-targets -- -D warnings

# Detect code duplication
code-dups:
	which jscpd || npm install -g jscpd
	jscpd src/ tests/ scripts/ --min-tokens=100
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
	@$(MAKE) validate-fmt-check-step
	@echo ""
	@$(MAKE) validate-clippy-step
	@echo ""
	@$(MAKE) validate-test-step
	@echo ""
	@$(MAKE) validate-examples-step
	@echo ""
	@echo "=== All validation steps completed ==="
	@echo ""

# Parallel validation steps - these will run concurrently when invoked with -j
.PHONY: validate-parallel-steps validate-impl-sequential validate-fmt-check-step validate-clippy-step validate-test-step validate-examples-step
validate-parallel-steps: validate-fmt-check-step validate-clippy-step validate-test-step validate-examples-step deck_list

validate-fmt-check-step:
	@$(MAKE) fmt-check
	@echo "✓ fmt-check completed"

validate-clippy-step:
	@$(MAKE) clippy
	@echo "✓ clippy completed"

validate-test-step:
	@$(MAKE) test
	@echo "✓ test completed"

validate-examples-step:
	@$(MAKE) examples
	@echo "✓ examples completed"

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
setup:
	@echo "=== Installing development tools ==="
	rustup component add rustfmt clippy

# Show project info
info:
	@echo "Project: MTG Forge Rust"
	@echo "Rust version: $$(rustc --version)"
	@echo "Cargo version: $$(cargo --version)"
	@cargo tree --depth 1

# Benchmarking
# ==============================================================================

plot:
	./scripts/plot_performance.py

# Run all performance benchmarks (takes a long time)
full-benchmark:
	@echo "=== Running all benchmarks ==="
	./scripts/run_benchmark.sh
#	cargo bench --bench game_benchmark

# Run snapshot benchmark only (fast)
bench-snapshot:
	@echo "=== Running snapshot benchmark ==="
	cargo bench --bench game_benchmark snapshot

# Run stdout logging benchmark only (fast)
bench-logging:
	@echo "=== Running stdout logging benchmark ==="
	cargo bench --bench game_benchmark stdout_logging

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
	@(cd experiment_results && sudo perf record -F 997 -g --call-graph dwarf \
		-- ../target/release/rewind_bench -n 5000 --sequential 2>&1 | tee perf_record.log) || \
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
	 echo "     perf stat -d target/release/rewind_bench -n 1000 --sequential"; \
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
	@(cd experiment_results && sudo perf report --stdio --no-children -n --sort symbol --percent-limit 0.5 | head -50)
	@echo ""
	@echo "=== Next Steps ==="
	@echo ""
	@echo "For interactive analysis:"
	@echo "  cd experiment_results && sudo perf report"
	@echo ""
	@echo "For detailed call graph:"
	@echo "  cd experiment_results && sudo perf report --stdio -g --no-children"
	@echo ""
	@echo "For cache miss details:"
	@echo "  cd experiment_results && sudo perf annotate --stdio"
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
	@if ls heaptrack.profile.* 2>/dev/null; then \
		mv heaptrack.profile.* experiment_results/ 2>/dev/null || true; \
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
	find decks/ forge-java/ -name "*.dck" | sort > $@
