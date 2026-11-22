//! Incremental builds against a real repository: which Verified units a
//! change reaches, which rows a gate or a re-check runs, the rule that a
//! gate blames a batch only for failures it caused, adoption of built
//! attempts, and culprit finding. The pure decisions live in
//! `vitrify_core::cascade` and `vitrify_core::facf`; this module runs git
//! and the commands.

pub mod adopt;
pub mod blame;
pub mod culprit;
pub mod dirty;
pub mod recheck;
pub mod rows;
