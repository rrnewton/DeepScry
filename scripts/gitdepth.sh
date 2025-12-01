#!/bin/bash
# Count commits in first-parent (main branch) history only
# This matches what users see in `git log --oneline --first-parent`
git rev-list --count --first-parent HEAD

