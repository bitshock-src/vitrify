//! Contracts beyond their text: header lines, the body, and *external*
//! contracts — one SDD's stand-in for a surface it does not own.
//!
//! Everything vitrify does is scoped to one SDD: a unit's `consumes` names
//! contracts of its own SDD, and its reading set and trace are built from
//! that SDD's files alone. A unit that codes against a surface another SDD
//! owns therefore consumes a contract of its own SDD that *stands for* the
//! other's — an external contract, one whose `Producer:` line begins
//! `external`. It takes one of two forms:
//!
//! * a **pin** — of a contract another SDD owns: the header carries a `Pin:`
//!   line naming the authority and the hash of its body, and there is no
//!   body: a reader follows the pin;
//! * a **transcription** — of a source no SDD owns: its `Producer:` line
//!   names no contract, and its body is the consumer's own text, which
//!   nothing in the repository can check.
//!
//! This module finds them, resolves the authority each pin names among the
//! repository's SDDs, and says whether the pin still matches — and what
//! text would make it match. Nothing here reads anything but spec files;
//! [`apply`] is the one function that writes.

use std::cell::OnceCell;
use std::path::{Path, PathBuf};

use crate::CoreError;
use crate::model::{self, Instance, SddHandle, Shape, Unit, is_empty_marker};
use crate::trace::hash12;

// ------------------------------------------------------------------ peers

/// The SDDs of one repository, found on first use: what an external
/// contract's authority is looked up in. Finding them costs a listing of the
/// repository, so it happens only when an SDD actually has such a contract.
pub struct Peers<'a> {
    find: Box<dyn Fn() -> Vec<SddHandle> + 'a>,
    found: OnceCell<Vec<SddHandle>>,
}

impl<'a> Peers<'a> {
    pub fn new(find: impl Fn() -> Vec<SddHandle> + 'a) -> Peers<'a> {
        Peers {
            find: Box::new(find),
            found: OnceCell::new(),
        }
    }

    /// SDDs that are already known.
    pub fn of(sdds: Vec<SddHandle>) -> Peers<'static> {
        Peers::new(move || sdds.clone())
    }

    pub fn all(&self) -> &[SddHandle] {
        self.found.get_or_init(|| (self.find)())
    }
}

/// An SDD's slug: its `sdd:` front-matter value.
pub fn slug(sdd: &SddHandle) -> &str {
    sdd.meta.get_str("sdd").map(str::trim).unwrap_or("")
}

fn same_root(a: &Path, b: &Path) -> bool {
    a == b || matches!((a.canonicalize(), b.canonicalize()), (Ok(x), Ok(y)) if x == y)
}

/// The repository's SDDs with `own` among them (an SDD named by `--sdd`
/// need not be one discovery finds).
fn with_own(own: &SddHandle, peers: &Peers) -> Vec<SddHandle> {
    let mut all: Vec<SddHandle> = peers.all().to_vec();
    if !all.iter().any(|s| same_root(&s.root, &own.root)) {
        all.push(own.clone());
    }
    all
}

// -------------------------------------------------------------- contracts

/// One contract of an SDD.
#[derive(Debug, Clone)]
pub struct Contract {
    pub id: String,
    /// Bundle: the contract's file. Single: the SDD file.
    pub file: PathBuf,
    /// Bundle: the file's text. Single: the `### C-###` region.
    pub text: String,
    /// Single shape: where the region lies in the SDD file.
    pub span: Option<(usize, usize)>,
}

impl Contract {
    /// A region of a single-file SDD rather than a file of its own.
    pub fn is_region(&self) -> bool {
        self.span.is_some()
    }
}

/// `C-<digits>` and nothing else.
pub fn is_contract_id(s: &str) -> bool {
    s.strip_prefix("C-")
        .is_some_and(|d| !d.is_empty() && d.bytes().all(|b| b.is_ascii_digit()))
}

/// The id a bundle's contract file carries in its name: `C-###-<slug>.md` or
/// `C-###.md` — the two names the resolution rule looks for.
fn id_of_file(name: &str) -> Option<String> {
    let stem = name.strip_suffix(".md")?;
    let digits: String = stem
        .strip_prefix("C-")?
        .chars()
        .take_while(char::is_ascii_digit)
        .collect();
    if digits.is_empty() {
        return None;
    }
    let rest = &stem[2 + digits.len()..];
    (rest.is_empty() || rest.starts_with('-')).then(|| format!("C-{digits}"))
}

