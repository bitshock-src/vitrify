//! vitrify library target: exposes the CLI internals so integration
//! tests can exercise the git layer and command modules directly. The
//! `vitrify` binary is a thin wrapper over [`run`].

pub mod cli;
pub mod cmd;
pub mod exit;
pub mod gate;
pub mod git;
pub mod help;
pub mod home;
pub mod layout;
pub mod out;
pub mod run;
pub mod signals;

use clap::Parser;
use vitrify_core::CoreError;

use crate::cli::{Cli, Cmd};

/// Parse argv and dispatch; returns the exit code.
pub fn run() -> u8 {
    let cli = Cli::parse();
    match dispatch(cli) {
        Ok(code) => code,
        Err(err) => {
            out::diag(&err.to_string());
            err.exit_code()
        }
    }
}

fn dispatch(cli: Cli) -> Result<u8, CoreError> {
    // Every command but `init` needs the home `init` sets up, and refuses —
    // the same way, naming `vitrify init` — on a machine without one. Each
    // reads the configuration the same way too, so a file that does not
    // parse, a value of the wrong type, a schema this vitrify does not read,
    // or a repository's config.toml carrying the probe record stops every
    // command alike, whether or not it reads a setting.
    if !matches!(cli.cmd, Cmd::Init { .. }) {
        let Some(home_dir) = cmd::home_dir() else {
            return Err(CoreError::refusal(
                1,
                "error: cannot resolve the vitrify home (set VITRIFY_HOME or HOME)",
            ));
        };
        home::require_set_up(&home_dir)?;
        cmd::load_config(&vitrify_core::config::Overrides::default())?;
    }
    match cli.cmd {
        Cmd::Locate { units, sdd, json } => cmd::locate::run(units, sdd.as_deref(), json),
        Cmd::Readset {
            unit,
            sdd,
            json,
            budget,
        } => cmd::readset::run(&unit, sdd.as_deref(), json, budget),
        Cmd::Claim(args) => cmd::claim::run(vitrify_core::claims::Op::Claim, &args),
        Cmd::Renew(args) => cmd::claim::run(vitrify_core::claims::Op::Renew, &args),
        Cmd::Release(args) => cmd::claim::run(vitrify_core::claims::Op::Release, &args),
        Cmd::Trace { mode } => match mode {
            cli::TraceMode::Record {
                unit,
                stamp,
                commit,
                sdd,
            } => cmd::trace::record(&unit, stamp, commit.as_deref(), sdd.as_deref()),
            cli::TraceMode::Verify { unit, sdd, json } => {
                cmd::trace::verify(&unit, sdd.as_deref(), json)
            }
        },
        Cmd::Ledger { mode } => match mode {
            cli::LedgerMode::Regen { sdd, all, check } => {
                cmd::ledger::regen(sdd.as_deref(), all, check)
            }
            cli::LedgerMode::Progress { sdd, all } => cmd::ledger::progress(sdd.as_deref(), all),
        },
        Cmd::Journal { mode } => match mode {
            cli::JournalMode::Append {
                event,
                unit,
                actor,
                claim_seq,
                entry,
                stale,
                cause,
                findings,
                commit,
                sdd,
            } => cmd::journal::append(
                event,
                cmd::journal::AppendArgs {
                    unit,
                    actor,
                    claim_seq,
                    entry,
                    stale,
                    cause,
                    findings,
                    commit,
                    sdd,
                },
            ),
            cli::JournalMode::Replay { json, sdd, all } => {
                cmd::journal::replay(json, sdd.as_deref(), all)
            }
        },
        Cmd::Audit {
            json,
            fix_leases,
            apply_stale,
            skip_rotten,
            journal,
            sdd,
            all,
        } => cmd::audit::run(cmd::audit::Flags {
            json,
            fix_leases,
            apply_stale,
            skip_rotten,
            journal,
            sdd,
            all,
        }),
        Cmd::Brief {
            phase,
            unit,
            holder,
            sdd,
        } => cmd::brief::run(phase, unit.as_deref(), holder.as_deref(), sdd.as_deref()),
        Cmd::New {
            kind,
            bundle: _,
            single,
            name,
        } => cmd::new::run(kind, single, name.as_deref()),
        Cmd::Schema { mode } => match mode {
            cli::SchemaMode::Show { tdd } => cmd::schema::show(tdd),
        },
        Cmd::Init { json } => cmd::init::run(json),
        Cmd::Probe { json, assume_yes } => cmd::probe::run(json, assume_yes),
        Cmd::Lint {
            sdd,
            all,
            built,
            json,
        } => cmd::lint::run(cmd::lint::Flags {
            sdd,
            all,
            built,
            json,
        }),
        Cmd::Check {
            sdd,
            all,
            skip_rotten,
            json,
        } => cmd::check::run(cmd::check::Flags {
            sdd,
            all,
            skip_rotten,
            json,
        }),
        Cmd::Change { mode } => match mode {
            cli::ChangeMode::Apply {
                sdd,
                all,
                dry_run,
                commit,
                json,
            } => cmd::change::apply(cmd::change::ApplyFlags {
                sdd,
                all,
                dry_run,
                commit,
                json,
            }),
        },
        Cmd::Pin { mode } => match mode {
            cli::PinMode::List { sdd, all, json } => cmd::pin::list(sdd.as_deref(), all, json),
            cli::PinMode::Sync {
                sdd,
                all,
                dry_run,
                json,
            } => cmd::pin::sync(sdd.as_deref(), all, dry_run, json),
        },
        Cmd::Questions {
            sdd,
            all,
            open,
            json,
        } => cmd::questions::run(sdd.as_deref(), all, open, json),
        Cmd::Run {
            sub: Some(cli::RunSub::Status(args)),
            ..
        } => cmd::run_status::run(&args),
        Cmd::Run {
            dry_run,
            cap,
            gate_jobs,
            wait,
            sdd,
            mainline,
            sub: None,
        } => cmd::run::run(run::RunFlags {
            dry_run,
            cap,
            sdd,
            mainline,
            gate_jobs,
            wait,
        }),
    }
}
