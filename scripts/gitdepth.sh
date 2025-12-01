#!/bin/bash
# Count total commits (inclusive of all history)
# Use --main-only flag to count only first-parent (main branch) commits

if [ "$1" = "--main-only" ]; then
    # Count only main-branch commits (linear history)
    git rev-list --count --first-parent HEAD
else
    # Count all commits (default)
    git rev-list --count HEAD
fi

