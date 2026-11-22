# Configuration

> Configure vitrify's run, harness, and lint settings in `~/.vitrify/config.toml`, their precedence and environment variables, and the embedded content you can override.

Use this page to change how vitrify behaves on your machine or in one repository. All of vitrify's configuration lives in one file, `~/.vitrify/config.toml`, which `vitrify init` writes with every setting at its default; edit it in place, and an upgrade never changes it. The same file holds the per-role models and effort, its `[harness.<name>]` tables, described in [Harness CLIs](harnesses.md#configure-role-profiles), and the probe record, its `[probe]` tables, which `vitrify init` and `vitrify probe` write. Correctness never depends on `[run]` values: lease expiry is advisory; the fencing token (`claim_seq`, carried in branch names `sdd/<sdd>/<unit>/c<seq>`) is the mechanism, checked at the integration gate. They tune throughput.

## config.toml Example

Configuration is TOML at `~/.vitrify/config.toml`. A repository may add its own `.vitrify/config.toml` that overrides any of it for that repository — `mainline`, `[run]`, `[lint]`, `[harness]` and the role profiles, key by key — and declares its own `schema`. Only the `[probe]` tables are refused there: they describe the machine, and `vitrify init` and `vitrify probe` write them. This example shows every setting at its default, as `vitrify init` writes it:

```toml
schema = 1                     # the file format version; must be 1

mainline = ""                  # mainline branch; "" = the primary checkout's
                               #   branch, else origin/HEAD, then main or master

[run]
worker_cap = 4                 # max concurrent workers
lease_minutes = 60             # claim lease
renew_fraction = 0.25          # renew when remaining < fraction × lease
abort_failed_waves = 2         # failed attempts of one unit before the
                               #   escalation call; twice as many set it aside
worker_timeout_minutes = 120   # hard wall-clock budget per worker
gate_jobs = 0                  # Verification rows run at once;
                               #   0 = one per CPU core, 1 = one at a time
lane_cap = 0                   # lanes (one SDD's part of a run) holding their
                               #   checkouts on disk at once; 0 = no limit
priority = "depth"             # how ready units are ranked: "depth" (every unit
                               #   weighs 1) or "history" (by past session times)
poll_seconds = 30              # how often an idle run re-reads mainline
worktree_dir = ""              # "" = ~/.vitrify/worktrees/<repo>-<hash>;
                               #   relative = from the repository root
status_check_seconds = 270     # how often an agent operating a run checks
                               #   `vitrify run status` when it cannot be notified

[lint]
readset_budget_bytes = 65536   # per-unit reading-set budget (bytes by design)

[harness]
preferred = ""                 # "" = the first the probe recorded, of claude,
                               #   codex, opencode
```

After these settings come the role profiles, `[harness.claude]`, `[harness.codex]` and `[harness.opencode]` with a table per role, and the probe record, `[probe]` and a `[probe.<harness>]` table per harness CLI.

## Fields

Use this table to look up each setting, its default, and its override.

