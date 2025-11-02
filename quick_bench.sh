#!/bin/bash
# Quick snapshot benchmark
set -e

echo "=== Quick Snapshot Format Comparison ==="
echo ""

# JSON test
echo "JSON format:"
time ./target/release/mtg tui decks/royal_assassin.dck --p1=heuristic --p2=heuristic --seed=12345 \
    --snapshot-output=/tmp/test.json --snapshot-format=json \
    --stop-on-choice=50 --verbosity=silent > /dev/null 2>&1
ls -lh /tmp/test.json | awk '{print "Size:", $5}'
echo ""

# Bincode test
echo "Bincode format:"
time ./target/release/mtg tui decks/royal_assassin.dck --p1=heuristic --p2=heuristic --seed=12345 \
    --snapshot-output=/tmp/test.bincode --snapshot-format=bincode \
    --stop-on-choice=50 --verbosity=silent > /dev/null 2>&1
ls -lh /tmp/test.bincode | awk '{print "Size:", $5}'
echo ""

echo "=== Loading Benchmark ==="
echo ""

echo "Loading JSON:"
time ./target/release/mtg resume /tmp/test.json --snapshot-format=json \
    --verbosity=silent --stop-on-choice=1 > /dev/null 2>&1
echo ""

echo "Loading Bincode:"
time ./target/release/mtg resume /tmp/test.bincode --snapshot-format=bincode \
    --verbosity=silent --stop-on-choice=1 > /dev/null 2>&1
