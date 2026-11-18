//! `vitrify run` — thin CLI wrapper over the orchestrator (`crate::run`).

use vitrify_core::CoreError;

use crate::run::{self, RunFlags};

pub fn run(flags: RunFlags) -> Result<u8, CoreError> {
    run::run(flags)
}
