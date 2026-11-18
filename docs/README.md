# vitrify Documentation

> Find every vitrify guide, concept, and reference page, organized by what you want to do.

Use this index to find a page. These pages are for people, and for the reasons behind the design: a coding agent learns vitrify from `vitrify --help` and the procedures `vitrify brief` lists. If you are new, start with the [Quick Start](getting-started/quick-start.md), then the [vitrify Primer](about/primer.md); the guides are usable without reading the concept pages, which go deeper when you need them. The product overview is the repository's [README](../README.md).

## Start Here

These two pages get you from nothing to a working mental model.

- [Quick Start](getting-started/quick-start.md) covers setting up your machine, the prompts that have your coding agent spec a new project or a feature for an existing one, and building it with `vitrify run`.
- [vitrify Primer](about/primer.md) covers the five-minute model: what vitrify adds, one build seen from both sides, and what it leaves to you.

## About vitrify

These pages explain what vitrify is and how it works.

- [vitrify Primer](about/primer.md) covers the minimum model needed to read the rest of the documentation.
- [Specs: TDD and SDD](about/concepts/specs.md) covers the two documents, their regions, shapes, schema version, and lifecycle.
- [Work Units](about/concepts/units.md) covers ownership, dependencies, the reading set, and the unit lifecycle.
- [Contracts](about/concepts/contracts.md) covers frozen cross-unit surfaces and the external contracts that cross SDDs.
- [Test Posture](about/concepts/test-posture.md) covers how each unit is verified, and the rules every posture shares.
- [Gates](about/concepts/gates.md) covers lint, preflight, the unit and integration gates, and the audit.
- [Build State](about/concepts/build-state.md) covers claims, leases, traces, the journal, and the ledger.
- [Runs](about/concepts/runs.md) covers what `vitrify run` does from preflight to its final report.
- [Architecture](about/architecture.md) covers the components, where state lives, and the managed execution pipeline.
- [Design Rationale](about/design-rationale.md) covers the design positions, goals, schema grounding, thresholds, and rejected alternatives.
- [Release Notes](about/release-notes.md) covers briefly what changed in each release.

## Getting Started

These pages install vitrify and prove it works.

- [Installation](getting-started/installation.md) covers requirements, installing, setting the machine up with `vitrify init`, preparing a harness CLI, upgrading, and uninstalling.
- [Quick Start](getting-started/quick-start.md) covers the one-time machine setup, the agent prompts for a new project, a feature, a change, and a build, and verifying the result.

## User Guides

These guides take you through each task, step by step. Each opens with the prompts that hand the task to your coding agent.

- [Author a Spec](user-guides/author-a-spec.md) covers turning a PRD into the TDD and SDD, choosing a test posture, and passing lint.
- [Run a Build](user-guides/run-a-build.md) covers starting a build, building interactively, resolving a Blocked result, and resuming; `vitrify brief run` is the agent's procedure.
- [Monitor a Run](user-guides/monitor-a-run.md) covers everything a run prints and records, how to read it while it works, and the final report it keeps.
- [Change a Spec](user-guides/change-a-spec.md) covers change entries, `vitrify change apply`, `vitrify check`, and rebuilding only what a change reaches; `vitrify brief change` is the agent's procedure.
- [Work With Several SDDs](user-guides/multiple-sdds.md) covers repositories with several SDDs, and the pins and transcriptions between them.

## CLI

These pages configure and look up the `vitrify` command.

- [Harness CLIs](cli/harnesses.md) covers probing, role profiles, permissions, and the notes for Claude Code, Codex, and OpenCode.
- [Configuration](cli/configuration.md) covers `~/.vitrify/config.toml` — every setting and its default — environment variables, and content overrides.
- [Command Reference](cli/command-reference.md) covers every command, its options, its output, and the exit codes, with the reasons and cross-links `--help` leaves out.

## Contribute

This page is for people changing vitrify itself.

- [Develop vitrify](contribute/development.md) covers the toolchain, the conventions, the golden corpus, the two test tiers, and publishing a release to crates.io.

## Reference

These pages specify exact formats and support claims.

- [Data Model and Formats Specification](reference/formats.md) covers the instance model, trace hash recipe, journal, ledger, external contracts, and versioning.
- [Support Matrix](reference/support-matrix.md) covers supported platforms, harness CLIs, the schema version, and dependencies.
- [Sources](reference/sources.md) covers the papers, documentation, and web pages behind the design's citations.
- The embedded [SDD schema](../crates/vitrify-core/assets/schemas/sdd-schema.md) and [TDD schema](../crates/vitrify-core/assets/schemas/tdd-schema.md) are the normative documents that `vitrify schema show` prints and every spec is written to.

## Resources

These pages help when you are stuck.

- [Support and FAQs](resources/faq.md) covers common questions about positioning, builds, tests, data, and help.
- [Glossary](resources/glossary.md) covers every term the documentation uses.
- [Troubleshooting Guide](resources/troubleshooting.md) covers symptoms, their causes, and their fixes.
