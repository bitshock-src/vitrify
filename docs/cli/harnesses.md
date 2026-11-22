# Harness CLIs

> Prepare Claude Code, Codex, or OpenCode for `vitrify run` and interactive builds: probing, role profiles, permissions, and the notes specific to each harness.

Use this page to set up the harness CLI that builds your units. A **harness CLI** is a coding-agent command-line tool that vitrify drives headlessly: each worker is one headless session building one unit, and each escalation is one call. Workers ride your existing harness login — vitrify stores no credentials and calls no model API itself — and vitrify passes no permission flag of its own. The supported harnesses, and the versions the defaults were researched against, are in the [Support Matrix](../reference/support-matrix.md#harness-clis).

## Requirements

Install at least one harness by its vendor's instructions, and sign in. Every vitrify command except `vitrify run` and the interactive kickoff works without one.

| Harness | Executable | Used by |
| --- | --- | --- |
| Claude Code | `claude` | `vitrify run`, interactive kickoff and authoring |
| Codex | `codex` | `vitrify run`, interactive kickoff and authoring |
| OpenCode | `opencode` | `vitrify run`, interactive kickoff and authoring |

## How vitrify Uses a Harness

`vitrify run` invokes a harness at exactly two of vitrify's three inference points; the third is you, authoring interactively. Use this table to see each role.

| Role | Invoked for | Volume |
| --- | --- | --- |
| `worker` | one per claimable unit, in parallel, up to `run.worker_timeout_minutes` each | high |
| `retry` | the worker for a unit whose earlier attempt did not land, with that attempt's files and the reason | occasional |
| `escalation` | a one-shot verdict — `retry`, `release`, `block`, or `abort` — on a unit that keeps failing or a merge that conflicts | rare |

Each worker starts from the recorded headless command plus its role's flags, with the generated worker prompt as the final argument, in a worktree outside your repository and with a `TMPDIR` of its own. Its output goes to its lane's records, `.vitrify/logs/<job>/<slug>/` (the SDD's slug); its last line is its report.

## Probe Harnesses With init and probe

`vitrify run` uses only the spellings recorded in the `[probe]` tables of `~/.vitrify/config.toml`. `vitrify init` records them when it sets the machine up; re-run `vitrify probe` after installing or upgrading vitrify or a harness CLI. `vitrify probe` prints what it found and each `change:` it would make to the record, and asks before it writes — `-y` (`--assume-yes`) answers yes; with no answer on stdin it changes nothing. It changes nothing else in the file. For each of `claude`, `codex` and `opencode`: a CLI not on PATH is `untested`, never an error. Otherwise the probe tries the harness's one headless spelling, passing the prompt `Reply with exactly: OK`, and records it when it exits 0 with non-empty stdout:

| Harness | Spelling |
| --- | --- |
| claude | `claude -p --output-format stream-json --verbose` |
| codex | `codex exec --json` |
| opencode | `opencode run --format json` |

Each spelling streams the session as it happens, one event per line, so a worker's log holds everything up to the moment it stops — however it stops. Each probe is a real one-shot model call. It runs in its own process group and is killed after 60 s (`VITRIFY_PROBE_TIMEOUT_SECS`). A CLI that is present but does not accept its spelling is a `FAIL`, and the line names the spelling it needs; update the CLI until it does.

For each harness with a recorded spelling, the probe then prints one line per role (`worker`, `retry` when config.toml names one, `escalation`) giving the model, effort and extra args `run` will pass, from the role profiles in `~/.vitrify/config.toml`. A role that adds flags gets its own probe — the recorded spelling plus those flags — and a role probe that fails, or a profile that does not parse, is a `FAIL`. So a model your account cannot reach fails at `vitrify init` or `vitrify probe`, not mid-run.

## Choose the Harness

When several harnesses are recorded, `vitrify run` uses `harness.preferred` if it is set, else the first of `claude`, `codex`, `opencode`. Set it in `~/.vitrify/config.toml` (or a repository's `.vitrify/config.toml`, or `VITRIFY_HARNESS_PREFERRED`):

```toml
[harness]
preferred = "codex"
```

The job header of a run of one SDD shows what a run selected, in its `Harness:` and `Roles:` lines. A project run's header has no such lines; its `run-started` event in `events.jsonl` and `vitrify run status --json` name the harness.

## Configure Role Profiles

`vitrify run` drives inference in three roles — `worker`, `retry` (the worker for a unit whose earlier attempt did not land) and `escalation` — and each gets its own model and reasoning effort, per harness CLI. They are the `[harness.<name>]` tables of `~/.vitrify/config.toml`: `vitrify init` writes the shipped defaults, compiled into the binary from [`crates/vitrify-core/assets/config.toml`](../../crates/vitrify-core/assets/config.toml), and you edit them in place:

```toml
[harness.claude]
model_flag = ["--model", "{model}"]     # how this CLI spells each knob
effort_flag = ["--effort", "{effort}"]

[harness.claude.worker]                 # one per claimable unit, in parallel
model = "opus"
effort = "medium"

[harness.claude.retry]                  # a unit's second and later attempts
model = "opus"                          #   (optional: without it, retries
effort = "high"                         #   run as the worker does)

[harness.claude.escalation]             # rare triage verdict; strongest
model = "best"
effort = "xhigh"
args = []                               # optional extra argv, verbatim
```

The role's flags are appended to the headless spelling the probe recorded, before the prompt. An empty `model` or `effort` omits its flag and leaves the harness's own default (its config file, env) in force; values pass through unvalidated, since each CLI owns its vocabulary. Unknown keys are refused by name. `vitrify probe` probes each role of each working harness, so a model the account cannot reach fails there rather than mid-run; the job header of a run of one SDD prints the selected harness's roles; a role profile that does not parse fails `vitrify probe` and `run` preflight. A repository's `.vitrify/config.toml` may override them for that repository, key by key: a `[harness.claude.worker]` table holding only `model = "sonnet"` changes the worker's model there and keeps its effort and `args`. `vitrify probe` probes the profiles of `~/.vitrify/config.toml` alone; `vitrify run` reads a repository's overrides, and its preflight refuses one that does not parse.

Use this table to read each key.

| Key | Table | Meaning | Default |
| --- | --- | --- | --- |
| `model_flag` | `[harness.<harness>]` | how the CLI spells the model flag; `{model}` is replaced by the role's model | per harness (below) |
| `effort_flag` | `[harness.<harness>]` | how the CLI spells the effort flag; `{effort}` is replaced by the role's effort | per harness (below) |
| `model` | `[harness.<harness>.<role>]` | the role's model; empty omits the flag | per role (below) |
| `effort` | `[harness.<harness>.<role>]` | the role's reasoning effort; empty omits the flag | per role (below) |
| `args` | `[harness.<harness>.<role>]` | extra argv appended verbatim | none |

The `[harness.<harness>.retry]` table is optional: delete it and retries run exactly as first attempts do. An upgrade never changes your config.toml, so new shipped defaults do not reach it on their own: [`crates/vitrify-core/assets/config.toml`](../../crates/vitrify-core/assets/config.toml) holds the defaults of each release, to copy from.

### Shipped Defaults

Shipped defaults (the file documents the alternatives and reasoning):

| Harness | Worker | Retry | Escalation |
| --- | --- | --- | --- |
| claude | `opus` · `medium` | `opus` · `high` | `best` (Fable, else Opus) · `xhigh` |
| codex | `gpt-6.1-sol` · `medium` | `gpt-6.1-sol` · `high` | `gpt-6-astra` · `xhigh` |
| opencode | `anthropic/claude-opus-5-5` · `medium` | `anthropic/claude-opus-5-5` · `high` | `anthropic/claude-opus-5-5` · `xhigh` |

Workers build decision-complete units — clear-scope implementation, at volume — so they get a strong coding model at moderate effort. A unit that has already failed once has shown itself harder than that: its retry gets the same model one effort step up, along with the failed attempt's files and the reason it did not land. Escalation gets the strongest model at high effort, one tier below the top: vendors flag the top tier as prone to overthinking (Claude `max`) or as delegating to sub-agents (Codex `ultra`).

## Grant Worker Permissions

vitrify passes no permission flag of its own. A worker is a headless session — nobody answers a permission prompt — and it has to edit files and run the SDD's commands (and `vitrify trace record`) in a worktree that lies outside your repository: by default under `~/.vitrify/worktrees/`, at the directory the job header prints (`run.worktree_dir` moves it). If your harness settings do not already allow that there, grant it per role through `args` in the role profiles of `~/.vitrify/config.toml`; how much a worker may do unasked is yours to decide.

> [!WARNING]
> Permissions granted through `args` apply to unattended sessions that edit files and run the commands your spec names. Grant the narrowest set that lets a worker edit its worktree and run its unit's commands, and build only specs you trust: the gate executes the SDD's own §5 Commands and Verification rows through the shell, by design.

For example, to pass a Claude Code permission flag to the worker and retry roles, add `args` to each — `<flag>` and `<value>` stand for the permission option your Claude Code version supports, such as its `--permission-mode` or `--allowedTools` flags:

```toml
[harness.claude.worker]
model = "opus"
effort = "medium"
args = ["<flag>", "<value>"]

[harness.claude.retry]
model = "opus"
effort = "high"
args = ["<flag>", "<value>"]
```

Run `vitrify probe` afterwards: a role that adds flags gets its own probe, so a flag the CLI rejects fails there.

## Build Interactively

You can also build from a harness session instead of `vitrify run`: send it `Use vitrify cli to build the spec from this session, without vitrify run.`, and it follows the kickoff procedure, which `vitrify brief kickoff` prints. [Or Build Interactively](../user-guides/run-a-build.md#3-or-build-interactively) gives the command for each harness. To have a session start and watch `vitrify run` instead, send `Use vitrify cli to build the spec.`; the procedure is `vitrify brief run`. Nothing is installed into the harness itself: every procedure is served on demand by `vitrify brief <task>`, and `vitrify brief` lists them.

## Claude Code

These notes apply only to Claude Code (`claude`).

- `--model` takes an alias (`fable`, `opus`, `sonnet`, `haiku`, `best`) or a full ID; aliases track the latest release. `best` is Fable where the account has it, else Opus.
- `--effort` takes `low`, `medium`, `high`, `xhigh`, or `max`. For a cheaper worker, the shipped file suggests `model = "sonnet"` with `effort = "high"`.
- Claude Code's `CLAUDE_CODE_EFFORT_LEVEL` env var outranks `--effort` — leave it unset when running `vitrify run`.
- The streaming spelling writes one JSON event per line. To print what a worker says as it says it:

  ```sh
  tail -f .vitrify/logs/<job>/<slug>/worker-<job>-w1.out \
    | jq -r 'select(.type == "assistant") | .message.content[]? | select(.type == "text") | .text'
  ```

## Codex

These notes apply only to Codex (`codex`).

- `-m` takes a model slug; there are no aliases, so pinned slugs age — update them on new releases.
- Effort is a config override, spelled `-c model_reasoning_effort="<effort>"`: `low`, `medium`, `high`, `xhigh`, `max`, or `ultra` (GPT-6 family; `gpt-6-luna` lacks `ultra`). For a cheaper worker, the shipped file suggests `model = "gpt-6-luna"` with `effort = "high"`.

## OpenCode

These notes apply only to OpenCode (`opencode`).

- OpenCode models are `provider/model`, so set them to a provider you hold credentials for.
- Its effort `variant` vocabulary is per provider; the shipped file lists the values for Anthropic, OpenAI, Google, and OpenRouter-compatible providers.
- OpenCode 1.x spells effort as `--variant`. OpenCode 2.x moves it into a `#variant` model suffix: there, set each role's `effort = ""` and put the variant in the model, as in `model = "anthropic/claude-opus-5-5#medium"`.
- OpenCode has no `best` alias; name the escalation model explicitly.

## Limitations

These are the boundaries of vitrify's harness support.

- vitrify drives only Claude Code, Codex, and OpenCode, and has no direct-API inference backend: workers ride the user's existing harness auth.
- Role values are not validated by vitrify; each CLI owns its vocabulary, and a value it rejects fails the role's probe in `vitrify init` or `vitrify probe`.
- Each harness is driven through its streaming spelling only; a CLI that lacks it cannot build.
- vitrify does not count tokens or cost. Whatever the harness itself reports about a session is in the last events of that worker's log.

## Next Steps

- [Run a Build](../user-guides/run-a-build.md) — start a build with the harness you prepared.
- [Configuration](configuration.md) — the run settings that decide how many workers start and for how long.
- [Monitor a Run](../user-guides/monitor-a-run.md#follow-a-worker-session) — follow a worker's session as it works.
