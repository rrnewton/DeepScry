#!/bin/bash
#
# Validation script with caching and atomic log writes
#
# This script runs comprehensive pre-commit validation including:
# - Code formatting checks
# - Linting with clippy
# - Unit tests
# - Examples
#
# Results are cached based on commit hash to avoid redundant validation.
# Smart caching: treats documentation-only changes (*.md) as cache hits.
#
# Usage:
#   ./validate.sh [OPTIONS]
#
# Options:
#   --force                    Skip cache checks and always run validation
#   --sequential               Run tests sequentially, failing on first failure (easier debugging)
#   --no-wip-commit            Completely suppress temporary WIP commit creation (fail if dirty)
#   --force-wip-commit         Force WIP commit creation even if submodules have changes
#   --validate-cmd <CMD>       Command to run for parallel validation (default: make validate-impl)
#   --validate-cmd-seq <CMD>   Command to run for sequential validation (default: make validate-impl-sequential)

set -e  # Exit on error
set -o pipefail  # Propagate pipeline errors

# Parse command line arguments
FORCE_VALIDATION=false
SEQUENTIAL_MODE=false
NO_WIP_COMMIT=false
FORCE_WIP_COMMIT=false
while [[ $# -gt 0 ]]; do
    case $1 in
        --force)
            FORCE_VALIDATION=true
            shift
            ;;
        --sequential)
            SEQUENTIAL_MODE=true
            shift
            ;;
        --no-wip-commit)
            NO_WIP_COMMIT=true
            shift
            ;;
        --force-wip-commit)
            FORCE_WIP_COMMIT=true
            shift
            ;;
        --validate-cmd)
            VALIDATE_CMD="$2"
            shift 2
            ;;
        --validate-cmd-seq)
            VALIDATE_CMD_SEQUENTIAL="$2"
            shift 2
            ;;
        *)
            echo "Unknown option: $1"
            echo "Usage: $0 [OPTIONS]"
            echo "Run '$0 --help' or see script header for full options"
            exit 1
            ;;
    esac
done

# Colors for output
GREEN='\033[0;32m'
RED='\033[0;31m'
YELLOW='\033[1;33m'
CYAN='\033[0;36m'
NC='\033[0m' # No Color

# Configuration
LOG_DIR="validate_logs"
LATEST_SYMLINK="$LOG_DIR/validate_latest.log"

# Default validation commands (can be overridden via CLI flags)
VALIDATE_CMD="make validate-impl"
VALIDATE_CMD_SEQUENTIAL="make validate-impl-sequential"

# Create log directory if it doesn't exist
mkdir -p "$LOG_DIR"

# Track whether we created a WIP commit (for cleanup)
CREATED_WIP_COMMIT=false

# Track whether caching should be disabled (dirty working copy with --no-wip-commit)
DISABLE_CACHING=false

# Cleanup function to uncommit WIP if needed
cleanup() {
    if [ "$CREATED_WIP_COMMIT" = true ]; then
        echo ""
        echo -e "${CYAN}Cleaning up temporary WIP commit...${NC}"
        git reset --soft HEAD~1
        echo -e "${CYAN}✓ Uncommitted WIP commit${NC}"
    fi
}

# Register cleanup to run on exit (success or failure)
trap cleanup EXIT

# Check if working copy is dirty (both regular changes and submodule changes)
# Refresh the index to avoid false positives from stale stat information
git update-index --refresh -q 2>/dev/null || true

# Check for regular working copy changes
HAS_REGULAR_CHANGES=false
if ! git diff-index --quiet HEAD -- 2>/dev/null; then
    HAS_REGULAR_CHANGES=true
fi

# Check for submodule changes (modified, untracked, etc.)
HAS_SUBMODULE_CHANGES=false
if git submodule status | grep -q '^+\|^-\|^U'; then
    HAS_SUBMODULE_CHANGES=true
fi

