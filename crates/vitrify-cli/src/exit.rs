//! The exit-code table. Every CLI error path maps through
//! this enum; `main` is the only place that converts to a process exit.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum ExitCode {
    /// Asserted state holds (recorded / clean / listed / published).
    Ok = 0,
    /// Unexpected error (git failure, I/O).
    Error = 1,
    /// Usage error, or a named check failed (audit/lint findings, an init or
    /// probe FAIL, probe changes not applied).
    Usage = 2,
    /// Trace or projection mismatch (keys/units named on stdout).
    Mismatch = 3,
    /// Not found / precondition failed.
    NotFound = 4,
    /// Lost publish race after retries.
    LostRace = 5,
    /// Lease already expired (renew refused — re-claim instead).
    LeaseExpired = 6,
}

impl ExitCode {
    pub fn from_code(code: u8) -> ExitCode {
        match code {
            0 => ExitCode::Ok,
            2 => ExitCode::Usage,
            3 => ExitCode::Mismatch,
            4 => ExitCode::NotFound,
            5 => ExitCode::LostRace,
            6 => ExitCode::LeaseExpired,
            _ => ExitCode::Error,
        }
    }
}
