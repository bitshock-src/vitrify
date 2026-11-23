//! Instance model: SDD discovery, shapes, front matter, regions, units, and
//! the claimable predicate — held by the regression goldens
//! (`fixtures/golden/`). `.vitrify` joins every skip set.

use std::path::{Path, PathBuf};

use crate::CoreError;
use crate::yamlish::{self, Value};

/// The empty-marker set: all equivalent to unset.
pub fn is_empty_marker(s: &str) -> bool {
    matches!(s, "" | "-" | "—" | "~" | "null" | "None")
}

/// Directory parts the repo-wide SDD search never enters: git's and
/// vitrify's own (vitrify's worktrees and scratch live under `.vitrify/`).
/// Nothing ecosystem-specific — a caller with git passes the files git would
/// commit instead ([`discover_with`]), so git's ignore rules decide.
pub const SKIP_PARTS: [&str; 2] = [".git", ".vitrify"];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Shape {
    Bundle,
    Single,
}

impl Shape {
    pub fn as_str(&self) -> &'static str {
        match self {
            Shape::Bundle => "bundle",
            Shape::Single => "single",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    Draft,
    Ready,
    Claimed,
    Built,
    Verified,
    Stale,
    Retired,
}

impl Status {
    pub fn parse(s: &str) -> Option<Status> {
        Some(match s {
            "Draft" => Status::Draft,
            "Ready" => Status::Ready,
            "Claimed" => Status::Claimed,
            "Built" => Status::Built,
            "Verified" => Status::Verified,
            "Stale" => Status::Stale,
            "Retired" => Status::Retired,
            _ => return None,
        })
    }
}

/// Ordered front-matter pairs with dict-style accessors.
#[derive(Debug, Clone, Default)]
pub struct Meta(pub Vec<(String, Value)>);

impl Meta {
    pub fn get(&self, key: &str) -> Option<&Value> {
        self.0.iter().find(|(k, _)| k == key).map(|(_, v)| v)
    }

    pub fn has(&self, key: &str) -> bool {
        self.get(key).is_some()
    }

    /// Scalar as string; lists/maps and absence yield `""` only via `get_str_or`.
    pub fn get_str(&self, key: &str) -> Option<&str> {
        self.get(key).and_then(Value::as_str)
    }

    pub fn get_str_or<'a>(&'a self, key: &str, default: &'a str) -> &'a str {
        self.get_str(key).unwrap_or(default)
    }

    /// List normalization: absent or empty-marker string → `[]`;
    /// scalar string → one-item list; list → itself.
    pub fn get_list(&self, key: &str) -> Vec<String> {
        match self.get(key) {
            None => Vec::new(),
            Some(Value::Str(s)) => {
                if is_empty_marker(s) {
                    Vec::new()
                } else {
                    vec![s.clone()]
                }
            }
            Some(Value::List(items)) => items.clone(),
            Some(Value::Map(pairs)) => pairs.iter().map(|(k, _)| k.clone()).collect(),
        }
    }

    pub fn get_map(&self, key: &str) -> Option<&Vec<(String, String)>> {
        match self.get(key) {
            Some(Value::Map(pairs)) => Some(pairs),
            _ => None,
        }
    }

    /// `int(m.get(key, 0) or 0)` over yamlish scalars; unparseable → 0.
    pub fn get_int(&self, key: &str) -> i64 {
        match self.get_str(key) {
            None => 0,
            Some("") => 0,
            Some(s) => s.trim().parse().unwrap_or(0),
        }
    }
}

/// Strict-UTF-8 text read (invalid UTF-8 in a spec file is exit 1), with
/// universal-newline translation: `\r\n` and lone `\r` become `\n` AT READ
/// TIME — every verb reads spec files this way, so the translated text is
/// the hashing/masking input class.
pub fn read_text(path: &Path) -> Result<String, CoreError> {
    let bytes = std::fs::read(path).map_err(|source| CoreError::Io {
        path: path.to_path_buf(),
        source,
    })?;
    let text = String::from_utf8(bytes).map_err(|_| CoreError::Utf8 {
        path: path.to_path_buf(),
    })?;
    if text.contains('\r') {
        Ok(text.replace("\r\n", "\n").replace('\r', "\n"))
    } else {
        Ok(text)
    }
}

pub fn write_text(path: &Path, text: &str) -> Result<(), CoreError> {
    std::fs::write(path, text).map_err(|source| CoreError::Io {
        path: path.to_path_buf(),
        source,
    })
}

/// Parse a document's front matter; `None` when absent or without the fence.
pub fn front_matter(text: &str) -> Option<Meta> {
    let (inner, _) = yamlish::split_front_matter(text)?;
    Some(Meta(yamlish::parse(inner)))
}

// --------------------------------------------------------- schema version

/// The schema versions this vitrify reads. 2.1 adds one thing to 2.0: a
/// unit's `depends_on` may name a pin of its own SDD, to wait for the units
/// that produce the pinned contract in another SDD. 2.2 adds one thing to
/// 2.1: §6 System Layout & Ownership may list the SDD's test files under
/// `tests:`. Every 2.0 and 2.1 document means the same under 2.2.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Schema {
    V2_0,
    V2_1,
    V2_2,
}

