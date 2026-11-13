# Firstmate's rules, enforced by hook

The chief of staff runs as firstmate (see [The managed firstmate clone](firstmate.md)), and
firstmate's `AGENTS.md` has hard rules about merging, tearing down work, writing to projects and
publishing. Firstmate follows them as instructions. Velvet Hammer also checks the ones a tool call
shows, in its PreToolUse hook (`src/guard.rs`, served by `src/bridge.rs`), before every tool use
of the chief of staff.

The **Firstmate rules** card in the agents column opens a page that lists each rule with what it
enforces, what it matches and what it does, a switch to turn it off, and what it decided lately.
All rules are on by default.

## How a rule decides

- A rule only **denies** a tool use, with a reason the model reads and can act on, or **asks**
  the captain in **Needs your input**, with a question that names the exact pull request, task,
  file or remote, followed by the reason and the tool's whole input. A rule never allows.
- Rules run before the [tool-approval settings](../README.md#tool-approval). A rule's deny is
  final. A rule's question is asked in every approval mode, including **Run without asking**, and
  no allow rule can answer it; only a deny rule of the captain's wins over it. A tool use that no
  rule matches is decided by the tool-approval settings as before.
- The rules apply only while the chief of staff runs as firstmate, and only to the chief of
  staff's own tool uses. Crewmates run in their own sessions, which this hook does not see.
- Every decision a rule makes is in the activity log, tagged with the rule.
- The choice of rules is kept in `firstmate-rules.json` in the app's data directory
  (`~/.velvet-hammer`, or `$VELVET_HAMMER_DATA`) and applies to a running session at once. If the
  file cannot be read, every rule is on and the page says why.

Rules read command text, like the approval rules. For a Bash command they look at each part
joined with `&&`, `;`, `|`, `$( )` or backticks; drop variable assignments and `env`, `sudo`,
`command`, `exec`, `nohup`, `time` and `nice` in front; take `bash script` as the script; follow
`cd` and `git -C`; and resolve `~`, `$HOME`, `$FM_HOME`, `.` and `..` as written.

## The rules

Sources are firstmate's `AGENTS.md` and scripts at commit `427056c4`.

| Rule (key) | Enforces | Tool calls matched | Decision | Tests |
| --- | --- | --- | --- | --- |
| Merge only through firstmate's merge script (`merge-through-script`) | `AGENTS.md` section 7: use `bin/fm-pr-merge.sh` for every task PR merge and `bin/fm-merge-local.sh` for local-only landing; never call a lower-level merge command around their guards. | Bash: `gh pr merge`, `glab mr merge`, `tea pulls merge` (or `pull`, `pr`), and `gh api` on a path ending `/pulls/<n>/merge`. `-R`/`--repo` and other flags may come between the words. | Deny: "merges around firstmate's guards. Merge a task's pull request with bin/fm-pr-merge.sh &lt;task-id&gt; &lt;pr-url&gt; (or land local-only work with bin/fm-merge-local.sh &lt;task-id&gt;); Velvet Hammer then asks the captain." | `guard::a_lower_level_merge_is_denied_with_the_script_to_use`, `bridge::every_rule_denies_or_asks_and_the_captains_answer_decides` |
| Every merge waits for your word (`merge-asks-captain`) | `AGENTS.md` hard rule 2: never merge a PR without the captain's explicit word. | Bash: `fm-pr-merge.sh` or `fm-merge-local.sh`, by any path, except with `-h` or `--help`. | Ask: "Merge &lt;pr-url&gt;?" or "Land task &lt;id&gt; on its project's default branch?" | `guard::the_merge_script_asks_the_captain_naming_the_pull_request`, `bridge::a_merge_asks_the_captain_by_name_and_no_allow_rule_answers_for_them`, `firstmate_rules_deny_a_force_push_and_ask_about_a_merge_by_name` |
| No admin-bypass merges (`no-admin-merge`) | `bin/fm-pr-merge.sh` refuses `--admin` unless `--attended-override` is given for an explicit captain instruction (its header admits that this combination bypasses branch protection). | Bash: `--admin` given to `fm-pr-merge.sh` or `gh pr merge`. | Deny: "would merge with --admin, which bypasses the project's branch protection. Merge without --admin through bin/fm-pr-merge.sh, or ask the captain; only the captain can turn this rule off, on Velvet Hammer's Firstmate rules page." | `guard::an_admin_merge_is_denied_even_through_the_script` |
| No force pushes (`no-force-push`) | `AGENTS.md` hard rule 1: no path authorizes forcing. Firstmate has no force-push rule of its own; this one is the captain's. | Bash: `git push` with `--force`, `-f` (also in a cluster such as `-uf`), `--force-with-lease`, `--mirror`, or a refspec starting with `+`. | Deny: "force-pushes, which can discard work on the remote. Push without forcing; if the remote has moved, investigate and ask the captain." With the rule off, the push still asks as a push. | `guard::a_forced_push_is_denied_and_a_plain_one_asks`, `bridge::a_firstmate_rule_denies_with_what_to_do_whatever_the_mode` |
| Discarding unlanded work waits for your word (`discard-asks-captain`) | `AGENTS.md` hard rule 3: never tear down unlanded work, and never use `--force` unless the captain explicitly authorized discarding it. | Bash: `fm-teardown.sh` with `--force`; `git worktree remove` with `--force` or `-f`. | Ask for the teardown: "Discard the unlanded work of task &lt;id&gt;?" Deny the worktree removal: "removes a worktree whatever it holds. Retire a task with bin/fm-teardown.sh &lt;task-id&gt;, which owns the landed-work check …". A teardown without `--force` is left to the script, which refuses unlanded work itself. | `guard::a_forced_teardown_asks_and_a_forced_worktree_removal_is_denied` |
| Projects are read-only to the chief of staff (`projects-read-only`) | `AGENTS.md` hard rule 1: never edit, commit or run state-changing commands under `projects/` or in a project worktree, except a concrete operation the captain approves in the moment. | Edit, Write, MultiEdit and NotebookEdit on a file under `$FM_HOME/projects/` or a task worktree. Bash: `git` add, am, apply, checkout, cherry-pick, clean, commit, merge, mv, pull, rebase, reset, restore, revert, rm, stash, switch, tag, worktree and branch (except the listing forms) in such a repository; rm, rmdir, unlink, mv, touch, mkdir, ln, chmod, chown, chgrp, truncate, shred or tee on such a path; the last operand of cp, install, rsync or ditto; `sed`, `gsed` or `perl` with `-i`; and a `>` or `>>` redirect onto such a path. Task worktrees are read from the `worktree=` and `project=` lines of `$FM_HOME/state/*.meta` at each tool use. | Ask: "Let the chief of staff edit &lt;path&gt;?", "… change &lt;path&gt;?" or "… run git &lt;subcommand&gt; in &lt;dir&gt;?". An ask rather than a deny, because the captain's in-the-moment approval is the rule's own exception. | `guard::writes_into_a_project_or_task_worktree_ask_and_reads_do_not`, `a_write_into_a_project_asks_and_names_the_file` |
| Publishing waits for your word (`publish-asks-captain`) | `AGENTS.md` section 1: a firstmate bug never becomes a push, fork, issue, comment or pull request on firstmate's own authority; section 6: project creation never authorizes an unmentioned remote, and the `project-management` skill needs the captain's consent to create a remote repository. The app cannot tell which remotes the captain approved, so it asks about each. | Bash: `git push` (every push; a forced one is also denied above), `gh repo create`/`fork`, `gh pr create`/`comment`, `gh issue create`/`comment`, `gh release create`, `gh gist create`, `glab repo create`/`fork`, `glab mr` or `issue` `create`/`note`. | Ask: "Push to &lt;remote&gt; from &lt;dir&gt;?" or "Publish with \`gh repo create\`?" | `guard::publishing_asks_with_the_exact_command`, `firstmate_rules_deny_a_force_push_and_ask_about_a_merge_by_name` |

Commands deliberately left alone, because firstmate uses them legitimately: `git fetch`,
`git clone`, `git init`, `git remote` and `git config` (the add-project procedure clones a project
and removes its remote), reading commands of any kind, a teardown without `--force`, and every
`bin/fm-*.sh` script other than the merge and teardown scripts. Firstmate's sanctioned writes to a
project (project initialization, fleet sync, self-update, local-only landing) happen inside its
scripts, where the hook does not look.

`bridge::every_rule_denies_or_asks_and_the_captains_answer_decides` runs one tool call per rule
through the hook: each deny is denied without a question, each ask is allowed when the captain
allows it and denied when the captain denies it, and commands no rule matches run by the mode.
The headless scenes `rules-settings`, `rules-merge-ask` and `rules-project-ask` show the page, a
merge question and a project-write question on the fake `claude` running as firstmate.

## Not enforceable by hook

| Firstmate rule or action | Why a PreToolUse hook does not enforce it |
| --- | --- |
| Whether the captain gave explicit word for a merge | The hook cannot read the conversation, and firstmate records only that a task is held, not that the captain said yes (`fm-pr-merge.sh` checks for an open hold, not for an approval). So every merge asks in the app instead. This also asks about merges in a `yolo` project, which firstmate would make on its own; turn **Every merge waits for your word** off to restore that. |
| Writes through a script, an interpreter or indirection: `bash -c "..."`, `eval`, `python -c`, `node -e`, `xargs`, `find -exec`, `make`, a path held in a variable or built at run time | The hook sees the command's text, not what it runs. Following these would need a shell, and firstmate's own scripts write projects through the sanctioned paths. |
| A path reached through a symlink or a relative path from a directory the command changed to in a way the hook cannot follow | Paths are resolved as written; the protected directories are also compared in their resolved form, but a command's own path is not. |
| Red-check waivers (`--allow-red`, `--allow-missing`, `--allow-no-pipeline`) | They need a current captain instruction naming the check. The merge question shows the whole command, so the captain sees a waiver before allowing it; `fm-pr-merge.sh` also refuses them while the away record exists. |
| Hard rule 4, crewmates never address the captain | Crewmates run in their own sessions, which this hook does not see. The same holds for crewmates' pushes and the delivery pipeline's pull requests. |
| Never add an agent name as a commit co-author | A commit message is free text; an agent's name cannot be told from a person's reliably. |
| Drive worker lifecycle through `bin/fm-control.sh`, never `fm-send`, for an interrupt or exit | `fm-send` is also used for ordinary messages; telling them apart needs the message's meaning. |
| Never edit `config/claude-account`, `config/pi-account` or `config/claude-accounts` to unblock a spawn | The rule forbids it for that purpose only; setting up an account is legitimate, and the hook cannot see the purpose. |
| A scout torn down without its report, or a teardown over an open captain question | `fm-teardown.sh` refuses these itself, even with `--force`. |
| `pkill -f bin/fm-watch.sh` and a persistent `cd` in the primary shell | Firstmate's own PreToolUse guards (`bin/fm-arm-pretool-check.sh`, `bin/fm-cd-pretool-check.sh`) already deny them, in the same session. |
| Removing a worktree with `treehouse` directly | Its command line was not verified against treehouse's own documentation, so a rule could deny the wrong thing. `fm-teardown.sh` owns returning a worktree. |

## Screenshots

| Scene | Screenshot |
| --- | --- |
| The page after a forced push was denied and a merge was asked about and allowed | [rules-settings.png](screens/hooks/rules-settings.png) |
| A merge through firstmate's script, waiting for the captain | [rules-merge-ask.png](screens/hooks/rules-merge-ask.png) |
| A write into a project, waiting for the captain | [rules-project-ask.png](screens/hooks/rules-project-ask.png) |
