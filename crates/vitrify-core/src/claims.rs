//! Claim-transaction planning — pure: preconditions + edits +
//! journal event + commit message from an `Instance` loaded at the tx base.
//! Execution (worktree, commit, CAS/push publish) lives in the binary
//! (`cmd/claim.rs`); core never spawns git.

use crate::CoreError;
use crate::journal::Event;
use crate::model::{self, Instance, UtcTime};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Op {
    Claim,
    Renew,
    Release,
}

impl Op {
    pub fn as_str(self) -> &'static str {
        match self {
            Op::Claim => "claim",
            Op::Renew => "renew",
            Op::Release => "release",
        }
    }
}

/// The planned transaction: scalar front-matter edits, the journal event
/// (seq and commit filled by the executor), and the commit message.
#[derive(Debug, Clone)]
pub struct TxPlan {
    pub edits: Vec<(String, String)>,
    pub event: Event,
    pub commit_message: String,
    pub claim_seq: i64,
    pub claim_expires: Option<String>,
}

/// Re-check preconditions at this head and build the plan. Refusal codes are
/// the exit-code table's: claim on unclaimable → 4 first attempt / 5 after a lost
/// race; renew not-Claimed-by-holder → 4, lease expired → 6; release
/// not-Claimed-by-holder → 4.
///
/// Every `depends_on` entry must name a unit of the SDD; an entry that
/// names none (a pin) holds the claim back. [`plan_with`] resolves pins.
pub fn plan(
    inst_at_base: &Instance,
    op: Op,
    unit_id: &str,
    holder: &str,
    lease_minutes: i64,
    now: UtcTime,
    lost_race_before: bool,
) -> Result<TxPlan, CoreError> {
    plan_with(
        inst_at_base,
        op,
        unit_id,
        holder,
        lease_minutes,
        now,
        lost_race_before,
        &|_| None,
    )
}

/// [`plan`] with the claimable predicate extended across SDDs: `resolve`
/// answers for the `depends_on` entries that name no unit of the SDD
/// ([`model::unit_view_with`]), so a pin's entry is met once every unit
/// producing its authority is Built or Verified. A refusal names each unmet
/// unit behind such an entry.
#[allow(clippy::too_many_arguments)]
pub fn plan_with(
    inst_at_base: &Instance,
    op: Op,
    unit_id: &str,
    holder: &str,
    lease_minutes: i64,
    now: UtcTime,
    lost_race_before: bool,
    resolve: model::Resolve,
) -> Result<TxPlan, CoreError> {
    let Some(unit) = inst_at_base.unit(unit_id) else {
        return Err(CoreError::refusal(
            4,
            format!("error: unit not found at mainline head: {unit_id}"),
        ));
    };
    let view = model::unit_view_with(unit, &inst_at_base.units, now, resolve);

    let (edits, event_name, new_seq, expires): (Vec<(String, String)>, &str, i64, Option<String>) =
        match op {
            Op::Claim => {
                if !view.claimable {
                    let blockers: Vec<String> = view
                        .depends_on
                        .iter()
                        .filter(|(_, s)| !model::is_done(s))
                        .map(|(d, s)| match view.through.iter().find(|t| &t.entry == d) {
                            // a pin: the units behind it that are not done
                            Some(t) => {
                                let unmet: Vec<String> = t
                                    .units
                                    .iter()
                                    .filter(|(_, s)| !model::is_done(s))
                                    .map(|(n, s)| format!("{n}={s}"))
                                    .collect();
                                format!("{d} ({})", unmet.join(", "))
                            }
                            None => format!("{d}={s}"),
                        })
                        .collect();
                    let why = if view.lease_live {
                        format!(
                            "status={}, live lease held by {}",
                            view.status,
                            view.claimed_by.as_deref().unwrap_or("None")
                        )
                    } else if blockers.is_empty() {
                        format!("status={}", view.status)
                    } else {
                        format!(
                            "status={}, unmet deps: {}",
                            view.status,
                            blockers.join(", ")
                        )
                    };
                    return Err(CoreError::refusal(
                        if lost_race_before { 5 } else { 4 },
                        format!("error: {unit_id} not claimable ({why})"),
                    ));
                }
                let new_seq = view.claim_seq + 1;
                let expires = now.add_minutes(lease_minutes).format();
                (
                    vec![
                        ("status".into(), "Claimed".into()),
                        ("claimed_by".into(), holder.to_string()),
                        ("claim_expires".into(), expires.clone()),
                        ("claim_seq".into(), new_seq.to_string()),
                    ],
                    "claim",
                    new_seq,
                    Some(expires),
                )
            }
            Op::Renew | Op::Release => {
                if view.status != "Claimed" || view.claimed_by.as_deref() != Some(holder) {
                    return Err(CoreError::refusal(
                        4,
                        format!(
                            "error: {unit_id} is not Claimed by {holder} (status={}, holder={})",
                            view.status,
                            view.claimed_by.as_deref().unwrap_or("None")
                        ),
                    ));
                }
                if op == Op::Renew {
                    if !view.lease_live {
                        return Err(CoreError::refusal(
                            6,
                            format!(
                                "error: lease on {unit_id} already expired — re-claim instead of renewing"
                            ),
                        ));
                    }
                    let expires = now.add_minutes(lease_minutes).format();
                    (
                        vec![("claim_expires".into(), expires.clone())],
                        "renew",
                        view.claim_seq,
                        Some(expires),
                    )
                } else {
                    (
                        vec![
                            ("status".into(), "Ready".into()),
                            ("claimed_by".into(), "—".into()),
                            ("claim_expires".into(), "—".into()),
                        ],
                        "release",
                        view.claim_seq,
                        None,
                    )
                }
            }
        };

    let event = Event {
        seq: 0, // assigned at append time by the executor
        at: now.format(),
        event: event_name.to_string(),
        unit: Some(unit_id.to_string()),
        actor: Some(holder.to_string()),
        claim_seq: Some(new_seq),
        sdd: inst_at_base.meta.get_str_or("version", "0.0.0").to_string(),
        ..Default::default()
    };

    Ok(TxPlan {
        edits,
        event,
        commit_message: format!(
            "sdd: {} {} (c{}) by {}",
            op.as_str(),
            unit_id,
            new_seq,
            holder
        ),
        claim_seq: new_seq,
        claim_expires: expires,
    })
}