impl Schema {
    /// Every version this vitrify reads, oldest first.
    pub const READ: [Schema; 3] = [Schema::V2_0, Schema::V2_1, Schema::V2_2];

    pub fn as_str(self) -> &'static str {
        match self {
            Schema::V2_0 => "2.0",
            Schema::V2_1 => "2.1",
            Schema::V2_2 => "2.2",
        }
    }

    /// Whether a unit's `depends_on` may name a pin of its own SDD.
    pub fn pins_in_depends_on(self) -> bool {
        self >= Schema::V2_1
    }

    /// Whether §6 may list the SDD's tests (`tests:`).
    pub fn tests_list(self) -> bool {
        self >= Schema::V2_2
    }

    /// The versions this vitrify reads, as a refusal names them:
    /// `2.0, 2.1 and 2.2`.
    pub fn read_list() -> String {
        let names: Vec<&str> = Schema::READ.iter().map(|s| s.as_str()).collect();
        match names.split_last() {
            Some((last, init)) if !init.is_empty() => format!("{} and {last}", init.join(", ")),
            _ => names.concat(),
        }
    }
}

/// Why an SDD's front matter names no schema version this vitrify reads.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SchemaRefusal {
    /// No `schema_version` key, or an empty marker.
    Missing,
    /// A version this vitrify does not read, as written.
    Unread(String),
}

/// The schema version an SDD's front matter declares: `2`, `2.0` and
/// `2.0.x` are 2.0; `2.1` and `2.1.x` are 2.1; `2.2` and `2.2.x` are 2.2.
/// The key is required;
/// anything else is a version this vitrify does not read.
pub fn schema_of(meta: &Meta) -> Result<Schema, SchemaRefusal> {
    let raw = meta.get_str("schema_version").unwrap_or("").trim();
    if is_empty_marker(raw) {
        return Err(SchemaRefusal::Missing);
    }
    let mut parts = raw.split('.');
    let numeric = |p: Option<&str>| p.map(|s| s.parse::<u32>().ok());
    let major = numeric(parts.next());
    let minor = numeric(parts.next()).unwrap_or(Some(0));
    let patch = numeric(parts.next()).unwrap_or(Some(0));
    if parts.next().is_some() || patch.is_none() {
        return Err(SchemaRefusal::Unread(raw.to_string()));
    }
    match (major, minor) {
        (Some(Some(2)), Some(0)) => Ok(Schema::V2_0),
        (Some(Some(2)), Some(1)) => Ok(Schema::V2_1),
        (Some(Some(2)), Some(2)) => Ok(Schema::V2_2),
        _ => Err(SchemaRefusal::Unread(raw.to_string())),
    }
}

/// The refusal (exit 4) for an SDD whose schema version this vitrify does
/// not read, naming the versions it does.
pub fn schema_refusal(file: &Path, why: &SchemaRefusal) -> CoreError {
    let read = Schema::read_list();
    let what = match why {
        SchemaRefusal::Missing => format!("schema_version is missing; this vitrify reads {read}"),
        SchemaRefusal::Unread(written) => {
            format!("schema_version {written} is not one this vitrify reads ({read})")
        }
    };
    CoreError::refusal(
        4,
        format!("error: {}: {what} → vitrify schema show", file.display()),
    )
}

// ------------------------------------------------------------- discovery

/// One discovered SDD: `root` is the bundle dir or the single file itself.
#[derive(Debug, Clone)]
pub struct SddHandle {
    pub root: PathBuf,
    pub shape: Shape,
    pub meta: Meta,
    pub file: PathBuf,
}

/// Tested on the root-relative path only, so a root that itself lives under
/// `.vitrify/` (vitrify's own worktrees) still searches its whole tree.
fn path_has_skip_part(path: &Path, root: &Path) -> bool {
    path.strip_prefix(root)
        .unwrap_or(path)
        .components()
        .any(|c| {
            c.as_os_str()
                .to_str()
                .is_some_and(|s| SKIP_PARTS.contains(&s))
        })
}

fn sorted_by_str(mut paths: Vec<PathBuf>) -> Vec<PathBuf> {
    paths.sort_by(|a, b| a.as_os_str().cmp(b.as_os_str()));
    paths
}

pub(crate) fn list_dir(dir: &Path) -> Vec<PathBuf> {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    sorted_by_str(rd.flatten().map(|e| e.path()).collect())
}

fn walk_md_files(root: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(rd) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in rd.flatten() {
            let p = entry.path();
            if p.is_dir() {
                stack.push(p);
            } else if p.extension().is_some_and(|e| e == "md") {
                out.push(p);
            }
        }
    }
    sorted_by_str(out)
}

/// Discover every SDD under `root`, walking the filesystem for the
/// repo-wide search — [`discover_with`] without a candidate list.
pub fn discover(root: &Path) -> Vec<SddHandle> {
    discover_with(root, None)
}