/// Every contract of an SDD, in file order (bundle) or document order
/// (single). An id held by several files is the one the resolution rule
/// picks ([`model::contract_file`]).
pub fn list(sdd: &SddHandle) -> Result<Vec<Contract>, CoreError> {
    let mut out: Vec<Contract> = Vec::new();
    match sdd.shape {
        Shape::Bundle => {
            let mut ids: Vec<String> = Vec::new();
            for p in model::list_dir(&sdd.root.join("contracts")) {
                let name = p.file_name().and_then(|n| n.to_str()).unwrap_or("");
                if p.is_file()
                    && let Some(id) = id_of_file(name)
                    && !ids.contains(&id)
                {
                    ids.push(id);
                }
            }
            for id in ids {
                if let Some(file) = model::contract_file(&sdd.root, &id) {
                    let text = model::read_text(&file)?;
                    out.push(Contract {
                        id,
                        file,
                        text,
                        span: None,
                    });
                }
            }
        }
        Shape::Single => {
            let text = model::read_text(&sdd.file)?;
            for (ident, _) in model::split_h3_regions(&text, "Interface Contracts") {
                if !is_contract_id(&ident) || out.iter().any(|c| c.id == ident) {
                    continue;
                }
                if let Some((s, e)) = model::find_region_span(&text, "Interface Contracts", &ident)
                {
                    out.push(Contract {
                        id: ident,
                        file: sdd.file.clone(),
                        text: text[s..e].to_string(),
                        span: Some((s, e)),
                    });
                }
            }
        }
    }
    Ok(out)
}

/// One contract of an SDD by id — the entry the trace hashes and the
/// reading set names.
pub fn find(sdd: &SddHandle, id: &str) -> Result<Option<Contract>, CoreError> {
    match sdd.shape {
        Shape::Bundle => match model::contract_file(&sdd.root, id) {
            Some(file) => Ok(Some(Contract {
                id: id.to_string(),
                text: model::read_text(&file)?,
                file,
                span: None,
            })),
            None => Ok(None),
        },
        Shape::Single => {
            let text = model::read_text(&sdd.file)?;
            Ok(
                model::find_region_span(&text, "Interface Contracts", id).map(|(s, e)| Contract {
                    id: id.to_string(),
                    file: sdd.file.clone(),
                    text: text[s..e].to_string(),
                    span: Some((s, e)),
                }),
            )
        }
    }
}

// ----------------------------------------------------------------- header

/// The level of an ATX heading line (`#` … `######`, then whitespace or the
/// end of the line), tested at column 0 as every vitrify parser does.
fn heading_level(line: &str) -> Option<usize> {
    let hashes = line.bytes().take_while(|b| *b == b'#').count();
    if !(1..=6).contains(&hashes) {
        return None;
    }
    let rest = &line[hashes..];
    (rest.is_empty() || rest.starts_with(char::is_whitespace)).then_some(hashes)
}

fn heading_title(line: &str) -> &str {
    line.trim_start_matches('#')
        .trim()
        .trim_end_matches('#')
        .trim_end()
}

/// Fenced code blocks: a line starting with three or more backticks or
/// tildes opens one, the same marker at least as long closes it. Lines
/// inside are code, whatever they start with.
#[derive(Default)]
struct Fences {
    open: Option<(u8, usize)>,
}

impl Fences {
    /// Feed one line; `true` when it is inside a fenced block or is one of
    /// its fence lines.
    fn step(&mut self, line: &str) -> bool {
        let t = line.trim_start_matches(' ');
        let indent = line.len() - t.len();
        let marker = t.bytes().next().filter(|b| *b == b'`' || *b == b'~');
        let run = match marker {
            Some(m) => t.bytes().take_while(|b| *b == m).count(),
            None => 0,
        };
        match self.open {
            None => {
                if let Some(m) = marker
                    && indent <= 3
                    && run >= 3
                    // a backtick fence's info string holds no backtick
                    && !(m == b'`' && t[run..].contains('`'))
                {
                    self.open = Some((m, run));
                    return true;
                }
                false
            }
            Some((m, n)) => {
                if indent <= 3 && marker == Some(m) && run >= n && t[run..].trim().is_empty() {
                    self.open = None;
                }
                true
            }
        }
    }
}

