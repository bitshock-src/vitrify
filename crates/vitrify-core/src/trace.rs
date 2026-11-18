//! Masking and trace hashing — byte-identical to the
//! golden corpus (`fixtures/golden/`). This module is the ONLY hashing/masking
//! implementation in vitrify; nothing may reimplement it.

use sha2::{Digest, Sha256};

use crate::CoreError;
use crate::model::{self, Instance, Shape, Unit};
use crate::newline::split_universal;
use crate::yamlish;

/// Volatile front-matter keys, masked out of the `unit` hash input.
pub const VOLATILE: [&str; 7] = [
    "status",
    "claimed_by",
    "claim_expires",
    "claim_seq",
    "built",
    "verified",
    "trace",
];

/// SHA-256 over the UTF-8 bytes; first 12 lowercase hex chars.
pub fn hash12(text: &str) -> String {
    let digest = Sha256::digest(text.as_bytes());
    let mut out = String::with_capacity(12);
    for byte in digest.iter().take(6) {
        out.push_str(&format!("{byte:02x}"));
    }
    out
}

/// Match `^([A-Za-z0-9_-]+):` — the masking walk's key-line test.
fn key_of_line(line: &str) -> Option<&str> {
    let colon = line.find(':')?;
    let key = &line[..colon];
    if key.is_empty()
        || !key
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
    {
        return None;
    }
    Some(key)
}

/// Remove volatile key lines (and the `trace` block's indented continuation
/// lines) from a metadata block. Lines joined with LF plus a trailing LF when
/// non-empty — the masking rule.
pub fn mask_meta_block(block: &str) -> String {
    mask_meta_keys(block, &VOLATILE)
}

/// The masking walk over an explicit key set: remove each listed key's line
/// (and, for `trace`, its indented continuation lines). With [`VOLATILE`] this
/// is the `unit` hash rule; the write-set check passes the narrower set
/// of fields an implementer's handoff may change.
pub fn mask_meta_keys(block: &str, keys: &[&str]) -> String {
    let mut out: Vec<&str> = Vec::new();
    let mut skipping = false;
    for line in split_universal(block) {
        if let Some(key) = key_of_line(line) {
            skipping = false;
            if keys.contains(&key) {
                skipping = key == "trace";
                continue;
            }
            out.push(line);
        } else if skipping
            && (line.trim().is_empty() || line.starts_with(' ') || line.starts_with('\t'))
        {
            continue;
        } else {
            skipping = false;
            out.push(line);
        }
    }
    if out.is_empty() {
        String::new()
    } else {
        let mut s = out.join("\n");
        s.push('\n');
        s
    }
}

/// Bundle unit file: `---\n` + masked inner + `---\n` + body; a file without
/// front matter is returned unchanged.
fn mask_bundle_unit(text: &str, keys: &[&str]) -> String {
    match yamlish::split_front_matter(text) {
        Some((inner, body)) => format!("---\n{}---\n{}", mask_meta_keys(inner, keys), body),
        None => text.to_string(),
    }
}

/// Single-shape `### U-###` region with its metadata block's inner content
/// masked in place; a region without a metadata block is returned unchanged.
fn mask_single_region(region: &str, keys: &[&str]) -> String {
    match model::find_meta_block_span(region) {
        Some((_, _, inner_s, inner_e)) => format!(
            "{}{}{}",
            &region[..inner_s],
            mask_meta_keys(&region[inner_s..inner_e], keys),
            &region[inner_e..]
        ),
        None => region.to_string(),
    }
}

/// The unit's entry with volatile fields masked — the `unit` hash input.
/// Bundle: the full unit file re-assembled as `---\n` + masked inner +
/// `---\n` + body. Single: the `### U-###` region with its metadata block's
/// inner content masked in place.
pub fn mask_unit_text(inst: &Instance, unit: &Unit) -> Result<String, CoreError> {
    match inst.shape {
        Shape::Bundle => Ok(mask_bundle_unit(&model::read_text(&unit.file)?, &VOLATILE)),
        Shape::Single => {
            let text = model::read_text(&inst.file)?;
            let Some((start, end)) = model::find_unit_span(&text, &unit.unit) else {
                return Err(CoreError::refusal(
                    4,
                    format!("error: unit not found: {}", unit.unit),
                ));
            };
            Ok(mask_single_region(&text[start..end], &VOLATILE))
        }
    }
}