/// Discover every SDD under `root` in a fixed order: bundle
/// `specs/*/sdd.md`, single `specs/*.sdd.md`, then a repo-wide search over
/// `candidates` — the root-relative files git would commit — or, when
/// `None`, over every file on disk outside `.git` and `.vitrify`. A file
/// counts only if its front matter has an `sdd` key.
pub fn discover_with(root: &Path, candidates: Option<&[String]>) -> Vec<SddHandle> {
    let mut found: Vec<SddHandle> = Vec::new();
    let mut seen: Vec<PathBuf> = Vec::new();

    let add = |path: &Path, shape: Shape, found: &mut Vec<SddHandle>, seen: &mut Vec<PathBuf>| {
        let Ok(text) = read_text(path) else { return };
        let Some(meta) = front_matter(&text) else {
            return;
        };
        if !meta.has("sdd") {
            return;
        }
        let key = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
        if seen.contains(&key) {
            return;
        }
        seen.push(key);
        let root_path = match shape {
            Shape::Bundle => path.parent().unwrap_or(path).to_path_buf(),
            Shape::Single => path.to_path_buf(),
        };
        found.push(SddHandle {
            root: root_path,
            shape,
            meta,
            file: path.to_path_buf(),
        });
    };

    for entry in list_dir(&root.join("specs")) {
        let candidate = entry.join("sdd.md");
        if entry.is_dir() && candidate.is_file() {
            add(&candidate, Shape::Bundle, &mut found, &mut seen);
        }
    }
    for entry in list_dir(&root.join("specs")) {
        if entry.is_file()
            && entry
                .file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.ends_with(".sdd.md"))
        {
            add(&entry, Shape::Single, &mut found, &mut seen);
        }
    }
    let md_files = match candidates {
        Some(files) => sorted_by_str(
            files
                .iter()
                .filter(|f| f.ends_with(".md"))
                .map(|f| root.join(f))
                .collect(),
        ),
        None => walk_md_files(root),
    };
    for p in md_files {
        if path_has_skip_part(&p, root) {
            continue;
        }
        let name = p.file_name().and_then(|n| n.to_str()).unwrap_or("");
        if name == "sdd.md" {
            add(&p, Shape::Bundle, &mut found, &mut seen);
        } else if name.ends_with(".sdd.md") {
            add(&p, Shape::Single, &mut found, &mut seen);
        }
    }
    found
}

/// Resolve `--sdd` (bundle dir, `sdd.md` path, or single file) or auto-locate.
/// Exit-4 refusals name what to do next.
pub fn resolve_sdd(root: &Path, sdd_arg: Option<&str>, cwd: &Path) -> Result<SddHandle, CoreError> {
    resolve_sdd_with(root, sdd_arg, cwd, None)
}

/// [`resolve_sdd`] with the repo-wide search over `candidates`
/// ([`discover_with`]).
pub fn resolve_sdd_with(
    root: &Path,
    sdd_arg: Option<&str>,
    cwd: &Path,
    candidates: Option<&[String]>,
) -> Result<SddHandle, CoreError> {
    if let Some(arg) = sdd_arg {
        let mut p = PathBuf::from(arg);
        if !p.is_absolute() {
            p = cwd.join(&p);
            p = p.canonicalize().unwrap_or(p);
        }
        if p.is_dir() && p.join("sdd.md").exists() {
            let text = read_text(&p.join("sdd.md"))?;
            let meta = front_matter(&text).unwrap_or_default();
            return Ok(SddHandle {
                root: p.clone(),
                shape: Shape::Bundle,
                meta,
                file: p.join("sdd.md"),
            });
        }
        if p.is_file() {
            let text = read_text(&p)?;
            if let Some(meta) = front_matter(&text)
                && meta.has("sdd")
            {
                let shape = if p.file_name().is_some_and(|n| n == "sdd.md") {
                    Shape::Bundle
                } else {
                    Shape::Single
                };
                let root_path = match shape {
                    Shape::Bundle => p.parent().unwrap_or(&p).to_path_buf(),
                    Shape::Single => p.clone(),
                };
                return Ok(SddHandle {
                    root: root_path,
                    shape,
                    meta,
                    file: p,
                });
            }
        }
        return Err(CoreError::refusal(
            4,
            format!("error: --sdd {arg} is not an SDD root"),
        ));
    }
    let all = discover_with(root, candidates);
    if all.is_empty() {
        return Err(CoreError::refusal(4, NO_SDD));
    }
    if all.len() > 1 {
        return Err(CoreError::refusal(4, several_sdds(&all, cwd)));
    }
    Ok(all.into_iter().next().unwrap())
}

/// The refusal when a repository holds no SDD, naming where to start.
pub const NO_SDD: &str = "error: no SDD found in repository → start one: vitrify brief authoring (or vitrify new sdd inside specs/)";

/// The refusal when several SDDs are found and `--sdd` is absent: each
/// candidate as the `--sdd` value that names it from `cwd`.
pub fn several_sdds(sdds: &[SddHandle], cwd: &Path) -> String {
    let mut msg = String::from("error: multiple SDDs found; pass --sdd <root>:");
    for s in sdds {
        msg.push_str(&format!("\n  --sdd {}", sdd_value(&s.root, cwd)));
    }
    msg
}

