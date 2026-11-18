# Author a Spec

> Take a Product Requirements Document (PRD) to an Active, lint-clean SDD that `vitrify run` can build: route the work, interview the human, write the TDD, derive the SDD, decide the test posture, and pass the gates.

Use this guide when you start a new spec from a PRD — for a new project or a feature — or add a new SDD to a repository. Authoring is the expensive step by design: splitting and sizing are plan-time work, frozen as a reviewed artifact and held to `vitrify lint` before the SDD goes Active. `vitrify brief authoring` prints the full authoring procedure; the phases below are that procedure in brief, with the calc example as the worked reference for each artifact. The ideas behind each phase are explained in [Specs: TDD and SDD](../about/concepts/specs.md), [Work Units](../about/concepts/units.md), [Contracts](../about/concepts/contracts.md), and [Test Posture](../about/concepts/test-posture.md).

## Agent Prompts

Your coding agent can take every phase of this guide with you. In a session of Claude Code, Codex, or OpenCode at the repository root, send the prompt for your PRD. Use this table to pick the prompt.

| To | Send |
| --- | --- |
| spec a new project | `Use vitrify cli to turn this PRD into a spec ready to build: <your PRD, or its path>` |
| spec a feature for a project you have | `Use vitrify cli to add this feature: <its PRD, or its path>` |

