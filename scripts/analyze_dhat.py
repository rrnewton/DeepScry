#!/usr/bin/env python3
"""
Analyze DHAT heap profiling results from dhat-heap.json

Usage:
    python3 scripts/analyze_dhat.py [dhat-heap.json]

Defaults to reading experiment_results/dhat-heap.json
"""

import json
import sys
from pathlib import Path


def format_bytes(num_bytes):
    """Format bytes in human-readable form"""
    for unit in ['B', 'KB', 'MB', 'GB']:
        if num_bytes < 1024.0:
            return f"{num_bytes:.2f} {unit}"
        num_bytes /= 1024.0
    return f"{num_bytes:.2f} TB"


def analyze_dhat_profile(json_path):
    """Analyze DHAT profile and print top allocation sites"""

    with open(json_path, 'r') as f:
        data = json.load(f)

    # Extract program points (allocation sites)
    pps = data.get('pps', [])
    ftbl = data.get('ftbl', [])

    # Sort by total bytes allocated
    pps_sorted = sorted(pps, key=lambda x: x.get('tb', 0), reverse=True)

    print("=" * 100)
    print("DHAT HEAP PROFILE ANALYSIS")
    print("=" * 100)
    print()

    # Summary statistics
    total_bytes = sum(pp.get('tb', 0) for pp in pps)
    total_blocks = sum(pp.get('tbk', 0) for pp in pps)

    print(f"Total allocations: {format_bytes(total_bytes)} in {total_blocks:,} blocks")
    print(f"Average block size: {total_bytes/total_blocks:.1f} bytes" if total_blocks > 0 else "N/A")
    print()

    # Peak memory at t-gmax
    if 'gmax' in data:
        print(f"Peak memory at t-gmax: {format_bytes(data['gmax'])}")

    # Memory at end
    if 'te' in data:
        print(f"Memory at t-end: {format_bytes(data['te'])}")
    print()

    print("=" * 100)
    print("TOP 20 ALLOCATION SITES BY TOTAL BYTES")
    print("=" * 100)
    print()

    for i, pp in enumerate(pps_sorted[:20], 1):
        tb = pp.get('tb', 0)
        tbk = pp.get('tbk', 0)
        avg_bytes = tb / tbk if tbk > 0 else 0
        percentage = 100.0 * tb / total_bytes if total_bytes > 0 else 0

        # Get the backtrace
        frames = pp.get('fs', [])

        print(f"#{i}: {format_bytes(tb)} ({percentage:.1f}%) in {tbk:,} blocks (avg {avg_bytes:.1f} bytes/block)")

        # Find first mtg_engine frame (most specific call site)
        mtg_frames = []
        other_frames = []

        for frame_idx in frames[:10]:  # Show first 10 frames
            if frame_idx < len(ftbl):
                frame = ftbl[frame_idx]
                if 'mtg_engine' in frame:
                    mtg_frames.append(frame)
                elif not mtg_frames:  # Only show other frames before first mtg frame
                    other_frames.append(frame)

        # Print our code frames (most important)
        if mtg_frames:
            print(f"  Location: {mtg_frames[0]}")
            for frame in mtg_frames[1:]:
                print(f"    ↳ {frame}")
        elif other_frames:
            # No mtg frames found, show allocator frames
            print(f"  (Allocator): {other_frames[0] if other_frames else 'unknown'}")

        print()

    print("=" * 100)
    print()
    print("For detailed interactive analysis:")
    print(f"  1. Open https://nnethercote.github.io/dh_view/dh_view.html")
    print(f"  2. Load {json_path}")
    print()


def main():
    # Determine input file
    if len(sys.argv) > 1:
        json_path = Path(sys.argv[1])
    else:
        json_path = Path("experiment_results/dhat-heap.json")

    if not json_path.exists():
        print(f"Error: {json_path} not found", file=sys.stderr)
        print(f"", file=sys.stderr)
        print(f"Run profiling first with: make dhatprofile", file=sys.stderr)
        sys.exit(1)

    try:
        analyze_dhat_profile(json_path)
    except json.JSONDecodeError as e:
        print(f"Error: Failed to parse JSON: {e}", file=sys.stderr)
        sys.exit(1)
    except Exception as e:
        print(f"Error: {e}", file=sys.stderr)
        sys.exit(1)


if __name__ == '__main__':
    main()