| Field | Default | Notes |
| --- | --- | --- |
| `schema` | `1` | The file format version, in `~/.vitrify/config.toml` and in a repository's `.vitrify/config.toml` alike. This vitrify reads version 1 and refuses any other value. |
| `mainline` | `""` | The mainline branch. Empty resolves it in order: the primary checkout's current branch, then `origin/HEAD` (when the checkout is detached), then `main` or `master` — so a run started or resumed from a checkout on a build branch publishes to that branch. Overridden by `--mainline` and `VITRIFY_MAINLINE`. |
| `run.worker_cap` | `4` | Maximum concurrent model sessions, across every lane of a run. Overridden by `--cap`. A single-file SDD's lane always runs one worker. |
| `run.lease_minutes` | `60` | The claim lease that `vitrify run` and generated worker prompts use. `vitrify claim` and `renew` do not read it; they take `--lease-minutes` (default 60). |
| `run.renew_fraction` | `0.25` | Renew a worker's lease when less than this fraction of it remains. |
| `run.abort_failed_waves` | `2` | Failed attempts of one unit before the one escalation call; twice as many set the unit aside for the run. |
| `run.worker_timeout_minutes` | `120` | Hard wall-clock budget per worker; the worker's process group is killed when it is spent. |
| `run.gate_jobs` | `0` | Commands at once: one pool serves every §5 command and Verification row of every lane's gate, re-check, culprit probe, preflight and head check, and `vitrify audit` runs its rows at the same count. `0` is one per CPU core, `1` is one at a time. Overridden by `--gate-jobs`. |
| `run.lane_cap` | `0` | Lanes holding their checkouts on disk at once in a project run — a lane's integration, base and culprit-probe checkouts count as one. A lane's checkout is made, and its §5 Bootstrap, Build and Lint run, when it is first admitted; units are taken highest priority first, so the lanes with the longest chains behind them are admitted first; and a lane with nothing left to do — no claim, re-check or culprit search, nothing ready or due — gives its checkouts back, so the next lane can be admitted. `0` (or less) is no limit: each lane's checkout is made at its first build or re-check and kept. There is no flag: set it in the file or with `VITRIFY_RUN_LANE_CAP`. |
| `run.priority` | `"depth"` | How a project run ranks the units that are ready. `"depth"` weighs every unit 1, so a unit's priority is the number of unfinished units on the longest chain from it to the end of the graph, itself included. `"history"` weighs a unit by how long its past worker sessions took, read from the records earlier runs left under `.vitrify/logs/` (the median, in seconds, of its last five sessions that verified, or of its last five finished sessions when none did; a unit with no history takes the median of the known weights, or 1 when none is known), so the priority is the longest chain by estimated duration. Ties break by SDD slug, then unit id. Any other value is refused (exit 2). There is no flag: overridden by `VITRIFY_RUN_PRIORITY`. |
| `run.poll_seconds` | `30` | How often an idle run re-reads mainline for a spec change, a lapsed lease, or another holder's release. |
| `run.worktree_dir` | `""` | Where unit worktrees and each SDD's integration checkout live. Empty is `~/.vitrify/worktrees/<repo>-<hash>`; a relative path is taken from the repository root. |
| `run.status_check_seconds` | `270` | How often an agent operating a run checks `vitrify run status` when its harness cannot notify it that the run ended. `vitrify brief run` prints the configured value; the run itself does not read it. See [Why 270 Seconds](#why-270-seconds). |
| `harness.preferred` | `""` | The harness `vitrify run` uses. Empty takes the first the probe recorded, of `claude`, `codex`, `opencode`. |
| `lint.readset_budget_bytes` | `65536` | The per-unit reading-set budget for lint L5 and `vitrify readset --budget`. Bytes by design: tokenizers differ between models. |

A value of the wrong type — a `run.status_check_seconds` that is not a positive integer, a `run.priority` that is neither `depth` nor `history`, for example — or a file that does not parse, is an error that names the key and the file (exit 2). Every command but `vitrify init` reads both files before it does anything else, so such an error stops any command alike, whether or not it uses the setting.

The role profiles resolve the same way, key by key: a repository's `[harness.claude.worker]` table holding only `model = "sonnet"` changes the worker's model in that repository and keeps its effort and arguments from `~/.vitrify/config.toml`. See [Configure Role Profiles](harnesses.md#configure-role-profiles).

## Precedence and Environment Variables

Settings resolve in this order, first match wins: CLI flags > `VITRIFY_*` env > repo config > home config > built-in defaults. Each setting maps to an environment variable named `VITRIFY_<SECTION>_<KEY>`:

| Variable | Setting |
| --- | --- |
| `VITRIFY_MAINLINE` | `mainline` |
| `VITRIFY_RUN_WORKER_CAP` | `run.worker_cap` |
| `VITRIFY_RUN_LEASE_MINUTES` | `run.lease_minutes` |
| `VITRIFY_RUN_RENEW_FRACTION` | `run.renew_fraction` |
| `VITRIFY_RUN_ABORT_FAILED_WAVES` | `run.abort_failed_waves` |
| `VITRIFY_RUN_WORKER_TIMEOUT_MINUTES` | `run.worker_timeout_minutes` |
| `VITRIFY_RUN_GATE_JOBS` | `run.gate_jobs` |
| `VITRIFY_RUN_LANE_CAP` | `run.lane_cap` |
| `VITRIFY_RUN_PRIORITY` | `run.priority` |
| `VITRIFY_RUN_POLL_SECONDS` | `run.poll_seconds` |
| `VITRIFY_RUN_WORKTREE_DIR` | `run.worktree_dir` |
| `VITRIFY_RUN_STATUS_CHECK_SECONDS` | `run.status_check_seconds` |
| `VITRIFY_HARNESS_PREFERRED` | `harness.preferred` |
| `VITRIFY_LINT_READSET_BUDGET_BYTES` | `lint.readset_budget_bytes` |

Two more variables are read directly. Use this table to find them.

| Variable | Default | Meaning |
| --- | --- | --- |
| `VITRIFY_HOME` | `$HOME/.vitrify` | The vitrify home: `config.toml`, content, and the default worktree root. `vitrify init` creates it. |
| `VITRIFY_PROBE_TIMEOUT_SECS` | `60` | How long `vitrify init` and `vitrify probe` wait for each harness probe before killing it. |

## Tune Throughput

These settings change speed, never results.

- `worker_cap` is how many units build at once. Dispatch is continuous, so a bundle with wide parallelism uses a large cap fully.
- `gate_jobs` is how many commands run at once — one pool for the whole run, however many lanes gate, re-check or probe at once; the rows are the bulk of them — and how many rows `vitrify audit` runs at once. The default follows the machine: one per CPU core available to the process, the rule `cargo test` uses for its test threads. As there, the checks are expected to be independent of each other — the protocol already asks that rows be hermetic — and a number is the opt-out: `gate_jobs = 1` (`--gate-jobs 1`, `VITRIFY_RUN_GATE_JOBS=1`) runs them one at a time, which is right when rows change shared state other rows read. A row that fails while others were running is run once more alone before it counts, so rows that merely disturb each other cannot turn a gate red. Each row has a `TMPDIR` of its own, so a tool that keeps a cross-process lock there (uv does, per project) does not hold other rows off: let the Bootstrap command bring the environment up to date, so that rows only read it. Within a gate, Bootstrap, build, lint and the Test command run one at a time, each holding one token; other lanes' commands run beside them within the pool.
- `worktree_dir` is where builds happen. Wherever it is, it must not be under a directory an ignore file of the repository covers. A `worktree_dir` inside the working tree is excluded through `.git/info/exclude`, and refused if one of the repository's ignore files covers it.

## Why 270 Seconds

`run.status_check_seconds` paces an agent session that watches a run, not the run. Each check re-sends the agent's whole conversation to its model, so the interval decides what watching costs.

- 270 seconds sits just inside the five-minute lifetime of a model provider's prompt cache, so each check re-reads the conversation at the cached price. The half minute of margin covers the time the model spends generating, which counts against the lifetime.
- Checking more often re-reads the same cache for little gain; checking less often misses the cache on every check.
- With cache reads at a tenth of the input price, 13 cached checks an hour cost about 1.3 full reads of the conversation, while 3 uncached checks an hour cost 3.

Where a model's cache lasts longer, raise the setting. An agent whose harness notifies it when a background command ends does not check at all; [`vitrify brief run`](command-reference.md#vitrify-brief) gives the procedure.

## Override Embedded Content

Content (briefs, templates, schemas) is embedded in the binary and materialized to `~/.vitrify/` by `vitrify init`; a file on disk overrides the embedded copy; deleting restores the default; an unmodified default follows the installed binary, refreshing on an upgrade or a rebuild; `vitrify probe` reports overrides and staleness. `config.toml` is not content: after `vitrify init` writes it, nothing restores or refreshes it. Use this table to find each file.

| Path under `~/.vitrify/` | Content | Read by |
| --- | --- | --- |
| `briefs/onboard.md`, `authoring.md`, `change.md`, `run.md`, `kickoff.md`, `build-unit.md`, `integrate.md` | the seven procedures | `vitrify brief`, and so every worker prompt |
| `briefs/escalation.md` | the escalation prompt template | `vitrify run` |
| `schemas/sdd-schema.md`, `schemas/tdd-schema.md` | the normative schemas | `vitrify schema show` |
| `templates/sdd-template-bundle/`, `templates/sdd-template-single.md`, `templates/tdd-template.md` | the instance templates | `vitrify new` |

To override a file, edit it in place. To restore the shipped version, delete it. `vitrify probe` lists each override as a `note`, and marks one forked from another shipped version as stale against the embedded copy.

> [!WARNING]
> An edited brief or schema changes what every worker and authoring session reads. An override keeps its content across upgrades, so it misses changes in newer shipped versions until you merge them in; check `vitrify probe`'s notes after every upgrade.

## Files vitrify Writes

vitrify writes only under `~/.vitrify/`, the repository's `.vitrify/`, your system temp directory, and — through its own commits — the SDD's files on mainline. Only `vitrify init` creates `~/.vitrify/`, and every other command refuses until it exists, naming `vitrify init`. Use this table to find each path.

| Path | Written by | Purpose |
| --- | --- | --- |
| `~/.vitrify/config.toml` | `vitrify init`; its `[probe]` tables, `vitrify probe` | every setting, the role profiles, and the recorded headless command per harness |
| `~/.vitrify/briefs/`, `schemas/`, `templates/` | `vitrify init`; restored and refreshed by the commands that read them | materialized content |
| `~/.vitrify/worktrees/<repo>-<hash>/` | `vitrify run` | per SDD, its integration checkout, base checkout and culprit probe checkout; one worktree per attempt |
| `<repo>/.vitrify/run.json` | `vitrify run` | a snapshot of the latest run |
| `<repo>/.vitrify/logs/<job>/` | `vitrify run` | the run record copy |
| `<repo>/.vitrify/logs/<job>/project.json` | `vitrify run` | what the run will do, each lane's state, unit and Verified counts (kept current) and records directory, the critical path as it ran, for `vitrify run status` |
| `<repo>/.vitrify/logs/<job>/<slug>/` | `vitrify run` | one lane's records: worker logs and prompts, escalation evidence, and `gate.json`, what its integrator is doing now, for `vitrify run status` |
| `<repo>/.vitrify/logs/<job>/events.jsonl` | `vitrify run` | the run's event log, one JSON object per line, appended as it happens, a lane's events naming it |
| `<repo>/.vitrify/logs/<job>/report.txt` | `vitrify run` | the run's final report, for `vitrify run status` |
| `<repo>/.vitrify/jobs/<job>.lock` | `vitrify run` | the lock that marks a run alive |
| `<repo>/.vitrify/tx-*` | `claim`, `renew`, `release`, `vitrify run` | throwaway checkouts for mainline transactions |
| `<repo>/.vitrify/primary-behind` | transactions and runs | which commit a primary checkout that could not follow still holds |
| `<system temp>/vfy-<hash>/` | gate commands and workers | per-command `TMPDIR` slots, mode 0700, reused rather than removed |

`.vitrify/` carries a `*` ignore file, so the repository never sees vitrify's scratch, and nothing is ever built in it.

## Common Configuration Issues

- **`error: invalid config value for …`** — a value has the wrong type, or is not a positive integer where one is required; the message names the key and the file.
- **`error: vitrify is not set up on this machine`** — `~/.vitrify/config.toml` does not exist; run `vitrify init`.
- **`vitrify probe` or preflight fails on a role profile** — a `[harness.<name>]` table names an unknown key (refused by name) or a value of the wrong type; fix the line the error names.
- **A repository's `.vitrify/config.toml` is refused** — it carries `[probe]` tables. The probe record describes the machine: it belongs in `~/.vitrify/config.toml`, where `vitrify init` and `vitrify probe` write it.
- **A run refuses its worktree root** — `run.worktree_dir` is under a directory one of the repository's ignore files covers; move it outside the working tree.
- **Claude Code ignores the configured effort** — `CLAUDE_CODE_EFFORT_LEVEL` is set in the shell; unset it before `vitrify run`.
- **`vitrify claim` uses a different lease than runs** — `claim` and `renew` take `--lease-minutes` and do not read `run.lease_minutes`.

## Related Topics

- [Harness CLIs](harnesses.md) — the role profiles and their defaults, and probing with `vitrify init` and `vitrify probe`.
- [Design Rationale](../about/design-rationale.md#thresholds) — the basis for each default.
- [Command Reference](command-reference.md) — the flags that override these settings.
