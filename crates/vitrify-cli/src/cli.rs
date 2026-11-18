//! The complete command tree, via clap derive.
//! Non-interactive always; stdout data, stderr diagnostics.
//! Every argument carries help text; each command's `--help` adds its block
//! from [`help::verbs`], and the root help is [`help::root`].
//! `tests/cli_help.rs` holds all three.

use clap::{Args, Parser, Subcommand, ValueEnum};

use crate::help;

#[derive(Parser)]
#[command(
    name = "vitrify",
    version,
    about = help::root::ABOUT,
    help_template = help::root::TEMPLATE,
    before_help = help::root::BEFORE_SHORT,
    before_long_help = help::root::BEFORE_LONG,
    after_help = help::root::AFTER_SHORT,
    after_long_help = help::root::AFTER_LONG
)]
pub struct Cli {
    #[command(subcommand)]
    pub cmd: Cmd,
}

#[derive(Subcommand)]
pub enum Cmd {
    /// Find SDD instances; list units and claimability
    #[command(after_long_help = help::verbs::LOCATE)]
    Locate {
        /// List units of one SDD with the computed claimable flag
        #[arg(long)]
        units: bool,
        /// SDD root: a bundle directory, its sdd.md, or a single *.sdd.md
        /// (default: every SDD; with --units, the one SDD in the repository)
        #[arg(long, value_name = "ROOT")]
        sdd: Option<String>,
        /// Print JSON on stdout instead of text
        #[arg(long)]
        json: bool,
    },
    /// Print the ordered minimal reading set for one unit
    #[command(after_long_help = help::verbs::READSET)]
    Readset {
        /// Unit ID, e.g. U-012
        #[arg(long)]
        unit: String,
        /// SDD root: a bundle directory, its sdd.md, or a single *.sdd.md
        /// (default: the one SDD in the repository)
        #[arg(long, value_name = "ROOT")]
        sdd: Option<String>,
        /// Print JSON on stdout instead of text
        #[arg(long)]
        json: bool,
        /// Total bytes and fail over budget (default: lint.readset_budget_bytes)
        #[arg(long, num_args = 0..=1, value_name = "BYTES")]
        budget: Option<Option<i64>>,
    },
    /// Claim a unit lease (atomic mainline transaction)
    #[command(after_long_help = help::verbs::CLAIM)]
    Claim(ClaimArgs),
    /// Renew a held lease
    #[command(after_long_help = help::verbs::RENEW)]
    Renew(ClaimArgs),
    /// Release a held lease
    #[command(after_long_help = help::verbs::RELEASE)]
    Release(ClaimArgs),
    /// Record or verify a unit's input-trace hashes
    #[command(after_long_help = help::verbs::TRACE)]
    Trace {
        #[command(subcommand)]
        mode: TraceMode,
    },
    /// Regenerate the derived ledger or print the progress line
    #[command(after_long_help = help::verbs::LEDGER)]
    Ledger {
        #[command(subcommand)]
        mode: LedgerMode,
    },
    /// Append journal events; replay and verify the journal
    #[command(after_long_help = help::verbs::JOURNAL)]
    Journal {
        #[command(subcommand)]
        mode: JournalMode,
    },
    /// The mechanical drift audit (seven checks)
    #[command(after_long_help = help::verbs::AUDIT)]
    Audit {
        /// Print JSON on stdout instead of text
        #[arg(long)]
        json: bool,
        /// Return lapsed Claimed units to Ready (release event, actor audit)
        #[arg(long)]
        fix_leases: bool,
        /// Write Stale marks for check 4–5 findings
        #[arg(long)]
        apply_stale: bool,
        /// Skip re-running Verified units' Verification tables
        #[arg(long)]
        skip_rotten: bool,
        /// Append one `audit` journal event
        #[arg(long)]
        journal: bool,
        /// SDD root: a bundle directory, its sdd.md, or a single *.sdd.md
        /// (default: the one SDD in the repository)
        #[arg(long, value_name = "ROOT")]
        sdd: Option<String>,
        /// Audit every SDD of the repository
        #[arg(long, conflicts_with = "sdd")]
        all: bool,
    },
    /// Print the procedure index, or the operative procedure for a task
    #[command(after_long_help = help::verbs::BRIEF)]
    Brief {
        /// The procedure to print (none: the index)
        #[arg(value_enum, value_name = "TASK")]
        phase: Option<BriefPhase>,
        /// Unit ID (build-unit worker-prompt generation)
        #[arg(long)]
        unit: Option<String>,
        /// Holder identity (build-unit worker-prompt generation)
        #[arg(long)]
        holder: Option<String>,
        /// SDD root for the worker prompt: a bundle directory, its sdd.md,
        /// or a single *.sdd.md (default: the one SDD in the repository)
        #[arg(long, value_name = "ROOT")]
        sdd: Option<String>,
    },
    /// Set this machine up: ~/.vitrify/config.toml with every default, then a first probe
    #[command(after_long_help = help::verbs::INIT)]
    Init {
        /// Print JSON on stdout instead of text
        #[arg(long)]
        json: bool,
    },
    /// Probe git and the harness CLIs again; update ~/.vitrify/config.toml after asking
    #[command(after_long_help = help::verbs::PROBE)]
    Probe {
        /// Print JSON on stdout instead of text
        #[arg(long)]
        json: bool,
        /// Apply the changes without asking
        #[arg(short = 'y', long)]
        assume_yes: bool,
    },
    /// The deterministic orchestrator: run a complete generation pass
    #[command(
        args_conflicts_with_subcommands = true,
        after_long_help = help::verbs::RUN
    )]
    Run {
        /// Print preflight results and the plan, then stop
        #[arg(long)]
        dry_run: bool,
        /// Override run.worker_cap: workers at once
        #[arg(long, value_name = "N")]
        cap: Option<i64>,
        /// Override run.gate_jobs: Verification rows run at once (default
        /// 0 = one per CPU core; 1 = one at a time)
        #[arg(long, value_name = "N")]
        gate_jobs: Option<i64>,
        /// When only blocked units remain, keep running and resume when
        /// the spec changes, instead of ending Blocked
        #[arg(long)]
        wait: bool,
        /// SDD root to build: a bundle directory, its sdd.md, or a single
        /// *.sdd.md (default: the one SDD in the repository)
        #[arg(long, value_name = "ROOT")]
        sdd: Option<String>,
        /// Mainline branch the run publishes to (default: config mainline,
        /// else origin/HEAD, else the primary checkout's branch)
        #[arg(long, value_name = "BRANCH")]
        mainline: Option<String>,
        #[command(subcommand)]
        sub: Option<RunSub>,
    },
    /// Mechanical authoring checks (L1–L11)
    #[command(after_long_help = help::verbs::LINT)]
    Lint {
        /// SDD root: a bundle directory, its sdd.md, or a single *.sdd.md
        /// (default: the one SDD in the repository)
        #[arg(long, value_name = "ROOT")]
        sdd: Option<String>,
        /// Lint every SDD of the repository
        #[arg(long, conflicts_with = "sdd")]
        all: bool,
        /// The SDD is being built, or has been: leave out L7 (state that
        /// must not exist before the build) — what `run` preflights with
        #[arg(long)]
        built: bool,
        /// Print JSON on stdout instead of text
        #[arg(long)]
        json: bool,
    },
    /// Every consistency check vitrify has, over one SDD or all of them
    #[command(after_long_help = help::verbs::CHECK)]
    Check {
        /// SDD root: a bundle directory, its sdd.md, or a single *.sdd.md
        /// (default: the one SDD in the repository)
        #[arg(long, value_name = "ROOT")]
        sdd: Option<String>,
        /// Check every SDD of the repository
        #[arg(long, conflicts_with = "sdd")]
        all: bool,
        /// Skip re-running Verified units' Verification tables (audit check 4)
        #[arg(long)]
        skip_rotten: bool,
        /// Print JSON on stdout instead of text
        #[arg(long)]
        json: bool,
    },
    /// Record a change entry's bookkeeping: journal events, Stale marks or
    /// re-recorded traces, the ledger
    #[command(after_long_help = help::verbs::CHANGE)]
    Change {
        #[command(subcommand)]
        mode: ChangeMode,
    },
    /// External contracts: one SDD's pins on another's contracts, and its
    /// transcriptions
    #[command(after_long_help = help::verbs::PIN)]
    Pin {
        #[command(subcommand)]
        mode: PinMode,
    },
    /// List the open questions (Q-###) and the units they block
    #[command(after_long_help = help::verbs::QUESTIONS)]
    Questions {
        /// SDD root: a bundle directory, its sdd.md, or a single *.sdd.md
        /// (default: the one SDD in the repository)
        #[arg(long, value_name = "ROOT")]
        sdd: Option<String>,
        /// Every SDD of the repository
        #[arg(long, conflicts_with = "sdd")]
        all: bool,
        /// Only the questions not marked resolved
        #[arg(long)]
        open: bool,
        /// Print JSON on stdout instead of text
        #[arg(long)]
        json: bool,
    },
    /// Materialize instance templates into the current directory
    #[command(after_long_help = help::verbs::NEW)]
    New {
        /// The document to materialize
        #[arg(value_enum)]
        kind: NewKind,
        /// Bundle shape (SDD only; default)
        #[arg(long, conflicts_with = "single")]
        bundle: bool,
        /// Single-file shape (SDD only)
        #[arg(long)]
        single: bool,
        /// Instance name (slug)
        name: Option<String>,
    },
    /// Print the embedded normative schema documents
    #[command(after_long_help = help::verbs::SCHEMA)]
    Schema {
        #[command(subcommand)]
        mode: SchemaMode,
    },
}