/// `root` as an `--sdd` value given from `cwd`: relative when it lies under
/// `cwd`, absolute otherwise.
fn sdd_value(root: &Path, cwd: &Path) -> String {
    let rel = root
        .strip_prefix(cwd)
        .ok()
        .map(Path::to_path_buf)
        .or_else(|| {
            let (root, cwd) = (root.canonicalize().ok()?, cwd.canonicalize().ok()?);
            root.strip_prefix(&cwd).ok().map(Path::to_path_buf)
        });
    match rel {
        Some(rel) if rel.as_os_str().is_empty() => ".".to_string(),
        Some(rel) => rel.to_string_lossy().replace('\\', "/"),
        None => root.display().to_string(),
    }
}

// ------------------------------------------------------------------ units

#[derive(Debug, Clone)]
pub struct Unit {
    pub unit: String,
    pub file: PathBuf,
    pub meta: Meta,
}

fn uid_like(s: &str) -> bool {
    s.strip_prefix("U-")
        .is_some_and(|r| r.starts_with(|c: char| c.is_ascii_digit()))
}

/// Load every unit of an SDD (bundle: `units/U-*.md` with `unit:` front
/// matter; single: `### U-###` regions under `## Work Units`).
pub fn load_units(sdd: &SddHandle) -> Result<Vec<Unit>, CoreError> {
    let mut units = Vec::new();
    match sdd.shape {
        Shape::Bundle => {
            for p in list_dir(&sdd.root.join("units")) {
                let name = p.file_name().and_then(|n| n.to_str()).unwrap_or("");
                if !p.is_file() || !name.starts_with("U-") || !name.ends_with(".md") {
                    continue;
                }
                let text = read_text(&p)?;
                if let Some(meta) = front_matter(&text)
                    && let Some(uid) = meta.get_str("unit")
                {
                    units.push(Unit {
                        unit: uid.to_string(),
                        file: p.clone(),
                        meta,
                    });
                }
            }
        }
        Shape::Single => {
            let text = read_text(&sdd.file)?;
            for (ident, region) in split_h3_regions(&text, "Work Units") {
                let meta = unit_meta_from_region(region);
                let uid = meta
                    .get_str("unit")
                    .map(str::to_string)
                    .unwrap_or_else(|| ident.clone());
                if uid_like(&uid) {
                    units.push(Unit {
                        unit: uid,
                        file: sdd.file.clone(),
                        meta,
                    });
                }
            }
        }
    }
    Ok(units)
}

/// A loaded instance: handle + units.
#[derive(Debug, Clone)]
pub struct Instance {
    pub root: PathBuf,
    pub shape: Shape,
    pub file: PathBuf,
    pub meta: Meta,
    pub units: Vec<Unit>,
}

impl Instance {
    /// Load an SDD's units. An SDD whose `schema_version` this vitrify does
    /// not read is refused (exit 4) rather than read as something it is not.
    pub fn load(sdd: SddHandle) -> Result<Instance, CoreError> {
        if let Err(why) = schema_of(&sdd.meta) {
            return Err(schema_refusal(&sdd.file, &why));
        }
        let units = load_units(&sdd)?;
        Ok(Instance {
            root: sdd.root,
            shape: sdd.shape,
            file: sdd.file,
            meta: sdd.meta,
            units,
        })
    }

    /// Discover + resolve + load in one step.
    pub fn discover(
        repo_root: &Path,
        sdd_arg: Option<&str>,
        cwd: &Path,
    ) -> Result<Instance, CoreError> {
        Instance::load(resolve_sdd(repo_root, sdd_arg, cwd)?)
    }

    pub fn handle(&self) -> SddHandle {
        SddHandle {
            root: self.root.clone(),
            shape: self.shape,
            meta: self.meta.clone(),
            file: self.file.clone(),
        }
    }

    pub fn unit(&self, uid: &str) -> Option<&Unit> {
        self.units.iter().find(|u| u.unit == uid)
    }

    /// The schema version the SDD declares ([`schema_of`]); 2.0, the
    /// version that permits least, for an instance built without
    /// [`Instance::load`]'s check.
    pub fn schema(&self) -> Schema {
        schema_of(&self.meta).unwrap_or(Schema::V2_0)
    }
}

// ------------------------------------------------------- region extraction

fn line_spans(text: &str) -> Vec<(usize, usize)> {
    // (start, start_of_next_line) spans, terminators included in the span.
    let bytes = text.as_bytes();
    let mut spans = Vec::new();
    let mut start = 0;
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'\n' => {
                i += 1;
                spans.push((start, i));
                start = i;
            }
            b'\r' => {
                i += 1;
                if i < bytes.len() && bytes[i] == b'\n' {
                    i += 1;
                }
                spans.push((start, i));
                start = i;
            }
            _ => i += 1,
        }
    }
    if start < bytes.len() {
        spans.push((start, bytes.len()));
    }
    spans
}