/// `text` split into lines with their byte offsets; each line without its
/// terminator.
fn lines_at(text: &str) -> impl Iterator<Item = (usize, &str)> {
    let mut pos = 0;
    text.split_inclusive('\n').map(move |raw| {
        let at = pos;
        pos += raw.len();
        (at, raw.trim_end_matches(['\n', '\r']))
    })
}

/// The header of a contract: the lines between its title and its first
/// section heading, each with its byte offset.
fn header_lines(text: &str) -> Vec<(usize, &str)> {
    let mut out: Vec<(usize, &str)> = Vec::new();
    let mut titled = false;
    for (at, line) in lines_at(text) {
        if heading_level(line).is_some() {
            if !titled && out.iter().all(|(_, l)| l.trim().is_empty()) {
                titled = true;
                continue;
            }
            break;
        }
        out.push((at, line));
    }
    out
}

/// `<rest>` of a header line `- <key>: <rest>` (the bullet is optional).
fn field_rest<'a>(line: &'a str, key: &str) -> Option<&'a str> {
    let t = line.trim_start();
    let t = t
        .strip_prefix("- ")
        .or_else(|| t.strip_prefix("* "))
        .unwrap_or(t)
        .trim_start();
    t.strip_prefix(key)?.strip_prefix(':')
}

fn is_bullet(line: &str) -> bool {
    let t = line.trim_start();
    t.starts_with("- ") || t.starts_with("* ")
}

/// The value of a contract's first `<key>:` header line, with any indented
/// continuation lines joined to it. `None` when the header has no such line.
pub fn field(text: &str, key: &str) -> Option<String> {
    let lines = header_lines(text);
    let i = lines
        .iter()
        .position(|(_, l)| field_rest(l, key).is_some())?;
    let mut value = field_rest(lines[i].1, key)?.trim().to_string();
    for (_, l) in &lines[i + 1..] {
        if l.trim().is_empty() || !l.starts_with([' ', '\t']) || is_bullet(l) {
            break;
        }
        value.push(' ');
        value.push_str(l.trim());
    }
    Some(value)
}

/// Every `<prefix><digits>` in `text` that stands as a word of its own —
/// not the tail of a longer word (`EC-13` holds no `C-13`) nor the head of
/// one — with its byte offset.
fn ids_at(text: &str, prefix: &str) -> Vec<(usize, String)> {
    let bytes = text.as_bytes();
    let mut out = Vec::new();
    let mut from = 0;
    while let Some(rel) = text[from..].find(prefix) {
        let at = from + rel;
        let digits = bytes[at + prefix.len()..]
            .iter()
            .take_while(|b| b.is_ascii_digit())
            .count();
        let end = at + prefix.len() + digits;
        let word = |c: char| c.is_alphanumeric() || c == '_';
        let starts_word = text[..at].chars().next_back().is_none_or(|c| !word(c));
        let ends_word = text[end..].chars().next().is_none_or(|c| !word(c));
        if digits > 0 && starts_word && ends_word {
            out.push((at, text[at..end].to_string()));
        }
        from = at + prefix.len();
    }
    out
}

/// The unit ids (`U-###`) written in `text`, in order, each once.
pub fn unit_ids(text: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for (_, id) in ids_at(text, "U-") {
        if !out.contains(&id) {
            out.push(id);
        }
    }
    out
}

/// Whether a `Producer:` value marks its contract external: it begins with
/// the word `external`.
pub fn is_external(producer: &str) -> bool {
    producer
        .trim_start()
        .strip_prefix("external")
        .is_some_and(|rest| {
            !rest.starts_with(|c: char| c.is_alphanumeric() || c == '_' || c == '-')
        })
}