# Determine if we need to handle dirty working copy
if [ "$HAS_REGULAR_CHANGES" = true ] || [ "$HAS_SUBMODULE_CHANGES" = true ]; then
    # If --no-wip-commit is set, skip WIP commit creation but continue with validation
    if [ "$NO_WIP_COMMIT" = true ]; then
        echo ""
        echo -e "${YELLOW}Warning: Working copy has uncommitted changes (--no-wip-commit specified)${NC}"
        echo ""
        if [ "$HAS_REGULAR_CHANGES" = true ]; then
            echo "Regular changes detected in working copy."
        fi
        if [ "$HAS_SUBMODULE_CHANGES" = true ]; then
            echo "Submodule changes detected."
        fi
        echo ""
        echo "Proceeding with validation WITHOUT creating WIP commit."
        echo "Caching will be disabled for this run."
        echo ""
        # Skip WIP commit creation and continue to validation
        # Disable caching since working copy is dirty
        DISABLE_CACHING=true
    else
        # If submodule changes exist and --force-wip-commit is NOT set, bail
        if [ "$HAS_SUBMODULE_CHANGES" = true ] && [ "$FORCE_WIP_COMMIT" = false ]; then
            echo ""
            echo -e "${RED}✗ Error: Submodule changes detected${NC}"
            echo ""
            echo "The working copy has submodule changes, which typically should not"
            echo "be included in temporary WIP commits."
            echo ""
            echo "Submodule status:"
            git submodule status | grep '^+\|^-\|^U' || true
            echo ""
            echo "Please either:"
            echo "  1. Commit or stash the submodule changes before validation"
            echo "  2. Use --force-wip-commit to override this check (not recommended)"
            echo "  3. Use --no-wip-commit to skip WIP commit and run validation anyway"
            echo ""
            echo "Usage: $0 [OPTIONS]"
            echo ""
            exit 1
        fi

        # Create temporary WIP commit
        echo ""
        echo -e "${CYAN}Working copy is dirty - creating temporary WIP commit...${NC}"
        if [ "$HAS_SUBMODULE_CHANGES" = true ]; then
            echo -e "${YELLOW}Warning: Including submodule changes in WIP commit (--force-wip-commit specified)${NC}"
        fi
        git add -A
        git commit -m "wip" --no-verify
        CREATED_WIP_COMMIT=true
        echo -e "${CYAN}✓ Created temporary WIP commit${NC}"
        echo ""
    fi
fi

# Get current commit hash (after potential WIP commit)
COMMIT_HASH=$(git rev-parse HEAD 2>/dev/null || echo "unknown")

# Determine log file name
if [ "$CREATED_WIP_COMMIT" = true ]; then
    BASE_LOG_FILE="$LOG_DIR/validate_${COMMIT_HASH}_dirty.log"
    STATUS_LABEL="dirty"
else
    BASE_LOG_FILE="$LOG_DIR/validate_${COMMIT_HASH}.log"
    STATUS_LABEL="clean"
fi

# If --force is set and the log file already exists, find a unique name with counter suffix
LOG_FILE="$BASE_LOG_FILE"
if [ "$FORCE_VALIDATION" = true ] && [ -f "$LOG_FILE" ]; then
    COUNTER=2
    BASE_NAME="${BASE_LOG_FILE%.log}"
    while [ -f "${BASE_NAME}_${COUNTER}.log" ]; do
        COUNTER=$((COUNTER + 1))
    done
    LOG_FILE="${BASE_NAME}_${COUNTER}.log"
    echo ""
    echo -e "${YELLOW}--force specified: bypassing cache${NC}"
    echo -e "${YELLOW}Log file collision detected, using: $(basename "$LOG_FILE")${NC}"
    echo ""
fi

WIP_FILE="${LOG_FILE}.wip"

# Skip cache checks if --force is specified or caching is disabled
if [ "$FORCE_VALIDATION" = false ] && [ "$DISABLE_CACHING" = false ]; then
    # Simple cache hit: exact match for this commit
    if [ -f "$LOG_FILE" ]; then
        echo ""
        echo "==================================="
        echo -e "${GREEN}✓ Validation cache hit for commit ${COMMIT_HASH}${NC}"
        echo -e "${GREEN}✓ Validation already passed!${NC}"
        echo "==================================="
        echo ""
        echo "Log file: $LOG_FILE"
        echo ""
        exit 0
    fi
fi