fn line_content(text: &str, span: (usize, usize)) -> &str {
    text[span.0..span.1].trim_end_matches(['\n', '\r'])
}

/// Does this line match `^##\s+<heading>\s*$`?
fn is_h2_heading(line: &str, heading: &str) -> bool {
    let Some(rest) = line.strip_prefix("##") else {
        return false;
    };
    let ws_len = rest.len() - rest.trim_start().len();
    if ws_len == 0 {
        return false;
    }
    let Some(tail) = rest.trim_start().strip_prefix(heading) else {
        return false;
    };
    tail.trim().is_empty()
}

/// Is a line start at `pos` an `^##\s` section boundary (lookahead `^##\s`)?
/// The whitespace may be the line's own terminator.
fn is_h2_boundary(text: &str, pos: usize) -> bool {
    let rest = &text[pos..];
    rest.strip_prefix("##")
        .and_then(|r| r.chars().next())
        .is_some_and(char::is_whitespace)
}

/// `## <heading>` section span: the heading line through (exclusive) the next
/// `^##\s` line, or EOF. `None` when the heading is absent.
pub fn section_span(text: &str, heading: &str) -> Option<(usize, usize)> {
    let spans = line_spans(text);
    let idx = spans
        .iter()
        .position(|&s| is_h2_heading(line_content(text, s), heading))?;
    let start = spans[idx].0;
    for &(ls, _) in &spans[idx + 1..] {
        if is_h2_boundary(text, ls) {
            return Some((start, ls));
        }
    }
    Some((start, text.len()))
}

/// `## <heading>` section text; empty string when absent (the trace recipe's
/// absent-section rule).
pub fn section<'a>(text: &'a str, heading: &str) -> &'a str {
    match section_span(text, heading) {
        Some((s, e)) => &text[s..e],
        None => "",
    }
}

fn h3_token(line: &str) -> Option<&str> {
    let rest = line.strip_prefix("###")?;
    let trimmed = rest.trim_start();
    if trimmed.len() == rest.len() {
        return None; // no whitespace after ###
    }
    let token = trimmed.split_whitespace().next()?;
    Some(token)
}

fn clean_ident(token: &str) -> &str {
    token.trim_end_matches([':', '—', '-', ' '])
}

/// Every `### <ID>` region under `## <section>` in a single-shape file:
/// (cleaned ident, region text from the heading line to the next `###`
/// heading or section end).
pub fn split_h3_regions<'a>(text: &'a str, section_name: &str) -> Vec<(String, &'a str)> {
    let Some((sec_start, sec_end)) = section_span(text, section_name) else {
        return Vec::new();
    };
    let body = &text[sec_start..sec_end];
    let spans = line_spans(body);
    let mut marks: Vec<(usize, String)> = Vec::new();
    for (i, &s) in spans.iter().enumerate() {
        if i == 0 {
            continue; // the ## heading line itself
        }
        if let Some(token) = h3_token(line_content(body, s)) {
            marks.push((s.0, clean_ident(token).to_string()));
        }
    }
    let mut out = Vec::new();
    for (i, (start, ident)) in marks.iter().enumerate() {
        let end = marks.get(i + 1).map_or(body.len(), |(s, _)| *s);
        out.push((ident.clone(), &body[*start..end]));
    }
    out
}

/// Absolute (start, end) of the `### U-###` region inside `## Work Units`
/// Single-shape region extraction.
pub fn find_unit_span(text: &str, uid: &str) -> Option<(usize, usize)> {
    find_region_span(text, "Work Units", uid)
}

/// Absolute (start, end) of the first `### <ident>` region inside
/// `## <section>` of a single-shape file — the span [`split_h3_regions`]
/// yields as text.
pub fn find_region_span(text: &str, section_name: &str, ident: &str) -> Option<(usize, usize)> {
    let (sec_start, sec_end) = section_span(text, section_name)?;
    let body = &text[sec_start..sec_end];
    let spans = line_spans(body);
    let mut marks: Vec<(usize, String)> = Vec::new();
    for (i, &s) in spans.iter().enumerate() {
        if i == 0 {
            continue;
        }
        if let Some(token) = h3_token(line_content(body, s)) {
            marks.push((s.0, clean_ident(token).to_string()));
        }
    }
    for (i, (start, found)) in marks.iter().enumerate() {
        if found == ident {
            let end = marks.get(i + 1).map_or(body.len(), |(s, _)| *s);
            return Some((sec_start + start, sec_start + end));
        }
    }
    None
}

