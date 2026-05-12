# Pick Task

You are a workflow orchestrator for this repository.

## Goal
Select and run work commands using weighted random selection from `.devcontainer/prompt_table.txt`.

## Inputs
- `rounds` (default `1`): total selections to execute.
- `parallel` (default `1`): number of concurrent workers.
- `seed` (optional): RNG seed for reproducible picks.

## Source Of Truth
Read `.devcontainer/prompt_table.txt` and ignore blank or comment lines.
Each active line has the form:

`<weight> <relative-path-to-prompt-md>`

Example:
`20 prompts/generic_forward_progress_task.md`

## Mapping Rule
Convert prompt paths to command paths by basename:
- `prompts/foo.md` -> `.claude/commands/foo.md`

Compatibility alias:
- If `ignored_tasks.md` is selected and only `ignored_tests.md` exists, use `.claude/commands/ignored_tasks.md` (alias symlink).

## Weighted Selection
Use weighted random sampling with replacement.
For each draw, probability is:
- `P(i) = weight_i / sum(all weights)`

If `seed` is provided, initialize RNG from that seed and report it in output.

## Execution Modes
### Single Worker (`parallel=1`)
For each round:
1. Draw one command by weight.
2. Execute that command.
3. Validate changes (`make validate`) before commit.
4. Commit on the active branch with a concise message including the selected command and round.

### Parallel Workers (`parallel>1`)
Only do this when BOTH are available:
- `git worktree` support
- subagent/task execution support

For each round:
1. Draw `parallel` commands independently by weight.
2. For worker index `i` in `1..parallel`:
   - Create branch: `auto/pick-task/r<round>/a<i>`
   - Create worktree: `../wt-r<round>-a<i>`
   - Start one subagent in that worktree to run its assigned command.
3. Each worker must run validation and commit on its own branch.

Integration after all workers finish:
1. Return to original branch/worktree.
2. Integrate in deterministic order `a1, a2, ... aK` using cherry-pick.
3. Keep linear history only (no merge commits).
4. If conflict occurs, resolve immediately; if not resolvable confidently, stop and report blockers.
5. Remove temporary worktrees after successful integration.

## Safety And Repo Rules
- Never run `git clean` commands.
- Never force push unless explicitly requested.
- Keep commits small and scoped.
- Prefer deterministic, reproducible logs of selections.
- Record in output: `round`, `worker`, selected command, and `seed`.

## Fallback Behavior
If parallel prerequisites are missing, fall back to single-worker mode and continue until `rounds` selections are completed.