/// Where a contract's `Consumers:` line and its units' `consumes` lists
/// disagree: units the line names that do not consume it, and units that
/// consume it and are not named. The contract's own producer is left out of
/// both — it may consume its own contract, and the line lists its
/// *consumers*.
pub fn consumer_mismatch(
    contract: &Contract,
    units: &[Unit],
) -> Option<(Vec<String>, Vec<String>)> {
    let producer = field(&contract.text, "Producer").unwrap_or_default();
    // an external producer's line names no unit of this SDD
    let producers = if is_external(&producer) {
        Vec::new()
    } else {
        unit_ids(&producer)
    };
    let listed: Vec<String> = unit_ids(&field(&contract.text, "Consumers").unwrap_or_default())
        .into_iter()
        .filter(|u| !producers.contains(u))
        .collect();
    let consuming: Vec<String> = units
        .iter()
        .filter(|u| u.meta.get_list("consumes").contains(&contract.id))
        .map(|u| u.unit.clone())
        .filter(|u| !producers.contains(u))
        .collect();
    let mut header_only: Vec<String> = listed
        .iter()
        .filter(|u| !consuming.contains(u))
        .cloned()
        .collect();
    let mut consumes_only: Vec<String> = consuming
        .iter()
        .filter(|u| !listed.contains(u))
        .cloned()
        .collect();
    if header_only.is_empty() && consumes_only.is_empty() {
        return None;
    }
    header_only.sort();
    consumes_only.sort();
    consumes_only.dedup();
    Some((header_only, consumes_only))
}

// ------------------------------------------------------------------- body

/// Where a contract's body starts: the byte offset and the level of its
/// first `Surface` heading outside fenced code. The body runs from there to
/// the end of the contract — the end of its file, or of its region in a
/// single-file SDD.
pub fn surface(text: &str) -> Option<(usize, usize)> {
    let mut fences = Fences::default();
    for (at, line) in lines_at(text) {
        if !fences.step(line)
            && let Some(level) = heading_level(line)
            && heading_title(line) == "Surface"
        {
            return Some((at, level));
        }
    }
    None
}

/// `s` without the blank lines at its end.
fn trim_blank_end(s: &str) -> &str {
    s.trim_end_matches(['\n', '\r', ' ', '\t'])
}

/// A contract's body, as found.
struct Body<'a> {
    text: &'a str,
    /// Part of a single-file SDD: the blank lines at its end separate it
    /// from what follows and are not its own.
    region: bool,
}

impl Body<'_> {
    /// The body as a reader is given it, and as a pin hashes it: a file's,
    /// byte for byte; a region's, without the blank lines that follow it.
    fn canonical(&self) -> String {
        if self.region {
            format!("{}\n", trim_blank_end(self.text))
        } else {
            self.text.to_string()
        }
    }
}

fn body_of(c: &Contract) -> Option<Body<'_>> {
    surface(&c.text).map(|(start, _)| Body {
        text: &c.text[start..],
        region: c.is_region(),
    })
}

// -------------------------------------------------------------------- pin

/// A `Pin:` header line: `<slug> C-### @ <hash>`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pin {
    pub slug: String,
    pub contract: String,
    /// `None` until a hash has been recorded.
    pub hash: Option<String>,
}

/// Read a `Pin:` value. `None` when it is not `<slug> C-###`, optionally
/// followed by `@ <hex>`.
pub fn parse_pin(value: &str) -> Option<Pin> {
    let (pointer, hash) = match value.split_once('@') {
        Some((p, h)) => (p.trim(), h.trim()),
        None => (value.trim(), ""),
    };
    let hash = if is_empty_marker(hash) {
        None
    } else {
        let h = hash.strip_prefix("sha256:").unwrap_or(hash);
        if !h.bytes().all(|b| b.is_ascii_hexdigit()) {
            return None;
        }
        Some(h.to_ascii_lowercase())
    };
    let (slug, contract) = pointer.rsplit_once(char::is_whitespace)?;
    let slug = slug.trim().trim_end_matches(',').trim_end();
    if slug.is_empty() || !is_contract_id(contract) {
        return None;
    }
    Some(Pin {
        slug: slug.to_string(),
        contract: contract.to_string(),
        hash,
    })
}

/// The hash a pin on `body` records: the trace's own digest of it.
pub fn pin_hash(body: &str) -> String {
    hash12(body)
}

/// A recorded pin hash against the current one, on the 12 characters a hash
/// is recorded with.
fn pin_matches(recorded: &str, current: &str) -> bool {
    recorded.len() >= current.len() && recorded[..current.len()] == *current
}