/// (start, end, inner_start, inner_end) of the ```yaml fence or `---` block
/// inside a unit region — offsets relative to `region`.
pub fn find_meta_block_span(region: &str) -> Option<(usize, usize, usize, usize)> {
    // ```yaml / ```yml fence
    for tag in ["```yaml", "```yml"] {
        let mut search_from = 0;
        while let Some(rel) = region[search_from..].find(tag) {
            let start = search_from + rel;
            let after_tag = start + tag.len();
            let inner_start = if region[after_tag..].starts_with("\r\n") {
                Some(after_tag + 2)
            } else if region[after_tag..].starts_with('\n') {
                Some(after_tag + 1)
            } else {
                None
            };
            if let Some(inner_start) = inner_start {
                if let Some(close_rel) = region[inner_start..].find("```") {
                    let close = inner_start + close_rel;
                    return Some((start, close + 3, inner_start, close));
                }
                return None;
            }
            search_from = after_tag;
        }
    }
    // --- block at a line start
    let open = find_fence_line(region, 0)?;
    let close = find_fence_line(region, open.1)?;
    Some((open.0, close.1, open.1, close.0))
}

/// Find `^---\r?\n` at or after `from`; returns (line_start, end_after_newline).
fn find_fence_line(text: &str, from: usize) -> Option<(usize, usize)> {
    let bytes = text.as_bytes();
    let mut pos = from;
    loop {
        let rest = &text[pos..];
        if let Some(r) = rest.strip_prefix("---") {
            if let Some(r2) = r.strip_prefix("\r\n") {
                return Some((pos, text.len() - r2.len()));
            }
            if let Some(r2) = r.strip_prefix('\n') {
                return Some((pos, text.len() - r2.len()));
            }
        }
        // advance to the next line start
        let next_nl = bytes[pos..].iter().position(|&b| b == b'\n')?;
        pos += next_nl + 1;
        if pos >= text.len() {
            return None;
        }
    }
}

fn unit_meta_from_region(region: &str) -> Meta {
    match find_meta_block_span(region) {
        Some((_, _, inner_s, inner_e)) => Meta(yamlish::parse(&region[inner_s..inner_e])),
        None => Meta(Vec::new()),
    }
}

/// The file holding contract `cid` of the bundle at `bundle_root`: the first
/// of sorted `contracts/C-###-*.md`, else `contracts/C-###.md` (the
/// resolution rule — the trace hashes this file, the reading set names it).
pub fn contract_file(bundle_root: &Path, cid: &str) -> Option<PathBuf> {
    let cdir = bundle_root.join("contracts");
    let mut matches: Vec<PathBuf> = list_dir(&cdir)
        .into_iter()
        .filter(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.starts_with(&format!("{cid}-")) && n.ends_with(".md"))
        })
        .collect();
    let exact = cdir.join(format!("{cid}.md"));
    if exact.exists() {
        matches.push(exact);
    }
    matches.into_iter().next()
}

/// Contract text: bundle — first of sorted `contracts/C-###-*.md` else
/// `contracts/C-###.md`; single — the `### C-###` region.
pub fn contract_text(sdd: &SddHandle, cid: &str) -> Result<Option<String>, CoreError> {
    contract_text_with(sdd, cid, &read_text)
}

/// [`contract_text`], each file's text read through `read`: what the
/// contract would be with files rewritten that are not written yet.
pub fn contract_text_with(
    sdd: &SddHandle,
    cid: &str,
    read: &dyn Fn(&Path) -> Result<String, CoreError>,
) -> Result<Option<String>, CoreError> {
    match sdd.shape {
        Shape::Bundle => match contract_file(&sdd.root, cid) {
            Some(p) => Ok(Some(read(&p)?)),
            None => Ok(None),
        },
        Shape::Single => {
            let text = read(&sdd.file)?;
            for (ident, region) in split_h3_regions(&text, "Interface Contracts") {
                if ident == cid {
                    return Ok(Some(region.to_string()));
                }
            }
            Ok(None)
        }
    }
}

// --------------------------------------------------------- metadata rewrite

/// Apply `edit` to the unit's metadata-block inner text and write the file
/// back, preserving every byte outside the block.
/// `edit` receives the inner block and returns the new inner block
/// (newline-terminated).
pub fn rewrite_unit_meta(
    sdd: &SddHandle,
    unit: &Unit,
    edit: impl FnOnce(&str) -> String,
) -> Result<PathBuf, CoreError> {
    match sdd.shape {
        Shape::Bundle => {
            let text = read_text(&unit.file)?;
            let Some((inner, body)) = yamlish::split_front_matter(&text) else {
                return Err(CoreError::refusal(
                    1,
                    format!("error: no front matter in {}", unit.file.display()),
                ));
            };
            let new_block = edit(inner);
            let new_text = format!("---\n{new_block}---\n{body}");
            write_text(&unit.file, &new_text)?;
            Ok(unit.file.clone())
        }
        Shape::Single => {
            let text = read_text(&sdd.file)?;
            let Some(span) = find_unit_span(&text, &unit.unit) else {
                return Err(CoreError::refusal(
                    1,
                    format!("error: unit region not found: {}", unit.unit),
                ));
            };
            let region = &text[span.0..span.1];
            let Some((_, _, inner_s, inner_e)) = find_meta_block_span(region) else {
                return Err(CoreError::refusal(
                    1,
                    format!("error: no metadata block in region {}", unit.unit),
                ));
            };
            let new_inner = edit(&region[inner_s..inner_e]);
            let new_region = format!("{}{}{}", &region[..inner_s], new_inner, &region[inner_e..]);
            let new_text = format!("{}{}{}", &text[..span.0], new_region, &text[span.1..]);
            write_text(&sdd.file, &new_text)?;
            Ok(sdd.file.clone())
        }
    }
}

