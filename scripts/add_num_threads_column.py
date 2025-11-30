#!/usr/bin/env python3
"""
Add num_threads column to existing perf_history.csv files.

For parallel benchmarks (par_*, pinned_par_*), sets num_threads based on CPU:
- Threadripper 7975WX: 32 threads
- Ryzen 7 9800X3D: 8 threads

For sequential benchmarks, sets num_threads=1.
"""

import sys
import csv
from pathlib import Path


def determine_num_threads(benchmark_name, cpu_name):
    """Determine number of threads based on benchmark name and CPU."""
    # Check if it's a parallel benchmark
    if benchmark_name.startswith('par_') or benchmark_name.startswith('pinned_par_'):
        # Determine CPU-specific thread count
        if '7975WX' in cpu_name:
            return 32
        elif '9800X3D' in cpu_name or 'Ryzen_7' in cpu_name:
            return 8
        else:
            # Unknown CPU - default to 1 for safety
            print(f"Warning: Unknown CPU '{cpu_name}', defaulting to 1 thread for parallel benchmark", file=sys.stderr)
            return 1
    else:
        return 1


def update_csv(csv_path):
    """Add num_threads column to CSV file."""
    csv_path = Path(csv_path)

    if not csv_path.exists():
        print(f"File not found: {csv_path}", file=sys.stderr)
        return False

    # Determine CPU name from path
    if 'Threadripper' in str(csv_path) or '7975WX' in str(csv_path):
        cpu_name = 'AMD_Ryzen_Threadripper_PRO_7975WX_32-Cores'
    elif '9800X3D' in str(csv_path) or 'Ryzen_7' in str(csv_path):
        cpu_name = 'AMD_Ryzen_7_9800X3D_8-Core_Processor'
    else:
        cpu_name = 'Unknown'

    print(f"Processing {csv_path} (CPU: {cpu_name})...")

    # Read existing data
    rows = []
    with open(csv_path, 'r') as f:
        reader = csv.DictReader(f)
        original_fieldnames = reader.fieldnames

        # Check if num_threads already exists
        if 'num_threads' in original_fieldnames:
            print(f"  Column 'num_threads' already exists, skipping...")
            return True

        for row in reader:
            rows.append(row)

    if not rows:
        print(f"  No data rows found, skipping...")
        return True

    # Create new fieldnames with num_threads inserted after seed
    new_fieldnames = []
    for field in original_fieldnames:
        new_fieldnames.append(field)
        if field == 'seed':
            new_fieldnames.append('num_threads')

    # Add num_threads to each row
    updated_rows = []
    for row in rows:
        benchmark_name = row.get('benchmark_name', '')
        num_threads = determine_num_threads(benchmark_name, cpu_name)
        row['num_threads'] = str(num_threads)
        updated_rows.append(row)

    # Write updated CSV
    backup_path = csv_path.with_suffix('.csv.backup')
    print(f"  Creating backup: {backup_path}")
    csv_path.rename(backup_path)

    with open(csv_path, 'w', newline='') as f:
        writer = csv.DictWriter(f, fieldnames=new_fieldnames)
        writer.writeheader()
        writer.writerows(updated_rows)

    # Count parallel vs sequential
    par_count = sum(1 for r in updated_rows if r['benchmark_name'].startswith(('par_', 'pinned_par_')))
    seq_count = len(updated_rows) - par_count

    print(f"  ✓ Updated {len(updated_rows)} rows ({par_count} parallel, {seq_count} sequential)")
    return True


def main():
    # Find all perf_history.csv files
    csv_files = [
        'experiment_results/AMD_Ryzen_Threadripper_PRO_7975WX_32-Cores/perf_history.csv',
        'experiment_results/AMD_Ryzen_7_9800X3D_8-Core_Processor/perf_history.csv',
    ]

    success_count = 0
    for csv_file in csv_files:
        if update_csv(csv_file):
            success_count += 1

    print(f"\nCompleted: {success_count}/{len(csv_files)} files updated")

    if success_count == len(csv_files):
        print("\n✓ All CSV files updated successfully!")
        return 0
    else:
        print(f"\n⚠ {len(csv_files) - success_count} file(s) failed to update", file=sys.stderr)
        return 1


if __name__ == '__main__':
    sys.exit(main())