/// `text` with its `Pin:` line rewritten to name `pin`'s authority at
/// `hash`. `None` when the header has no such line.
fn repinned(text: &str, pin: &Pin, hash: &str) -> Option<String> {
    let (at, line) = header_lines(text)
        .into_iter()
        .find(|(_, l)| field_rest(l, "Pin").is_some())?;
    let lead = &line[..line.find("Pin:")?];
    Some(format!(
        "{}{lead}Pin: {} {} @ {hash}{}",
        &text[..at],
        pin.slug,
        pin.contract,
        &text[at + line.len()..]
    ))
}

// --------------------------------------------------------------- authority

/// What an external `Producer:` line names.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Naming {
    /// No contract id: nothing in the repository to hold it against (a
    /// transcription of a source that has no SDD).
    Nothing,
    /// A contract id, with no SDD of the repository named before it.
    NoSdd { contract: String },
    /// The contract `contract` of the SDD whose slug is `slug`.
    Authority { slug: String, contract: String },
}

/// Read an external `Producer:` value against the slugs of the repository's
/// SDDs. An authority is named by the producer's slug followed — however
/// many words later — by the contract's id: `external — pool, C-010`. Of
/// several slugs before an id, the nearest names its SDD.
pub fn naming(producer: &str, slugs: &[&str]) -> Naming {
    let ids = ids_at(producer, "C-");
    for (at, id) in &ids {
        if let Some(slug) = nearest_slug(&producer[..*at], slugs) {
            return Naming::Authority {
                slug: slug.to_string(),
                contract: id.clone(),
            };
        }
    }
    match ids.first() {
        Some((_, id)) => Naming::NoSdd {
            contract: id.clone(),
        },
        None => Naming::Nothing,
    }
}

/// The slug whose last whole-word occurrence in `before` lies furthest to
/// the right.
fn nearest_slug<'s>(before: &str, slugs: &[&'s str]) -> Option<&'s str> {
    let part = |c: char| c.is_alphanumeric() || c == '-' || c == '_';
    let mut best: Option<(usize, &'s str)> = None;
    for slug in slugs.iter().filter(|s| !s.is_empty()) {
        let mut from = 0;
        while let Some(rel) = before[from..].find(slug) {
            let at = from + rel;
            let end = at + slug.len();
            let whole = before[..at].chars().next_back().is_none_or(|c| !part(c))
                && before[end..].chars().next().is_none_or(|c| !part(c));
            if whole && best.is_none_or(|(e, s)| end > e || (end == e && slug.len() > s.len())) {
                best = Some((end, slug));
            }
            from = at + slug.len().max(1);
            if from > before.len() || !before.is_char_boundary(from) {
                break;
            }
        }
    }
    best.map(|(_, s)| s)
}

/// The contract an external contract stands for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Authority {
    pub slug: String,
    pub contract: String,
    /// The file holding it, once found.
    pub file: Option<PathBuf>,
    /// The size of its body, once found.
    pub body_bytes: Option<u64>,
}

/// An authority found, with its body.
struct Found {
    authority: Authority,
    contract: Contract,
}

enum Lookup {
    Found(Box<Found>),
    /// The SDD or the contract is not there.
    Missing(Authority, String),
    /// It is there, and cannot serve as an authority.
    Unusable(Authority, String),
}