// --------------------------------------------------------------- timestamps

/// UTC timestamp, seconds since the epoch; parses the kit's accepted formats
/// and formats `%Y-%m-%dT%H:%M:%SZ`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct UtcTime(pub i64);

impl UtcTime {
    pub fn now() -> UtcTime {
        let secs = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0);
        UtcTime(secs)
    }

    pub fn add_minutes(self, minutes: i64) -> UtcTime {
        UtcTime(self.0 + minutes * 60)
    }

    /// The kit's `parse_when`: `%Y-%m-%dT%H:%M:%SZ`, `%Y-%m-%dT%H:%MZ`,
    /// `%Y-%m-%d %H:%M:%SZ`, then a basic ISO-8601 fallback. Empty markers
    /// parse to `None`.
    pub fn parse(ts: &str) -> Option<UtcTime> {
        let ts = ts.trim();
        if is_empty_marker(ts) {
            return None;
        }
        // date part
        let b = ts.as_bytes();
        if b.len() < 10 || b[4] != b'-' || b[7] != b'-' {
            return None;
        }
        let year: i64 = ts.get(0..4)?.parse().ok()?;
        let month: i64 = ts.get(5..7)?.parse().ok()?;
        let day: i64 = ts.get(8..10)?.parse().ok()?;
        if !(1..=12).contains(&month) || !(1..=31).contains(&day) {
            return None;
        }
        let rest = &ts[10..];
        let (mut hour, mut minute, mut second, mut offset) = (0i64, 0i64, 0i64, 0i64);
        if !rest.is_empty() {
            let rest = rest.strip_prefix(['T', ' '])?;
            let (time_part, tz_part) = match rest.find(['Z', '+']) {
                Some(i) => (&rest[..i], &rest[i..]),
                None => match rest.rfind('-') {
                    Some(i) if i > 0 => (&rest[..i], &rest[i..]),
                    _ => (rest, ""),
                },
            };
            let mut it = time_part.split(':');
            hour = it.next()?.parse().ok()?;
            minute = it.next()?.parse().ok()?;
            second = match it.next() {
                Some(s) => s.split('.').next()?.parse().ok()?,
                None => 0,
            };
            if !(0..24).contains(&hour) || !(0..60).contains(&minute) || !(0..61).contains(&second)
            {
                return None;
            }
            if !tz_part.is_empty() && tz_part != "Z" {
                let sign = if tz_part.starts_with('-') { -1 } else { 1 };
                let hm = &tz_part[1..];
                let (oh, om) = hm.split_once(':')?;
                offset = sign * (oh.parse::<i64>().ok()? * 3600 + om.parse::<i64>().ok()? * 60);
            }
        }
        let days = days_from_civil(year, month, day);
        Some(UtcTime(
            days * 86400 + hour * 3600 + minute * 60 + second - offset,
        ))
    }

    /// `%Y-%m-%dT%H:%M:%SZ`
    pub fn format(self) -> String {
        let (y, m, d, hh, mm, ss) = self.civil();
        format!("{y:04}-{m:02}-{d:02}T{hh:02}:{mm:02}:{ss:02}Z")
    }

    /// `%Y-%m-%d`
    pub fn format_date(self) -> String {
        let (y, m, d, _, _, _) = self.civil();
        format!("{y:04}-{m:02}-{d:02}")
    }

    fn civil(self) -> (i64, i64, i64, i64, i64, i64) {
        let days = self.0.div_euclid(86400);
        let secs = self.0.rem_euclid(86400);
        let (y, m, d) = civil_from_days(days);
        (y, m, d, secs / 3600, (secs % 3600) / 60, secs % 60)
    }
}

// Howard Hinnant's civil-date algorithms.
fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146097 + doe - 719468
}

fn civil_from_days(z: i64) -> (i64, i64, i64) {
    let z = z + 719468;
    let era = if z >= 0 { z } else { z - 146096 } / 146097;
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (if m <= 2 { y + 1 } else { y }, m, d)
}

// ------------------------------------------------------------- stamp + view

/// Render the stamp `YYYY-MM-DD @ SDD <version>, commit <short-sha>`.
pub fn render_stamp(date: &str, version: &str, commit: &str) -> String {
    format!("{date} @ SDD {version}, commit {commit}")
}

/// Extract the SDD version from a stamp (`@ SDD\s+(\S+?),`); `—` when absent.
pub fn stamp_version(stamp: &str) -> String {
    let mut search = stamp;
    while let Some(i) = search.find("@ SDD") {
        let after = &search[i + 5..];
        let ws = after.len() - after.trim_start().len();
        if ws > 0 {
            let token = &after[ws..];
            if let Some(end) = token.find(|c: char| c.is_whitespace() || c == ',')
                && token[end..].starts_with(',')
                && end > 0
            {
                return token[..end].to_string();
            }
        }
        search = &search[i + 5..];
    }
    "—".to_string()
}