The agent finds the authoring procedure in `vitrify --help` — `vitrify brief authoring` — and follows it. It routes the PRD ([step 1](#1-start-from-a-prd)), interviews you at most five questions at a time, and writes the TDD for you to approve. A feature inside an SDD you have becomes a TDD revision and a change to that SDD, which the agent lands as in [Change a Spec](change-a-spec.md); anything else becomes a new TDD and SDD, which it derives, lints, commits, and preflights. It ends **Ready to build**, quoting the evidence, or with the gaps that remain and who must close them. Design authority stays yours: the agent asks rather than decides, records what nobody has decided as an open question, and never approves the TDD itself.

## Before You Start

Check that you have what each phase needs. Use this table to compare your project with the calc example.

| Requirement | Your project | calc example |
| --- | --- | --- |
| A PRD | what to build, for whom, and how to tell it works — a file, or a few lines | — |
| A git repository | the repository the code will live in | `fixtures/example/`, copied into its own repository |
| vitrify installed, and the machine set up with `vitrify init` | see [Installation](../getting-started/installation.md) | — |
| A person with design authority | approves the TDD and answers questions | the TDD's author |
| Exact tool and dependency versions | for Technology Decisions | Python 3.10 or later, standard library only |
| A verification approach for each unit | your test posture | an acceptance suite written before the build |

## 1. Start From a PRD

Most specs begin as a Product Requirements Document (PRD): what to build, for whom, and how to tell it works, in the words of the person who holds design authority. It may be a file or a few lines, however rough: the interview fills in what it leaves open. Before writing anything, decide where the work belongs, then turn the PRD into a TDD with that person.

### Route the Work

Run `vitrify locate` to list the repository's SDDs; `vitrify locate --json` adds each SDD's `scope_roots`. Use this table to decide what the PRD becomes.

| The PRD describes | It becomes | Procedure |
| --- | --- | --- |
| a feature inside an existing SDD's `scope_roots` | a TDD revision plus a change entry that adds units | the interview below, then `vitrify brief change` and [Change a Spec](change-a-spec.md) |
| a small fix inside an existing SDD | a change entry, not a new TDD | `vitrify brief change` |
| a new scope | a new TDD and a new SDD, with its own `sdd:` slug and disjoint `scope_roots` | the steps below |

Fit the process to the change: a new TDD is for a new scope, not for every request.

### Interview the Human

Ask the human in rounds of at most five targeted questions, each aimed at a TDD section that cannot yet be written, until every section, §1–§6, can be written.

1. Read the PRD against the TDD schema (`vitrify schema show --tdd`) and find the sections it leaves open.
2. Ask at most five targeted questions about those sections.
3. Write each answer into the TDD, in the section it decides.
4. Record what is still unknown as an open question in the TDD — never a guess.
5. Repeat until §1–§6 can be written, then ask the human to approve the TDD — or, for a feature inside an existing SDD, the TDD revision, before the SDD changes.

## 2. Write the TDD

The TDD is the design authority — the why behind the spec. Create it from the embedded template and keep the schema it is written to beside you:

```sh
mkdir -p docs/tdd && cd docs/tdd
vitrify new tdd calc
vitrify schema show --tdd
```

`vitrify new tdd calc` materializes `calc.md` from the embedded template, refusing to overwrite an existing file. Fill §1–§6 with the interview's answers: problem, goals, non-goals; a component diagram with **every cross-component edge named** (each edge becomes exactly one SDD contract); exact data schemas and algorithms; cross-cutting constraints with numeric thresholds; rollout ordering; named rejected alternatives. Drive it to **Approved** with a pinnable revision identifier.

**Gate:** the SDD-readiness checklist at the end of the TDD schema, line by line. A TDD that fails it stalls SDD authoring with open questions — sharpen the failing lines rather than carrying them forward.

**Worked reference:** [`fixtures/example/tdd/calc.md`](../../fixtures/example/tdd/calc.md).

## 3. Author the SDD

The SDD is the buildable spec — the what and exactly how. Make it decision-complete before any unit goes Ready.

### Create the SDD

Create SDDs inside `specs/`, where discovery looks first (`specs/*/sdd.md`, then `specs/*.sdd.md`):

```sh
mkdir -p specs && cd specs
vitrify new sdd calc
vitrify schema show
```

`vitrify new sdd calc` creates a bundle, `calc/`, with `sdd.md`, `units/U-001-foundation.md`, `contracts/C-001-example.md`, `changelog.md`, and `decisions.md`. For an SDD of about eight units or fewer that will be built one unit at a time, `vitrify new sdd --single calc` creates one file, `calc.sdd.md`, instead. The bundle shape is what lets parallel workers claim and stamp units without touching the same file.

### Make It Decision-Complete

Work through these rules; each one maps to a check that runs later.

- **Pin Technology Decisions exactly** — language, runtime, every dependency version — and write the four §5 Commands (Bootstrap, Build, Test, Lint). Bootstrap, Build, and Lint must run clean from the repository root before anything is built, or the build refuses to start. Test is the whole suite: it need not pass before the build, but once it passes on mainline no integration may turn it red, and the build is Complete only when it passes — so make it a command a correct build keeps green (tests whose subject is not built yet skip there), or write `none`.
- **Make unit ownership exclusive and exhaustive** over `scope_roots`, and freeze every cross-unit surface in exactly one contract before its consumers go Ready.
- **Give every unit** EARS requirements, a decision-complete Implementation block (exact paths, signatures, algorithm steps, prohibitions), and a Verification table mapping every requirement to an exact command. A sentence that leaves the implementer a choice is a defect — decide it, or open a `Q-###` and stop rather than guess.
- **Keep every unit buildable from its reading set alone** — Ground Rules, Technology Decisions, its own entry, its consumed contracts, nothing else. Check sizes with `vitrify readset --unit U-### --budget`. The budget counts what you wrote — not the claim, stamp, and trace fields vitrify fills in later — so building a unit never pushes it over.
- **Order execution foundation-first**, with `[P]` groups of mutually independent units.
- **Record every consequential decision** as a citation of `TDD §n` or a `D-###` entry in the Decision Log.

If the repository holds or will hold other SDDs, give this one its own `sdd:` slug and disjoint `scope_roots`, and consume other SDDs' surfaces through external contracts — see [Work With Several SDDs](multiple-sdds.md).

**Worked reference:** [`fixtures/example/specs/calc/`](../../fixtures/example/specs/calc/sdd.md).

## 4. Decide the Test Posture

vitrify takes no position on tests. Decide, per unit, how it is verified — whether tests exist before the build, whether its Verification runs them, and whether building the unit includes writing them — and encode the choice in the spec. Ownership and Verification tables are the whole mechanism; there is no posture setting. [Test Posture](../about/concepts/test-posture.md) lists the postures and the rules they share.

1. Choose a posture for each unit; postures mix across units.
2. State the posture in Ground Rules, so every worker reads it.
3. Make absence fail: a Verification row that runs tests should fail when the code it judges is missing, while the §5 Test command stays green by skipping those tests. calc does this with `SDD_REQUIRE_IMPL=1` in every generated unit's Verification rows.
4. Give the tests to an owner: the unit that writes them, a test unit the implementation units depend on, a Verified baseline unit, or nobody (outside `scope_roots`).
5. If you write tests now and want them locked, ship their unit as an authored baseline (below).

### Ship an Authored Baseline

To ship tests written now, stamp their unit with vitrify's own state verbs — never by hand — so that its trace, journal, and ledger agree. This is the exact sequence that produced calc's Verified U-001:

```sh
vitrify journal append --event change  --entry CH-001 --actor author --commit 0000000
vitrify journal append --event claim   --unit U-001 --actor author --claim-seq 1 --commit 0000000
vitrify trace record --unit U-001 --stamp built    --commit 0000000
vitrify journal append --event built   --unit U-001 --actor author --claim-seq 1 --commit 0000000
vitrify trace record --unit U-001 --stamp verified --commit 0000000
vitrify journal append --event verified --unit U-001 --actor author --claim-seq 1 --commit 0000000
vitrify ledger regen
```

`--commit` sets the commit each stamp and event records; it defaults to the short `HEAD`, and calc records the placeholder `0000000` by its own decision D-002. These verbs edit the working tree only: commit the result, then confirm that `vitrify journal replay` and `vitrify ledger regen --check` both exit 0.

## 5. Lint and Go Active

Run the authoring checks until they pass:

```sh
vitrify lint
```

`lint` runs the eleven mechanical checks, L1–L11, and prints `lint clean (L1–L11)` with exit 0 when the SDD passes, or one `[L#] subject: cause → remediation` line per finding with exit 2. If you shipped an authored baseline, use `vitrify lint --built`: plain lint's L7 reports any build state, and a baseline is build state. The [Troubleshooting Guide](../resources/troubleshooting.md#vitrify-lint-reports-findings) maps each check to its usual fix.

Then set `status: Active` in the SDD's front matter and commit everything at the mainline head.

## 6. Run the Preflight

Run the same preflight that `vitrify run` and kickoff will enforce:

```sh
vitrify check
vitrify locate --units
vitrify run --dry-run
```

`vitrify check` runs lint, the audit, the journal replay where a journal exists, the ledger check, and the pin states in one call, and exits 0 when all are clean. `vitrify run --dry-run` runs the whole preflight — including the SDD's §5 Bootstrap, Build, and Lint commands from the repository root — prints the plan, and ends with the `Start:` line that starts the build. The Test command does not gate the start.

## Check the Result

The spec is ready to build when every check below passes. Use this table as the acceptance list.

| Check | Evidence |
| --- | --- |
| The TDD is Approved | its metadata names `Approved` and the revision the SDD pins |
| Lint is clean | `vitrify lint` (or `--built` with a baseline) prints `lint clean (L1–L11)` and exits 0 |
| The repository is consistent | `vitrify check` prints `check clean: 1 SDD` and exits 0 |
| Phase 1 is claimable | `vitrify locate --units` marks exactly the Phase-1 units `claimable` |
| The preflight passes | `vitrify run --dry-run` exits 0 and ends with its `Start:` line |
| The SDD is committed | `status: Active`, committed at the mainline head |

## Common Issues

- **L5: a reading set is over budget.** Split the unit, trim what every unit reads (Ground Rules and Technology Decisions count against every budget), or consume fewer contracts.
- **L2: a file is neither owned nor `unmanaged`.** Add it to one unit's `owns` globs or list it under `unmanaged:`; files git ignores never count.
- **L4: a `Consumers:` line disagrees with `consumes`.** Make the line name exactly the units that consume the contract.
- **L8: `OPEN(Q-###)` in a Ready unit.** Settle the question and remove the marker before the unit is Ready, and make sure every marker's question has a Decision Log entry.
- **The PRD needs an answer nobody has.** Record it as an open question in the TDD rather than guess, and leave it to the human to settle before approving the TDD.
- **L10 or L11 findings.** Make `version` and `updated` match the last change entry, and pin the current revision of an Approved TDD.
- **A §5 command fails.** Run it exactly as written from the repository root; commands must be non-interactive and exit 0 in every consistent state.

## Next Steps

- [Run a Build](run-a-build.md) — build the Active SDD with `vitrify run` or an interactive kickoff.
- [Monitor a Run](monitor-a-run.md) — watch the build while it works.
- [Command Reference](../cli/command-reference.md#vitrify-lint) — every lint and audit check.