/// What `vitrify run` does besides running: looking at a run from outside.
#[derive(Subcommand)]
pub enum RunSub {
    /// Show what a run is doing, unit by unit (read-only)
    #[command(after_long_help = help::verbs::RUN_STATUS)]
    Status(StatusArgs),
}

/// `vitrify run status`: read-only, so it can be pointed at a live run from
/// a second terminal without disturbing it.
#[derive(Args, Clone)]
pub struct StatusArgs {
    /// Keep watching: one line per change, until the run ends
    #[arg(long)]
    pub follow: bool,
    /// JSON: one object; with --follow, one event per line
    #[arg(long)]
    pub json: bool,
    /// With --follow: first replay what the run recorded since it started
    #[arg(long, requires = "follow")]
    pub since_start: bool,
    /// The run to show, by job ID (default: the latest)
    #[arg(long, value_name = "JOB")]
    pub job: Option<String>,
    /// Seconds between looks while following
    #[arg(long, default_value_t = 2, value_parser = clap::value_parser!(u64).range(1..))]
    pub interval: u64,
    /// Seconds without worker output before a worker is flagged quiet
    #[arg(long, default_value_t = 300, value_parser = clap::value_parser!(u64).range(1..))]
    pub quiet_after: u64,
}

#[derive(Args, Clone)]
pub struct ClaimArgs {
    /// Unit ID, e.g. U-012
    #[arg(long)]
    pub unit: String,
    /// Lease holder identity
    #[arg(long, value_name = "ID")]
    pub holder: String,
    /// Lease length in minutes
    #[arg(long, default_value_t = 60, value_name = "N")]
    pub lease_minutes: i64,
    /// SDD root, as a path from where the verb runs (the primary checkout
    /// or any worktree of it); default: the one SDD in the repository
    #[arg(long, value_name = "ROOT")]
    pub sdd: Option<String>,
    /// Mainline branch the lease commit is published to (default: config
    /// mainline, else origin/HEAD, else the primary checkout's branch)
    #[arg(long, value_name = "BRANCH")]
    pub mainline: Option<String>,
}