# Smart cache hit: check if only *.md files changed compared to latest validation
# (also skipped if --force is specified or caching is disabled)
if [ "$FORCE_VALIDATION" = false ] && [ "$DISABLE_CACHING" = false ] && [ -L "$LATEST_SYMLINK" ]; then
    # Extract the hash from the latest symlink target
    LATEST_LOG=$(readlink "$LATEST_SYMLINK")
    # Extract hash from filename: validate_HASH[_dirty].log
    LATEST_HASH=$(echo "$LATEST_LOG" | sed -E 's/validate_([a-f0-9]+)(_dirty)?\.log/\1/')

    # Validate that we extracted a valid git hash (40 hex characters)
    if [ -n "$LATEST_HASH" ] && [[ "$LATEST_HASH" =~ ^[a-f0-9]{40}$ ]] && [ "$LATEST_HASH" != "$COMMIT_HASH" ]; then
        echo ""
        echo -e "${CYAN}Checking for documentation-only changes...${NC}"
        echo -e "${CYAN}Comparing: ${LATEST_HASH} → ${COMMIT_HASH}${NC}"

        # Verify both commits exist before comparing
        if ! git cat-file -e "$LATEST_HASH" 2>/dev/null; then
            echo -e "${YELLOW}Warning: Previous validation commit ${LATEST_HASH} not found - forcing validation${NC}"
            echo ""
        elif ! git cat-file -e "$COMMIT_HASH" 2>/dev/null; then
            echo -e "${YELLOW}Warning: Current commit ${COMMIT_HASH} not found - forcing validation${NC}"
            echo ""
        else
            # Get list of changed files (capture exit code separately)
            CHANGED_FILES=$(git diff --name-only "$LATEST_HASH" "$COMMIT_HASH" 2>/dev/null)
            DIFF_EXIT_CODE=$?

            if [ $DIFF_EXIT_CODE -ne 0 ]; then
                # git diff failed - don't trust the result, force validation
                echo -e "${YELLOW}Warning: git diff failed (exit code $DIFF_EXIT_CODE) - forcing validation${NC}"
                echo ""
            elif [ -z "$CHANGED_FILES" ]; then
                # No changes at all - perfect cache hit
                echo -e "${GREEN}✓ No changes detected - using cached validation${NC}"
                CACHE_HIT=true
            else
                # Check if all changed files are *.md
                NON_MD_FILES=$(echo "$CHANGED_FILES" | grep -v '\.md$' || true)

                if [ -z "$NON_MD_FILES" ]; then
                    # Only .md files changed
                    echo -e "${GREEN}✓ Only documentation files changed:${NC}"
                    echo "$CHANGED_FILES" | sed 's/^/  - /'
                    echo -e "${GREEN}✓ Using cached validation${NC}"
                    CACHE_HIT=true
                else
                    # Non-markdown files changed
                    echo -e "${YELLOW}Code changes detected - validation required${NC}"
                    echo "Changed files:"
                    echo "$CHANGED_FILES" | sed 's/^/  - /'
                    CACHE_HIT=false
                fi
            fi

            if [ "$CACHE_HIT" = true ]; then
                # Create symlink from old log to new log
                LATEST_LOG_PATH="$LOG_DIR/$LATEST_LOG"
                ln -s "$(basename "$LATEST_LOG")" "$LOG_FILE"

                # Update latest symlink to point to new hash
                rm -f "$LATEST_SYMLINK"
                ln -s "$(basename "$LOG_FILE")" "$LATEST_SYMLINK"

                echo ""
                echo "==================================="
                echo -e "${GREEN}✓ Smart cache hit!${NC}"
                echo "==================================="
                echo ""
                echo "Cached from: $LATEST_LOG_PATH"
                echo "Linked to:   $LOG_FILE"
                echo "Latest:      $LATEST_SYMLINK -> $(basename "$LOG_FILE")"
                echo ""
                exit 0
            fi
            echo ""
        fi
    fi
fi

# Run validation (not cached or code changes detected)
echo "==================================="
echo "Running validation"
echo "Commit: ${COMMIT_HASH} (${STATUS_LABEL})"
echo "Log file: ${LOG_FILE}"
echo "==================================="
echo ""

# Run validation using configured commands
run_validation() {
    if [ "$SEQUENTIAL_MODE" = true ]; then
        eval "$VALIDATE_CMD_SEQUENTIAL"
    else
        eval "$VALIDATE_CMD"
    fi
}

# Run validation and capture output to WIP file
# The output goes both to the file and to stdout (via tee)
if run_validation 2>&1 | tee "$WIP_FILE"; then
    # Validation succeeded - atomically move WIP to final log file
    mv "$WIP_FILE" "$LOG_FILE"

    # Update latest symlink (only for successful validations)
    rm -f "$LATEST_SYMLINK"
    ln -s "$(basename "$LOG_FILE")" "$LATEST_SYMLINK"

    echo ""
    echo "==================================="
    echo -e "${GREEN}✓ All validation checks passed!${NC}"
    echo "==================================="
    echo ""
    echo "Results cached to: $LOG_FILE"
    echo "Latest: $LATEST_SYMLINK -> $(basename "$LOG_FILE")"
    echo ""
    exit 0
else
    # Validation failed - remove WIP file (don't cache failures)
    rm -f "$WIP_FILE"

    echo ""
    echo "==================================="
    echo -e "${RED}✗ Validation failed!${NC}"
    echo "==================================="
    echo ""
    exit 1
fi
