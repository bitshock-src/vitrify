# vitrify

> Hand a whole spec to your AI coding agents, let them build it in parallel, unattended, for hours, and keep revising it as you learn. Deterministic orchestration decides what runs, mechanical gates decide what lands, and each change to the spec rebuilds exactly what it reaches.

`vitrify-cli` installs `vitrify`, one binary for the whole spec-driven development (SDD) lifecycle. You and your coding agent write a decision-complete spec; `vitrify run` builds it. Each unit of the spec goes to its own headless session of Claude Code, Codex, or OpenCode, and the sessions run in parallel while Rust code — not a model — tracks the dependencies and contracts between units, gates every merge on the combined tree, and retries or escalates what fails. Build state lives in git, so a run stopped at any point resumes where it left off, and every run ends in one clear report: `Complete.` or `Blocked.`

## Install

```sh
cargo install vitrify-cli --locked
```

The crate is `vitrify-cli`; the binary it installs is `vitrify`. `--locked` builds with the dependency versions the release was published with.

| Requirement | Version | Needed for |
| --- | --- | --- |
| macOS or Linux | on Windows, inside WSL 2 | everything |
| Rust and cargo | 1.89 or later | installing |
| git | 2.30 or later | every command; the only runtime dependency |
| A harness CLI, signed in | Claude Code, Codex, or OpenCode | `vitrify run` and interactive builds only |

Then set the machine up, once:

```sh
vitrify init    # ~/.vitrify/config.toml with every default, then probes git + harness CLIs
```

## Use It

Open a session of Claude Code, Codex, or OpenCode at the root of your repository and say what you want, beginning "Use vitrify cli to" — the agent learns the rest from `vitrify --help`:

```text
Use vitrify cli to turn this PRD into a spec ready to build: <your PRD, or its path>
```

Once the spec is ready, build it from your own terminal, and watch it from a second one:

```sh
vitrify run --dry-run                      # preflight and the plan; publishes nothing
nohup vitrify run > run.out 2> run.err &   # outlives the terminal; ends Complete or Blocked
vitrify run status --follow                # each unit's stage, time and changes
```

## Documentation

- [vitrify on GitHub](https://github.com/bitshock-src/vitrify#readme) — what vitrify does, and how it works.
- [Installation](https://github.com/bitshock-src/vitrify/blob/main/docs/getting-started/installation.md) — requirements, machine setup, harness CLIs, upgrading, and uninstalling.
- [Quick Start](https://github.com/bitshock-src/vitrify/blob/main/docs/getting-started/quick-start.md) — from a PRD to a verified build, with the shipped example.
- [Command Reference](https://github.com/bitshock-src/vitrify/blob/main/docs/cli/command-reference.md) — every verb, output, and exit code.
- [Documentation index](https://github.com/bitshock-src/vitrify/blob/main/docs/README.md) — every page.

## License

MIT — see [`LICENSE`](https://github.com/bitshock-src/vitrify/blob/main/LICENSE).