#[derive(Subcommand)]
pub enum TraceMode {
    /// Compute and write the trace map into unit front matter, and stamp
    /// the unit Built or Verified
    #[command(after_long_help = help::verbs::TRACE_RECORD)]
    Record {
        /// Unit ID, e.g. U-012
        #[arg(long)]
        unit: String,
        /// The status to set, with its stamp
        #[arg(long, value_enum)]
        stamp: StampKind,
        /// Short SHA for the stamp (default: HEAD)
        #[arg(long, value_name = "SHA")]
        commit: Option<String>,
        /// SDD root: a bundle directory, its sdd.md, or a single *.sdd.md
        /// (default: the one SDD in the repository)
        #[arg(long, value_name = "ROOT")]
        sdd: Option<String>,
    },
    /// Compare recorded hashes against current content
    #[command(after_long_help = help::verbs::TRACE_VERIFY)]
    Verify {
        /// Unit ID, e.g. U-012
        #[arg(long)]
        unit: String,
        /// SDD root: a bundle directory, its sdd.md, or a single *.sdd.md
        /// (default: the one SDD in the repository)
        #[arg(long, value_name = "ROOT")]
        sdd: Option<String>,
        /// Print JSON on stdout instead of text
        #[arg(long)]
        json: bool,
    },
}