/// A `depends_on` entry that names no unit of the SDD, as the caller's
/// resolver gave it: the units of the repository it waits on — for a pin,
/// the units on its authority's `Producer:` line — each named
/// `<slug> <unit>`, with its status.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Through {
    pub entry: String,
    pub units: Vec<(String, String)>,
}

/// What a caller knows of `depends_on` entries that name no unit of the
/// SDD: `Some(units)` with each unit's `(<slug> <unit>, status)` when the
/// entry resolves, `None` when it does not.
pub type Resolve<'a> = &'a dyn Fn(&str) -> Option<Vec<(String, String)>>;

/// Whether a dependency in this status lets its dependents be claimed.
pub fn is_done(status: &str) -> bool {
    status == "Built" || status == "Verified"
}

/// The computed per-unit view (the kit's `unit_view`): dependency statuses,
/// lease state, and the claimable predicate.
#[derive(Debug, Clone)]
pub struct UnitView {
    pub unit: String,
    pub status: String,
    pub parallel_group: Option<String>,
    /// (dep, status) in listed order; status `MISSING` for unknown deps.
    /// An entry resolved through [`unit_view_with`]'s resolver carries
    /// `Verified` when every unit it waits on is Verified, `Built` when each
    /// is Built or Verified, otherwise the status of the first that is not.
    pub depends_on: Vec<(String, String)>,
    /// The entries of `depends_on` that the resolver resolved, in listed
    /// order, with the units they wait on.
    pub through: Vec<Through>,
    pub claimed_by: Option<String>,
    /// Re-formatted `%Y-%m-%dT%H:%M:%SZ` when parseable.
    pub claim_expires: Option<String>,
    pub claim_seq: i64,
    pub lease_live: bool,
    pub claimable: bool,
    pub file: PathBuf,
}

/// The view of `u` among its SDD's units `all`. A `depends_on` entry that
/// names none of them is `MISSING`, so the unit is not claimable: without
/// the repository's other SDDs nothing says what a pin waits on.
pub fn unit_view(u: &Unit, all: &[Unit], now: UtcTime) -> UnitView {
    unit_view_with(u, all, now, &|_| None)
}

/// [`unit_view`] with `resolve` answering for the `depends_on` entries that
/// name no unit of `all` — a pin, which waits on the units that produce its
/// authority in another SDD. Such an entry is met only when it resolves to
/// at least one unit and every one is Built or Verified; one that does not
/// resolve is `MISSING`, as in [`unit_view`].
pub fn unit_view_with(u: &Unit, all: &[Unit], now: UtcTime, resolve: Resolve) -> UnitView {
    let m = &u.meta;
    let status = m.get_str_or("status", "Draft").to_string();
    let deps = m.get_list("depends_on");
    let mut through: Vec<Through> = Vec::new();
    let dep_status: Vec<(String, String)> = deps
        .iter()
        .map(|d| {
            if let Some(x) = all.iter().find(|x| &x.unit == d) {
                return (d.clone(), x.meta.get_str_or("status", "Draft").to_string());
            }
            let s = match resolve(d) {
                Some(units) if !units.is_empty() => {
                    let s = match units.iter().find(|(_, s)| !is_done(s)) {
                        Some((_, s)) => s.clone(),
                        None if units.iter().all(|(_, s)| s == "Verified") => "Verified".into(),
                        None => "Built".into(),
                    };
                    through.push(Through {
                        entry: d.clone(),
                        units,
                    });
                    s
                }
                _ => "MISSING".to_string(),
            };
            (d.clone(), s)
        })
        .collect();
    let deps_ok = dep_status.iter().all(|(_, s)| is_done(s));
    let expires = UtcTime::parse(m.get_str_or("claim_expires", ""));
    let lease_live = status == "Claimed" && expires.is_some_and(|e| e > now);
    let claimable =
        deps_ok && (status == "Ready" || status == "Stale" || (status == "Claimed" && !lease_live));
    let claimed_by = match m.get_str_or("claimed_by", "") {
        s if is_empty_marker(s) => None,
        s => Some(s.to_string()),
    };
    let parallel_group = match m.get_str_or("parallel_group", "") {
        s if is_empty_marker(s) => None,
        s => Some(s.to_string()),
    };
    UnitView {
        unit: u.unit.clone(),
        status,
        parallel_group,
        depends_on: dep_status,
        through,
        claimed_by,
        claim_expires: expires.map(UtcTime::format),
        claim_seq: m.get_int("claim_seq"),
        lease_live,
        claimable,
        file: u.file.clone(),
    }
}

/// The claimable predicate alone.
pub fn is_claimable(u: &Unit, all: &[Unit], now: UtcTime) -> bool {
    unit_view(u, all, now).claimable
}

/// The claimable predicate alone, with [`unit_view_with`]'s resolver.
pub fn is_claimable_with(u: &Unit, all: &[Unit], now: UtcTime, resolve: Resolve) -> bool {
    unit_view_with(u, all, now, resolve).claimable
}