fn look_up(own: &SddHandle, this: &Contract, slug: &str, id: &str, sdds: &[SddHandle]) -> Lookup {
    let named = Authority {
        slug: slug.to_string(),
        contract: id.to_string(),
        file: None,
        body_bytes: None,
    };
    let holders: Vec<&SddHandle> = sdds.iter().filter(|s| self::slug(s) == slug).collect();
    let sdd = match holders.as_slice() {
        [] => {
            return Lookup::Missing(
                named,
                format!("no SDD in this repository has the slug `{slug}`"),
            );
        }
        [one] => *one,
        several => {
            let roots: Vec<String> = several
                .iter()
                .map(|s| s.root.display().to_string())
                .collect();
            return Lookup::Unusable(
                named,
                format!(
                    "the slug `{slug}` is used by several SDDs ({})",
                    roots.join(", ")
                ),
            );
        }
    };
    if same_root(&sdd.root, &own.root) && id == this.id {
        return Lookup::Unusable(named, "it names itself as its authority".to_string());
    }
    let contract = match find(sdd, id) {
        Ok(Some(c)) => c,
        Ok(None) => return Lookup::Missing(named, format!("{slug} has no contract {id}")),
        Err(e) => return Lookup::Missing(named, format!("{slug} {id} cannot be read ({e})")),
    };
    let mut authority = named;
    authority.file = Some(contract.file.clone());
    match body_of(&contract) {
        Some(body) => {
            authority.body_bytes = Some(body.canonical().len() as u64);
            Lookup::Found(Box::new(Found {
                authority,
                contract,
            }))
        }
        None => Lookup::Unusable(
            authority,
            format!(
                "its authority {slug} {id} has no `Surface` heading — it has no body to hold this against"
            ),
        ),
    }
}

// ---------------------------------------------------------------- examine

/// The two forms an external contract takes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// A stand-in for a contract another SDD owns, held against it by a
    /// `Pin:` line — or meant to be: its `Producer:` line names one.
    Pin,
    /// The consumer's own text for a source no SDD owns.
    Transcription,
}

impl Kind {
    pub fn as_str(self) -> &'static str {
        match self {
            Kind::Pin => "pin",
            Kind::Transcription => "transcription",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum State {
    /// The pin matches its authority.
    Ok,
    /// The pin no longer matches; [`External::fix`] holds what would.
    Stale,
    /// It names an authority that cannot be found.
    MissingAuthority,
    /// What it declares cannot be acted on, and no sync puts it right.
    Invalid,
    /// A transcription: there is nothing to check it against.
    Unchecked,
}

impl State {
    pub fn as_str(self) -> &'static str {
        match self {
            State::Ok => "ok",
            State::Stale => "stale",
            State::MissingAuthority => "missing-authority",
            State::Invalid => "invalid",
            State::Unchecked => "unchecked",
        }
    }

    /// A state lint reports and a run refuses to start on.
    pub fn is_finding(self) -> bool {
        matches!(
            self,
            State::Stale | State::MissingAuthority | State::Invalid
        )
    }
}

/// A rewrite of one contract: the whole of `file`, or `span` of it.
#[derive(Debug, Clone)]
pub struct Fix {
    pub file: PathBuf,
    pub span: Option<(usize, usize)>,
    pub text: String,
}

/// One external contract of an SDD, examined.
#[derive(Debug, Clone)]
pub struct External {
    pub id: String,
    pub file: PathBuf,
    pub kind: Kind,
    pub state: State,
    pub authority: Option<Authority>,
    /// Why it is not `Ok`, as a clause that follows the contract's id
    /// (empty when it is).
    pub detail: String,
    /// The size of this SDD's own entry for it.
    pub bytes: u64,
    /// A pin's hash as its authority's body now gives it, once found.
    pub hash: Option<String>,
    /// What makes it match, when a sync can.
    pub fix: Option<Fix>,
}

impl External {
    fn new(c: &Contract, kind: Kind) -> External {
        External {
            id: c.id.clone(),
            file: c.file.clone(),
            kind,
            state: State::Ok,
            authority: None,
            detail: String::new(),
            bytes: c.text.len() as u64,
            hash: None,
            fix: None,
        }
    }

