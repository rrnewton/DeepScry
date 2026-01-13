# CI Integration Agent

You are performing a continuous integration task. We are using multiple branches:
- **stable branch**: `main`
- **integration branch**: `integration`
- **current feature branch(es)**: Check with `git branch -r | grep -v HEAD | grep origin/` for active feature branches

## Your Job

1. **Monitor CI** using `gh run list` to check the status of all feature branches
2. **Pull GREEN feature branches** into `integration` - only integrate branches that have passed CI
3. **Fix merge conflicts** using semantic understanding of what the commits on the feature branches were trying to accomplish
4. **Validate locally** by running `make validate` on the integration branch
5. **Push integration** and verify CI passes on the integration branch
6. **Push to main** once integration is green, catching main up to the integrated changes
7. **Archive completed branches** as tags (e.g., `branchname.v1`) per project conventions

## Key Commands

```bash
# Check CI status for all branches
gh run list --limit 20

# Check specific branch CI
gh run list --branch <branchname> --limit 3

# See commits ahead of main
git log --oneline origin/main..origin/<branchname>

# Cherry-pick or merge commits
git cherry-pick <commit>
git merge <branch> --no-edit

# Archive completed feature branch
git tag -a <branch>.v1 origin/<branch> -m "Archive <branch> after merging to main"
git push origin <branch>.v1
```

## Workflow

1. `git fetch origin` - Get latest state
2. Check CI status of all feature branches
3. Wait for any in-progress CI to complete
4. Checkout integration: `git checkout integration && git pull origin integration`
5. Cherry-pick/merge GREEN feature branch commits
6. Run `make validate` locally
7. Push integration and wait for CI to pass
8. Merge integration to main: `git checkout main && git merge integration --no-edit`
9. Push main and archive feature branches as tags

## Important Notes

- Only integrate branches that are GREEN in CI
- Use `--no-edit` for merges to use default commit messages
- Fast-forward merges are preferred when possible
- Always run local validation before pushing
- Archive completed feature branches as tags per CLAUDE.md conventions
