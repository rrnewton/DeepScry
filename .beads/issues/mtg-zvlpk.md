---
title: 'Bug-report gh filing fails on VM: hardcoded /usr/bin/with-proxy + baked build cwd both ENOENT'
status: open
priority: 2
issue_type: bug
created_at: 2026-06-04T06:01:06.393506310+00:00
updated_at: 2026-06-04T06:01:06.393506310+00:00
---

# Description

DISCOVERED 2026-06-04 by live smoke-test of mtg-5ejgo on deepscry.net. The two-phase bug-report widget works perfectly (disk-confirm + graceful github-failure, no spinner), but the GitHub issue-filing step itself FAILS on the production VM with "No such file or directory (os error 2)" (ENOENT) — so real issues are never filed. SEPARATE server/deploy bug from the widget (mtg-5ejgo DONE).

ROOT CAUSE (code inspection + read-only ssh to VM 178.156.252.200):
1. PRIMARY — hardcoded proxy. run_gh_command_with_runner (server.rs:3533) + claude-autofix spawn (:3562) unconditionally prepend `/usr/bin/with-proxy`: ["/usr/bin/with-proxy","/usr/bin/gh",...]. with-proxy is a Meta-devserver egress wrapper, ABSENT on the Hetzner VM. So Command::new("/usr/bin/with-proxy") ENOENTs before gh runs. Confirmed gh works DIRECTLY on the VM (no proxy): `gh api user` -> rrnewton.
2. SECONDARY — baked build cwd. run_command/run_gh set current_dir(bug_report_repo_root()); bug_report_repo_root() (:3419) = env!(CARGO_MANIFEST_DIR).parent() = compile-time path /mnt/btrfs-storage/deepscry/worktrees/slot05, ABSENT on the VM -> current_dir ENOENT even after fixing #1. repo_root is used ONLY as the gh cwd; gh is already scoped with -R OWNER/REPO so cwd is irrelevant to correctness, just must EXIST.

VM STATE (confirmed working): /usr/bin/gh present; gh auth = logged in rrnewton w/ active github_pat; gh api user OK. Auth+egress+repo-scoping all fine; ONLY the spawn ENOENT (proxy + cwd) blocks filing.

FIX (no secrets — gh already authed on VM):
A. Config-drive the command prefix, default NO proxy (direct /usr/bin/gh). Env MTG_GH_PROXY: prepend only when set+nonempty; default unset=direct. Apply to gh (:3533) + claude-autofix (:3562). Update with-proxy tests (:4244,:4463) to expect direct-by-default + add a MTG_GH_PROXY-set test.
B. Fix gh cwd: use a guaranteed-existing dir (runtime report_dir or std::env::temp_dir()) instead of the baked build path.
C. deploy-cloud.sh config: no proxy on prod VM (default); set MTG_GH_PROXY only where an egress proxy is required. No GITHUB_TOKEN needed.

ALTERNATIVE (deferred, heavier): REST API via reqwest + GITHUB_TOKEN + native timeout, dropping gh-binary/spawn_blocking. Needs a user-provisioned GITHUB_TOKEN secret — flag for user, do NOT auto-provision.

VERIFY: after A+B, redeploy + confirm a real submit files an issue (throwaway/closeable, no junk).