    fn is(mut self, state: State, detail: impl Into<String>) -> External {
        self.state = state;
        self.detail = detail.into();
        self
    }
}

fn cannot_be_found(why: &str) -> String {
    format!("its authority cannot be found: {why}")
}

fn fix_of(c: &Contract, text: String) -> Fix {
    Fix {
        file: c.file.clone(),
        span: c.span,
        text,
    }
}

/// Examine every external contract of `inst`: each contract whose
/// `Producer:` line begins `external`, and any that carries a `Pin:` line.
pub fn examine(inst: &Instance, peers: &Peers) -> Result<Vec<External>, CoreError> {
    let own = inst.handle();
    let mut out = Vec::new();
    for c in list(&own)? {
        let producer = field(&c.text, "Producer").unwrap_or_default();
        let pin = field(&c.text, "Pin");
        if !is_external(&producer) && pin.is_none() {
            continue;
        }
        out.push(match pin {
            Some(pin) => examine_pin(&own, &c, &producer, &pin, peers).0,
            None => examine_unpinned(&own, &c, &producer, peers),
        });
    }
    Ok(out)
}

/// An external contract with no `Pin:` line: a transcription when its
/// `Producer:` line names no contract; otherwise a pin whose `Pin:` line
/// is still to be written.
fn examine_unpinned(own: &SddHandle, c: &Contract, producer: &str, peers: &Peers) -> External {
    let sdds = with_own(own, peers);
    let slugs: Vec<&str> = sdds.iter().map(slug).collect();
    match naming(producer, &slugs) {
        Naming::Nothing => External::new(c, Kind::Transcription).is(
            State::Unchecked,
            "a transcription: its Producer line names no contract of this repository",
        ),
        Naming::NoSdd { contract } => External::new(c, Kind::Pin).is(
            State::MissingAuthority,
            cannot_be_found(&format!(
                "its Producer line names {contract}, and no SDD of this repository before it"
            )),
        ),
        Naming::Authority { slug, contract } => {
            let mut ext = External::new(c, Kind::Pin);
            ext.authority = Some(match look_up(own, c, &slug, &contract, &sdds) {
                Lookup::Found(found) => found.authority,
                Lookup::Missing(authority, _) | Lookup::Unusable(authority, _) => authority,
            });
            let place = if surface(&c.text).is_some() {
                "in place of its body"
            } else {
                "in its header"
            };
            ext.is(
                State::Invalid,
                format!(
                    "its Producer line names {slug} {contract} and it has no `Pin:` line — write `- Pin: {slug} {contract}` {place}, then `vitrify pin sync` records the hash"
                ),
            )
        }
    }
}

/// The body a matching pin stands for.
struct Served {
    text: String,
    /// The authority is a region of a single-file SDD.
    region: bool,
}

/// A pinned contract examined, and — when its pin holds — the body it
/// stands for.
fn examine_pin(
    own: &SddHandle,
    c: &Contract,
    producer: &str,
    pin_value: &str,
    peers: &Peers,
) -> (External, Option<Served>) {
    let ext = External::new(c, Kind::Pin);
    let invalid = |ext: External, why: String| (ext.is(State::Invalid, why), None);
    let Some(pin) = parse_pin(pin_value) else {
        return invalid(
            ext,
            format!("its `Pin:` line does not read `<slug> C-### @ <hash>` (`{pin_value}`)"),
        );
    };
    if !is_external(producer) {
        return invalid(
            ext,
            "it is pinned, so its `Producer:` line must begin `external`".to_string(),
        );
    }
    if surface(&c.text).is_some() {
        return invalid(
            ext,
            "it is pinned and still carries a body (`Surface` onward) — delete the body: a reader follows the pin"
                .to_string(),
        );
    }
    let sdds = with_own(own, peers);
    let slugs: Vec<&str> = sdds.iter().map(slug).collect();
    if let Naming::Authority { slug, contract } = naming(producer, &slugs)
        && (slug != pin.slug || contract != pin.contract)
    {
        return invalid(
            ext,
            format!(
                "its Producer line names {slug} {contract}, its pin {} {}",
                pin.slug, pin.contract
            ),
        );
    }
    let found = match look_up(own, c, &pin.slug, &pin.contract, &sdds) {
        Lookup::Found(found) => found,
        Lookup::Missing(authority, why) => {
            let mut ext = ext.is(State::MissingAuthority, cannot_be_found(&why));
            ext.authority = Some(authority);
            return (ext, None);
        }
        Lookup::Unusable(authority, why) => {
            let mut ext = ext.is(State::Invalid, why);
            ext.authority = Some(authority);
            return (ext, None);
        }
    };
    let mut ext = ext;
    ext.authority = Some(found.authority.clone());
    // invariant: `look_up` returns `Found` only for a contract with a body
    let Some(body) = body_of(&found.contract).map(|b| b.canonical()) else {
        return (ext, None);
    };
    let current = pin_hash(&body);
    ext.hash = Some(current.clone());
    if pin
        .hash
        .as_deref()
        .is_some_and(|h| pin_matches(h, &current))
    {
        let region = found.contract.is_region();
        return (ext, Some(Served { text: body, region }));
    }
    let what = match &pin.hash {
        Some(h) => format!(
            "its pin {h} is not the hash of {} {}'s body (now {current})",
            pin.slug, pin.contract
        ),
        None => format!(
            "its pin records no hash yet ({} {}'s body hashes to {current})",
            pin.slug, pin.contract
        ),
    };
    ext.fix = repinned(&c.text, &pin, &current).map(|text| fix_of(c, text));
    (ext.is(State::Stale, what), None)
}

/// What a reader of a pinned contract is given in place of the body it
/// does not carry.
#[derive(Debug, Clone)]
pub struct PinnedBody {
    pub authority: Authority,
    /// The authority's file.
    pub file: PathBuf,
    /// The part of that file the body is, as a reading names it.
    pub region: String,
    /// The body itself — the text the pin's hash is the hash of.
    pub text: String,
}

/// Follow contract `c`'s pin. `Ok(None)`: it has none. `Err`: it has one
/// that does not resolve or no longer matches — the state it is in and the
/// reason, as a clause that follows the contract's id. A body is only ever
/// handed out against a matching hash: a unit must not be built from text
/// its trace does not cover.
pub fn pinned_body(
    inst: &Instance,
    c: &Contract,
    peers: &Peers,
) -> Result<Option<PinnedBody>, (State, String)> {
    let Some(pin_value) = field(&c.text, "Pin") else {
        return Ok(None);
    };
    let producer = field(&c.text, "Producer").unwrap_or_default();
    let (ext, body) = examine_pin(&inst.handle(), c, &producer, &pin_value, peers);
    match (body, ext.authority) {
        (Some(served), Some(authority)) => {
            // invariant: a matching pin's authority was found, file and all
            let file = authority.file.clone().unwrap_or_default();
            let region = if served.region {
                format!(
                    "Interface Contracts / {} / Surface onward",
                    authority.contract
                )
            } else {
                "Surface onward".to_string()
            };
            Ok(Some(PinnedBody {
                authority,
                file,
                region,
                text: served.text,
            }))
        }
        _ => Err((ext.state, ext.detail)),
    }
}

// ------------------------------------------------------------------ apply

/// Write `fixes` to the working tree. Several fixes to one file — the
/// regions of a single-file SDD — are applied back to front, so each span
/// still means what it meant when it was computed; a region is followed by
/// one blank line unless it ends the file.
pub fn apply(fixes: &[Fix]) -> Result<(), CoreError> {
    for (file, text) in rendered(fixes)? {
        model::write_text(&file, &text)?;
    }
    Ok(())
}

/// What [`apply`] writes: each file `fixes` touch, in the order they first
/// name it, with the text it has once they are applied. Writes nothing, so
/// a dry run reads the same texts the sync would write.
pub fn rendered(fixes: &[Fix]) -> Result<Vec<(PathBuf, String)>, CoreError> {
    let mut files: Vec<&Path> = Vec::new();
    for f in fixes {
        if !files.contains(&f.file.as_path()) {
            files.push(&f.file);
        }
    }
    let mut out = Vec::new();
    for file in files {
        let mut of_file: Vec<&Fix> = fixes.iter().filter(|f| f.file == file).collect();
        if let Some(whole) = of_file.iter().find(|f| f.span.is_none()) {
            out.push((file.to_path_buf(), whole.text.clone()));
            continue;
        }
        of_file.sort_by_key(|f| std::cmp::Reverse(f.span.map(|(s, _)| s)));
        let mut text = model::read_text(file)?;
        for fix in of_file {
            let Some((start, end)) = fix.span else {
                continue;
            };
            if end > text.len() || !text.is_char_boundary(start) || !text.is_char_boundary(end) {
                return Err(CoreError::refusal(
                    1,
                    format!(
                        "error: {} changed while it was being synced",
                        file.display()
                    ),
                ));
            }
            let mut region = trim_blank_end(&fix.text).to_string();
            region.push('\n');
            if end < text.len() {
                region.push('\n');
            }
            text.replace_range(start..end, &region);
        }
        out.push((file.to_path_buf(), text));
    }
    Ok(out)
}