/// The text of the file holding unit `uid`'s entry, with that unit's `keys`
/// masked in place — bundle: the unit file; single: the whole SDD file, only
/// `uid`'s metadata block masked. Two versions of the file that differ only in
/// those keys of that unit mask equal. `None` when a single-shape file has no
/// region for `uid`.
pub fn mask_unit_fields(shape: Shape, text: &str, uid: &str, keys: &[&str]) -> Option<String> {
    match shape {
        Shape::Bundle => Some(mask_bundle_unit(text, keys)),
        Shape::Single => {
            let (start, end) = model::find_unit_span(text, uid)?;
            Some(format!(
                "{}{}{}",
                &text[..start],
                mask_single_region(&text[start..end], keys),
                &text[end..]
            ))
        }
    }
}

/// A trace-verify mismatch: a key missing on either side or differing on the
/// compared prefix.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Mismatch {
    pub key: String,
    pub recorded: Option<String>,
    pub current: Option<String>,
}

/// Ordered (key, hash12) pairs: `unit`, each consumed contract, `global`.
pub type TraceMap = Vec<(String, String)>;

/// Compute the ordered trace map: `unit`, each consumed contract in listed
/// order, `global`. Problems name missing contracts (exit-4 semantics).
pub fn compute(inst: &Instance, unit: &Unit) -> Result<(TraceMap, Vec<String>), CoreError> {
    let core_text = model::read_text(&inst.file)?;
    let consumes = unit.meta.get_list("consumes");
    let handle = inst.handle();
    let mut problems = Vec::new();
    let mut trace: Vec<(String, String)> =
        vec![("unit".to_string(), hash12(&mask_unit_text(inst, unit)?))];
    for cid in &consumes {
        match model::contract_text(&handle, cid)? {
            Some(text) => trace.push((cid.clone(), hash12(&text))),
            None => problems.push(format!("consumed contract missing: {cid}")),
        }
    }
    let global_input = format!(
        "{}\n{}",
        model::section(&core_text, "Ground Rules"),
        model::section(&core_text, "Technology Decisions")
    );
    trace.push(("global".to_string(), hash12(&global_input)));
    Ok((trace, problems))
}

/// Per-key comparison over the union of recorded and current key sets, on the
/// shorter prefix of the two (minimum 12); any missing or differing key is a
/// named mismatch. Keys compared in sorted order (union of both key sets).
pub fn verify(recorded: &[(String, String)], current: &[(String, String)]) -> Vec<Mismatch> {
    let mut keys: Vec<&str> = recorded
        .iter()
        .map(|(k, _)| k.as_str())
        .chain(current.iter().map(|(k, _)| k.as_str()))
        .collect();
    keys.sort_unstable();
    keys.dedup();
    let lookup = |set: &[(String, String)], key: &str| -> Option<String> {
        set.iter().find(|(k, _)| k == key).map(|(_, v)| v.clone())
    };
    let mut mismatches = Vec::new();
    for key in keys {
        let rec = lookup(recorded, key);
        let cur = lookup(current, key);
        let n = {
            let n = rec
                .as_deref()
                .map(str::len)
                .unwrap_or(0)
                .min(cur.as_deref().map(str::len).unwrap_or(0));
            if n == 0 { 12 } else { n }
        };
        let differs = match (&rec, &cur) {
            (Some(r), Some(c)) => r[..n.min(r.len())] != c[..n.min(c.len())],
            _ => true,
        };
        if differs {
            mismatches.push(Mismatch {
                key: key.to_string(),
                recorded: rec,
                current: cur,
            });
        }
    }
    mismatches
}
