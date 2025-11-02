#!/bin/bash
# Benchmark snapshot serialization performance (JSON vs Bincode)

set -e

DECK="decks/royal_assassin.dck"
ITERATIONS=20

echo "=== Snapshot Serialization Performance Benchmark ==="
echo ""
echo "Running $ITERATIONS iterations of snapshot save/load cycles"
echo "Deck: $DECK"
echo ""

# Build release binary
echo "Building release binary..."
cargo build --release --quiet 2>&1 | grep -v "Compiling\|Finished" || true
echo ""

# Function to benchmark a format
benchmark_format() {
    local format=$1
    local snapshot_file="/tmp/bench_snapshot.$format"
    local total_create_time=0
    local total_load_time=0

    echo "Testing format: $format"
    echo ""

    for i in $(seq 1 $ITERATIONS); do
        rm -f "$snapshot_file"

        # Time snapshot creation
        create_start=$(date +%s.%N)
        ./target/release/mtg tui "$DECK" --p1=heuristic --p2=heuristic --seed=12345 \
            --snapshot-output="$snapshot_file" --snapshot-format="$format" \
            --stop-on-choice=20 --verbosity=silent > /dev/null 2>&1
        create_end=$(date +%s.%N)
        create_time=$(echo "$create_end - $create_start" | bc)
        total_create_time=$(echo "$total_create_time + $create_time" | bc)

        # Time snapshot loading (using resume command)
        load_start=$(date +%s.%N)
        ./target/release/mtg resume "$snapshot_file" --snapshot-format="$format" \
            --verbosity=silent --stop-on-choice=1 > /dev/null 2>&1
        load_end=$(date +%s.%N)
        load_time=$(echo "$load_end - $load_start" | bc)
        total_load_time=$(echo "$total_load_time + $load_time" | bc)

        printf "  Run %2d: create %.3fs, load %.3fs\n" $i $create_time $load_time
    done

    avg_create=$(echo "scale=4; $total_create_time / $ITERATIONS" | bc)
    avg_load=$(echo "scale=4; $total_load_time / $ITERATIONS" | bc)
    avg_total=$(echo "scale=4; $avg_create + $avg_load" | bc)

    # Get final file size
    if [ -f "$snapshot_file" ]; then
        size=$(stat -c%s "$snapshot_file" 2>/dev/null || stat -f%z "$snapshot_file" 2>/dev/null)
        size_kb=$(echo "scale=1; $size / 1024" | bc)
    fi

    echo ""
    echo "  Average create time: ${avg_create}s"
    echo "  Average load time:   ${avg_load}s"
    echo "  Average total time:  ${avg_total}s"
    echo "  Snapshot size:       ${size_kb} KB"
    echo ""

    rm -f "$snapshot_file"
}

# Benchmark JSON
benchmark_format "json"

# Benchmark Bincode
benchmark_format "bincode"

echo "=== Benchmark Complete ==="