/// The status `trace record --stamp` sets.
#[derive(Clone, Copy, ValueEnum)]
pub enum StampKind {
    /// Status Built; writes the `built` stamp (the worker's last step)
    Built,
    /// Status Verified; writes the `verified` stamp (the integrator, after the gate)
    Verified,
}

#[derive(Subcommand)]
pub enum LedgerMode {
    /// Rewrite the derived ledger (never hand-edit it)
    #[command(after_long_help = help::verbs::LEDGER_REGEN)]
    Regen {
        /// SDD root: a bundle directory, its sdd.md, or a single *.sdd.md
        /// (default: the one SDD in the repository)
        #[arg(long, value_name = "ROOT")]
        sdd: Option<String>,
        /// Every SDD of the repository
        #[arg(long, conflicts_with = "sdd")]
        all: bool,
        /// Write nothing: compare the ledger on disk with a fresh
        /// derivation; exit 3 when they differ
        #[arg(long)]
        check: bool,
    },
    /// Print the one-line progress summary
    #[command(after_long_help = help::verbs::LEDGER_PROGRESS)]
    Progress {
        /// SDD root: a bundle directory, its sdd.md, or a single *.sdd.md
        /// (default: the one SDD in the repository)
        #[arg(long, value_name = "ROOT")]
        sdd: Option<String>,
        /// Every SDD of the repository
        #[arg(long, conflicts_with = "sdd")]
        all: bool,
    },
}

#[derive(Subcommand)]
pub enum ChangeMode {
    /// Apply the last change entry of each Active SDD: the `change` event,
    /// then Stale marks (MINOR, MAJOR) or re-recorded traces (PATCH), then
    /// the ledger (working tree only)
    #[command(after_long_help = help::verbs::CHANGE_APPLY)]
    Apply {
        /// SDD root: a bundle directory, its sdd.md, or a single *.sdd.md
        /// (default: the one SDD in the repository)
        #[arg(long, value_name = "ROOT")]
        sdd: Option<String>,
        /// Every SDD of the repository
        #[arg(long, conflicts_with = "sdd")]
        all: bool,
        /// Print what would be done; write nothing
        #[arg(long)]
        dry_run: bool,
        /// Code commit to record on the events
        #[arg(long, value_name = "SHA")]
        commit: Option<String>,
        /// Print JSON on stdout instead of text
        #[arg(long)]
        json: bool,
    },
}

#[derive(Subcommand)]
pub enum PinMode {
    /// List every external contract with its authority and state
    #[command(after_long_help = help::verbs::PIN_LIST)]
    List {
        /// SDD root: a bundle directory, its sdd.md, or a single *.sdd.md
        /// (default: the one SDD in the repository)
        #[arg(long, value_name = "ROOT")]
        sdd: Option<String>,
        /// Every SDD of the repository
        #[arg(long, conflicts_with = "sdd")]
        all: bool,
        /// Print JSON on stdout instead of text
        #[arg(long)]
        json: bool,
    },
    /// Rewrite stale pins from their authorities (working tree only);
    /// print the units whose trace that moved
    #[command(after_long_help = help::verbs::PIN_SYNC)]
    Sync {
        /// SDD root: a bundle directory, its sdd.md, or a single *.sdd.md
        /// (default: the one SDD in the repository)
        #[arg(long, value_name = "ROOT")]
        sdd: Option<String>,
        /// Every SDD of the repository
        #[arg(long, conflicts_with = "sdd")]
        all: bool,
        /// Print what would be synced, and the Staleness applied lines; write nothing
        #[arg(long)]
        dry_run: bool,
        /// Print JSON on stdout instead of text
        #[arg(long)]
        json: bool,
    },
}

