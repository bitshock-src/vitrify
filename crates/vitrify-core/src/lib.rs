//! vitrify-core: pure functions over SDD/TDD instance files — parsing, masking,
//! trace hashing, journal append/replay, ledger derivation, audit checks,
//! claim-transaction planning, and the write-set law. No process management;
//! no printing; no git.
//!
//! Regression goldens: the golden corpus under `fixtures/golden/` and the
//! example instance under `fixtures/example/` hold the trace recipe, the
//! journal and ledger formats and the reading of `schema_version: 2.0`, byte
//! for byte. A change to the document format or the trace recipe raises
//! `schema_version` and regenerates the corpus in the same change; any other
//! drift from it is a bug.

pub mod audit;
pub mod changelog;
pub mod claims;
pub mod config;
pub mod content;
pub mod contract;
pub mod harness;
pub mod journal;
pub mod ledger;
pub mod model;
pub mod newline;
pub mod questions;
pub mod readset;
pub mod tdd;
pub mod trace;
pub mod writeset;
pub mod yamlish;

use std::path::PathBuf;

/// Core error: typed I/O and UTF-8 failures (exit-1 class), or a `Refusal`
/// carrying an exit code from the exit-code table with the exact diagnostic text
/// the CLI prints to stderr (including its `error: ` prefix).
#[derive(Debug, thiserror::Error)]
pub enum CoreError {
    #[error("error: {path}: {source}")]
    Io {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("error: invalid UTF-8 in {path}")]
    Utf8 { path: PathBuf },
    #[error("{message}")]
    Refusal { code: u8, message: String },
}

impl CoreError {
    pub fn refusal(code: u8, message: impl Into<String>) -> CoreError {
        CoreError::Refusal {
            code,
            message: message.into(),
        }
    }

    /// The exit code this error maps to.
    pub fn exit_code(&self) -> u8 {
        match self {
            CoreError::Refusal { code, .. } => *code,
            _ => 1,
        }
    }
}