#[derive(Subcommand)]
pub enum JournalMode {
    /// Append one event (inside mainline-bound commits only)
    #[command(after_long_help = help::verbs::JOURNAL_APPEND)]
    Append {
        /// The event kind
        #[arg(long, value_name = "KIND")]
        event: EventKind,
        /// Unit ID (every kind except change and audit)
        #[arg(long)]
        unit: Option<String>,
        /// Who acted: a lease holder's identity, or the role that acted
        #[arg(long)]
        actor: Option<String>,
        /// The claim number the event belongs to (claim, renew, release)
        #[arg(long, value_name = "N")]
        claim_seq: Option<i64>,
        /// Change entry id (CH-###)
        #[arg(long)]
        entry: Option<String>,
        /// Comma-separated unit ids (change event)
        #[arg(long)]
        stale: Option<String>,
        /// Cause text (stale event)
        #[arg(long)]
        cause: Option<String>,
        /// Finding count (audit event)
        #[arg(long, value_name = "N")]
        findings: Option<i64>,
        /// Relevant code commit
        #[arg(long, value_name = "SHA")]
        commit: Option<String>,
        /// SDD root: a bundle directory, its sdd.md, or a single *.sdd.md
        /// (default: the one SDD in the repository)
        #[arg(long, value_name = "ROOT")]
        sdd: Option<String>,
    },
    /// Replay from empty and compare the projection with front matter
    #[command(after_long_help = help::verbs::JOURNAL_REPLAY)]
    Replay {
        /// Print JSON on stdout instead of text
        #[arg(long)]
        json: bool,
        /// SDD root: a bundle directory, its sdd.md, or a single *.sdd.md
        /// (default: the one SDD in the repository)
        #[arg(long, value_name = "ROOT")]
        sdd: Option<String>,
        /// Every SDD of the repository
        #[arg(long, conflicts_with = "sdd")]
        all: bool,
    },
}

/// The journal event kinds `journal append --event` takes.
#[derive(Clone, Copy, ValueEnum, Debug)]
pub enum EventKind {
    /// A lease taken
    Claim,
    /// A lease extended
    Renew,
    /// A lease given back
    Release,
    /// A unit stamped Built
    Built,
    /// A unit stamped Verified
    Verified,
    /// A unit marked Stale (with --cause)
    Stale,
    /// A unit retired
    Retired,
    /// A change entry recorded (with --entry, and --stale for MINOR or MAJOR)
    Change,
    /// An audit run recorded (with --findings)
    Audit,
}

impl EventKind {
    pub fn as_str(self) -> &'static str {
        match self {
            EventKind::Claim => "claim",
            EventKind::Renew => "renew",
            EventKind::Release => "release",
            EventKind::Built => "built",
            EventKind::Verified => "verified",
            EventKind::Stale => "stale",
            EventKind::Retired => "retired",
            EventKind::Change => "change",
            EventKind::Audit => "audit",
        }
    }
}

/// The procedures `vitrify brief` prints, in index order.
#[derive(Clone, Copy, ValueEnum, Debug, PartialEq, Eq)]
pub enum BriefPhase {
    /// Take a repository from nothing to ready-to-build, or name the gaps
    Onboard,
    /// A PRD to an Approved TDD and an Active, lint-clean SDD
    Authoring,
    /// Change TDDs and SDDs, one or many, and record what each change stales
    Change,
    /// Start, watch and read a `vitrify run` build
    Run,
    /// Drive the build loop from a session instead of `vitrify run`
    Kickoff,
    /// Build one unit (--unit --holder prints a worker's whole prompt)
    #[value(name = "build-unit")]
    BuildUnit,
    /// Take one built unit through the gate to Verified
    Integrate,
}

impl BriefPhase {
    /// Every procedure, in index order.
    pub const ALL: [BriefPhase; 7] = [
        BriefPhase::Onboard,
        BriefPhase::Authoring,
        BriefPhase::Change,
        BriefPhase::Run,
        BriefPhase::Kickoff,
        BriefPhase::BuildUnit,
        BriefPhase::Integrate,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            BriefPhase::Onboard => "onboard",
            BriefPhase::Authoring => "authoring",
            BriefPhase::Change => "change",
            BriefPhase::Run => "run",
            BriefPhase::Kickoff => "kickoff",
            BriefPhase::BuildUnit => "build-unit",
            BriefPhase::Integrate => "integrate",
        }
    }
}

/// The documents `vitrify new` materializes.
#[derive(Clone, Copy, ValueEnum)]
pub enum NewKind {
    /// A TDD: <name>.md (default tdd.md) from the TDD template
    Tdd,
    /// An SDD: a bundle directory <name>/ (default scope/), or
    /// <name>.sdd.md with --single
    Sdd,
}

#[derive(Subcommand)]
pub enum SchemaMode {
    /// Print the embedded normative schema (SDD by default)
    #[command(after_long_help = help::verbs::SCHEMA_SHOW)]
    Show {
        /// Print the TDD schema instead
        #[arg(long)]
        tdd: bool,
    },
}
