//! Find, quick open, go to declaration, and the Explore information browser.
//!
//! The rules match the browser editor. Names are matched without case.
//! Find compares ASCII letters without case, and matches do not overlap.

use std::collections::{BTreeMap, BTreeSet};

use serde_json::Value;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct QuickItem {
    pub kind: String,
    pub label: String,
    pub detail: String,
    pub file: String,
    pub line: i32,
    pub entity_id: Option<i64>,
}

pub fn find_spans(text: &str, query: &str) -> Vec<(usize, usize)> {
    let needle: Vec<char> = query.chars().collect();
    if needle.is_empty() {
        return Vec::new();
    }
    let chars: Vec<char> = text.chars().collect();
    let mut spans = Vec::new();
    let mut i = 0usize;
    while i + needle.len() <= chars.len() {
        let same = chars[i..i + needle.len()]
            .iter()
            .zip(&needle)
            .all(|(hay, pat)| hay.eq_ignore_ascii_case(pat));
        if same {
            spans.push((i, i + needle.len()));
            i += needle.len();
        } else {
            i += 1;
        }
    }
    spans
}

/// Char index of the match at or after `from`. Wraps to the first match.
pub fn find_from(spans: &[(usize, usize)], from: usize) -> Option<usize> {
    spans
        .iter()
        .find(|(start, _)| *start >= from)
        .or_else(|| spans.first())
        .map(|(start, _)| *start)
}

/// Step to another match. An unknown current match with a positive step starts at the first.
pub fn find_move(spans: &[(usize, usize)], current: Option<usize>, dir: i32) -> Option<usize> {
    if spans.is_empty() {
        return None;
    }
    let len = spans.len() as i32;
    let idx = current
        .and_then(|at| spans.iter().position(|(start, _)| *start == at))
        .map(|i| i as i32)
        .unwrap_or(if dir < 0 { 0 } else { -1 });
    let next = (idx + dir).rem_euclid(len) as usize;
    Some(spans[next].0)
}

/// The word touching a character index. The index may sit just after the word.
pub fn word_at(text: &str, index: usize) -> Option<(usize, usize, String)> {
    let chars: Vec<char> = text.chars().collect();
    if chars.is_empty() {
        return None;
    }
    let mut a = index.min(chars.len());
    let is_word = |ch: char| ch.is_ascii_alphanumeric() || ch == '_';
    let here = (a < chars.len()).then(|| chars[a]);
    if a > 0 && !here.is_some_and(is_word) && is_word(chars[a - 1]) {
        a -= 1;
    }
    let mut b = a;
    while a > 0 && is_word(chars[a - 1]) {
        a -= 1;
    }
    while b < chars.len() && is_word(chars[b]) {
        b += 1;
    }
    if a == b {
        return None;
    }
    Some((a, b, chars[a..b].iter().collect()))
}

pub fn declaration_id(entities: &[Value], refs: &[Value], file: &str, line: i64, word: &str) -> Option<i64> {
    if word.is_empty() {
        return None;
    }
    let named: Vec<&Value> = entities.iter().filter(|ent| name_is(ent, word)).collect();
    for kind in ["native", "function", "procedure", "form", "unit", "widget", "builtin"] {
        let mut hits = named.iter().copied().filter(|ent| kind_is(ent, kind));
        let Some(first) = hits.next() else {
            continue;
        };
        if hits.next().is_none() {
            return Some(ident(first));
        }
    }
    let at = entity_at_caret(entities, refs, file, line, word)?;
    if name_is(at, word) { Some(ident(at)) } else { None }
}

/// The entity under the caret: a reference on this line, the declaration, or the enclosing routine.
pub fn entity_at_caret<'a>(entities: &'a [Value], refs: &[Value], file: &str, line: i64, word: &str) -> Option<&'a Value> {
    if !word.is_empty() {
        let named: Vec<&Value> = entities
            .iter()
            .filter(|ent| name_is(ent, word) || qname_is(ent, word))
            .collect();
        if let Some(ent) = named.iter().copied().find(|ent| mentioned(refs, file, line, ident(ent))) {
            return Some(ent);
        }
        if let Some(ent) = named.iter().copied().find(|ent| file_is(ent, file) && line_of(ent) == line) {
            return Some(ent);
        }
        if let Some(ent) = named.iter().copied().find(|ent| {
            matches!(kind_of(ent), "procedure" | "function" | "native" | "builtin" | "widget")
        }) {
            let kind = kind_of(ent);
            let same = named.iter().filter(|ent| kind_of(ent) == kind && name_is(ent, word)).count();
            if same == 1 {
                return Some(ent);
            }
        }
    }
    let mut covers: Vec<&Value> = entities
        .iter()
        .filter(|ent| {
            file_is(ent, file)
                && line_of(ent) <= line
                && end_line(ent) >= line
                && matches!(kind_of(ent), "procedure" | "function" | "native")
        })
        .collect();
    covers.sort_by_key(|ent| end_line(ent) - line_of(ent));
    covers.first().copied()
}

pub fn context_id(entities: &[Value], refs: &[Value], file: &str, line: i64, word: &str) -> Option<i64> {
    entity_at_caret(entities, refs, file, line, word).map(ident)
}

pub fn quick_items(query: &str, files: &[String], entities: &[Value]) -> Vec<QuickItem> {
    let q = query.trim().to_ascii_lowercase();
    let mut ranked: Vec<(i32, i32, i64, QuickItem)> = Vec::new();
    for file in files {
        let name = file.rsplit('/').next().unwrap_or(file);
        let score = quick_score(&q, name, file, name);
        if !q.is_empty() && score == 0 {
            continue;
        }
        ranked.push((
            score,
            quick_rank("file"),
            0,
            QuickItem {
                kind: "file".into(),
                label: name.to_string(),
                detail: file.clone(),
                file: file.clone(),
                line: 1,
                entity_id: None,
            },
        ));
    }
    for ent in entities {
        let kind = kind_of(ent);
        if kind == "native-file" {
            continue;
        }
        if q.is_empty() && matches!(kind, "param" | "local" | "builtin") {
            continue;
        }
        let name = str_of(ent, "name");
        let qname = str_of(ent, "qname");
        let score = quick_score(&q, name, "", qname);
        if !q.is_empty() && score == 0 {
            continue;
        }
        if q.is_empty() && !matches!(kind, "procedure" | "function" | "native" | "form" | "unit") {
            continue;
        }
        let file = str_of(ent, "file").to_string();
        let line = line_of(ent).max(1) as i32;
        let label = if qname.is_empty() { name.to_string() } else { qname.to_string() };
        let detail = if file.is_empty() { String::new() } else { format!("{file}:{line}") };
        let fan_in = ent.get("fanIn").and_then(|v| v.as_i64()).unwrap_or(0);
        ranked.push((
            score,
            quick_rank(kind),
            fan_in,
            QuickItem {
                kind: kind.to_string(),
                label,
                detail,
                file,
                line,
                entity_id: Some(ident(ent)),
            },
        ));
    }
    if q.is_empty() {
        let mut files: Vec<QuickItem> = ranked
            .iter()
            .filter(|row| row.3.kind == "file")
            .map(|row| row.3.clone())
            .collect();
        let mut routines: Vec<(i64, QuickItem)> = ranked
            .iter()
            .filter(|row| row.3.kind != "file")
            .map(|row| (row.2, row.3.clone()))
            .collect();
        files.sort_by(|a, b| a.label.cmp(&b.label));
        routines.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.label.cmp(&b.1.label)));
        files.extend(routines.into_iter().map(|(_, item)| item));
        files.truncate(40);
        return files;
    }
    ranked.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)).then(a.3.label.cmp(&b.3.label)));
    ranked.into_iter().take(40).map(|(_, _, _, item)| item).collect()
}

fn quick_score(query: &str, name: &str, path: &str, qname: &str) -> i32 {
    if query.is_empty() {
        return 1;
    }
    let label = name.to_ascii_lowercase();
    let path = path.to_ascii_lowercase();
    let qualified = qname.to_ascii_lowercase();
    if label == query || qualified == query {
        return 100;
    }
    if label.starts_with(query) {
        return 80;
    }
    if query.contains('.') && qualified.contains(query) {
        return 70;
    }
    if label.contains(query) {
        return 55;
    }
    let slash = format!("/{query}");
    if path == query || path.ends_with(&slash) {
        return 50;
    }
    if path.contains(query) {
        return 35;
    }
    0
}

fn quick_rank(kind: &str) -> i32 {
    match kind {
        "native" | "function" | "procedure" => 0,
        "form" | "unit" => 1,
        "widget" => 2,
        "file" => 3,
        _ => 4,
    }
}

fn str_of<'a>(v: &'a Value, key: &str) -> &'a str {
    v.get(key).and_then(|x| x.as_str()).unwrap_or("")
}

fn ident(v: &Value) -> i64 {
    v.get("id").and_then(|x| x.as_i64()).unwrap_or(0)
}

fn kind_of(v: &Value) -> &str {
    str_of(v, "kind")
}

fn kind_is(v: &Value, kind: &str) -> bool {
    kind_of(v) == kind
}

fn name_is(v: &Value, word: &str) -> bool {
    str_of(v, "name").eq_ignore_ascii_case(word)
}

fn qname_is(v: &Value, word: &str) -> bool {
    str_of(v, "qname").eq_ignore_ascii_case(word)
}

fn file_is(v: &Value, file: &str) -> bool {
    str_of(v, "file") == file
}

fn line_of(v: &Value) -> i64 {
    v.get("line").and_then(|x| x.as_i64()).unwrap_or(1)
}

fn end_line(v: &Value) -> i64 {
    match v.get("endLine").and_then(|x| x.as_i64()) {
        Some(end) if end > 0 => end,
        _ => line_of(v),
    }
}

/// One cross-reference as the information browser shows it: the other entity,
/// where it happens, and the source line at that place.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Site {
    pub other_id: i64,
    pub other_name: String,
    pub file: String,
    pub line: i32,
    pub context: String,
}

/// A line in a project file that contains the search text.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FileHit {
    pub file: String,
    pub line: i32,
    pub context: String,
}

/// Matches across project buffers. Fewer than two letters matches nothing.
/// The list stops at 40 lines. Files are visited in path order.
pub fn project_hits(buffers: &BTreeMap<String, String>, query: &str) -> Vec<FileHit> {
    let query = query.trim();
    if query.chars().count() < 2 {
        return Vec::new();
    }
    let needle = query.to_ascii_lowercase();
    let mut hits = Vec::new();
    'files: for (file, text) in buffers {
        for (index, line) in text.lines().enumerate() {
            if !line.to_ascii_lowercase().contains(&needle) {
                continue;
            }
            let line_no = (index as i32) + 1;
            hits.push(FileHit {
                file: file.clone(),
                line: line_no,
                context: source_line(text, line_no),
            });
            if hits.len() == 40 {
                break 'files;
            }
        }
    }
    hits
}

/// The source line, trimmed. A long line keeps its first 96 characters.
pub fn source_line(text: &str, line: i32) -> String {
    if line < 1 {
        return String::new();
    }
    text.lines()
        .nth(line as usize - 1)
        .unwrap_or("")
        .trim()
        .chars()
        .take(96)
        .collect()
}

/// References of `kind` that touch `focus`. Outgoing ones start at the focus
/// (calls, reads, writes). Incoming ones end at the focus (called by, read by).
/// A procedure, function, or native `fun` in source order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Routine {
    pub name: String,
    pub line: i32,
}

fn keyword_rest<'a>(line: &'a str, keyword: &str) -> Option<&'a str> {
    if line.len() < keyword.len() || !line[..keyword.len()].eq_ignore_ascii_case(keyword) {
        return None;
    }
    let rest = &line[keyword.len()..];
    if rest.starts_with(|ch: char| ch.is_ascii_whitespace()) {
        Some(rest.trim_start())
    } else {
        None
    }
}

fn routine_on_line(line: &str) -> Option<&str> {
    let trimmed = line.trim();
    if trimmed.starts_with("//") || trimmed.starts_with('{') || trimmed.starts_with("(*") {
        return None;
    }
    let rest = keyword_rest(trimmed, "procedure")
        .or_else(|| keyword_rest(trimmed, "function"))
        .or_else(|| keyword_rest(trimmed, "fun"))?;
    let end = rest
        .find(|ch: char| !(ch.is_ascii_alphanumeric() || ch == '_'))
        .unwrap_or(rest.len());
    if end == 0 {
        None
    } else {
        Some(&rest[..end])
    }
}

/// Procedures, functions, and `fun` routines. A comment line is skipped.
/// Each routine owns the lines from its header up to the next header.
pub fn routines(text: &str) -> Vec<Routine> {
    let mut found = Vec::new();
    for (index, line) in text.lines().enumerate() {
        if let Some(name) = routine_on_line(line) {
            found.push(Routine {
                name: name.to_string(),
                line: (index as i32) + 1,
            });
        }
    }
    found
}

/// The routine that contains this 1-based line. Lines above the first header
/// belong to no routine.
pub fn routine_index(routines: &[Routine], line: i32) -> Option<usize> {
    let mut found = None;
    for (index, routine) in routines.iter().enumerate() {
        if routine.line <= line {
            found = Some(index);
        } else {
            break;
        }
    }
    found
}

pub fn sites(
    focus: i64,
    kind: &str,
    outgoing: bool,
    refs: &[Value],
    names: &BTreeMap<i64, String>,
    buffers: &BTreeMap<String, String>,
) -> Vec<Site> {
    let mut rows = Vec::new();
    for xref in refs {
        if str_of(xref, "kind") != kind {
            continue;
        }
        let src = xref.get("src").and_then(|v| v.as_i64()).unwrap_or(-1);
        let dst = xref.get("dst").and_then(|v| v.as_i64()).unwrap_or(-1);
        let (hit, other) = if outgoing {
            (src == focus, dst)
        } else {
            (dst == focus, src)
        };
        if !hit {
            continue;
        }
        let file = str_of(xref, "file").to_string();
        let line = xref.get("line").and_then(|v| v.as_i64()).unwrap_or(1).max(1) as i32;
        let context = buffers
            .get(&file)
            .map(|text| source_line(text, line))
            .unwrap_or_default();
        rows.push(Site {
            other_id: other,
            other_name: names.get(&other).cloned().unwrap_or_else(|| "?".into()),
            file,
            line,
            context,
        });
    }
    rows.sort_by(|a, b| (&a.file, a.line, &a.other_name).cmp(&(&b.file, b.line, &b.other_name)));
    rows
}

/// One name in the next call hop. Several call sites of the same entity count once.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HopName {
    pub id: i64,
    pub name: String,
}

/// The calls and callers of one entity, one hop past the focused row.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CallHop {
    pub calls: Vec<HopName>,
    pub called_by: Vec<HopName>,
}

/// Call edges that touch `id`. A self-call is listed on both sides. Other kinds are skipped.
pub fn call_hop(id: i64, refs: &[Value], names: &BTreeMap<i64, String>) -> CallHop {
    let mut calls = BTreeSet::new();
    let mut called_by = BTreeSet::new();
    for xref in refs {
        if str_of(xref, "kind") != "call" {
            continue;
        }
        let src = xref.get("src").and_then(|v| v.as_i64()).unwrap_or(-1);
        let dst = xref.get("dst").and_then(|v| v.as_i64()).unwrap_or(-1);
        if src == id {
            calls.insert(dst);
        }
        if dst == id {
            called_by.insert(src);
        }
    }
    CallHop {
        calls: hop_names(calls, names),
        called_by: hop_names(called_by, names),
    }
}

fn hop_names(ids: BTreeSet<i64>, names: &BTreeMap<i64, String>) -> Vec<HopName> {
    let mut rows: Vec<HopName> = ids
        .into_iter()
        .map(|id| HopName {
            name: names.get(&id).cloned().unwrap_or_else(|| "?".into()),
            id,
        })
        .collect();
    rows.sort_by(|a, b| a.name.cmp(&b.name).then(a.id.cmp(&b.id)));
    rows
}

/// Opened steps past the focused entity. The next name after this is listed and cannot be opened.
pub const CALL_TREE_DEPTH: usize = 4;

/// One name in an opened call chain. Nested lists are filled only while that name is open.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CallBranch {
    pub id: i64,
    pub name: String,
    /// A cycle, or a name past [`CALL_TREE_DEPTH`], stays closed.
    pub expandable: bool,
    /// This name was expanded, so `calls` and `called_by` are its next hop.
    pub open: bool,
    pub calls: Vec<CallBranch>,
    pub called_by: Vec<CallBranch>,
}

/// Both sides of one opened entity.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CallSides {
    pub calls: Vec<CallBranch>,
    pub called_by: Vec<CallBranch>,
}

/// Children of `id`. `depth` is how far `id` sits past the focused entity.
/// `trail` already contains `id`. An open child expands when `depth` is below
/// [`CALL_TREE_DEPTH`] and the child is not already on the trail.
pub fn call_branches(
    id: i64,
    open: &BTreeSet<i64>,
    refs: &[Value],
    names: &BTreeMap<i64, String>,
    depth: usize,
    trail: &[i64],
) -> CallSides {
    let hop = call_hop(id, refs, names);
    CallSides {
        calls: branch_list(&hop.calls, open, refs, names, depth, trail),
        called_by: branch_list(&hop.called_by, open, refs, names, depth, trail),
    }
}

fn branch_list(
    rows: &[HopName],
    open: &BTreeSet<i64>,
    refs: &[Value],
    names: &BTreeMap<i64, String>,
    depth: usize,
    trail: &[i64],
) -> Vec<CallBranch> {
    rows.iter()
        .map(|row| {
            let expandable = depth < CALL_TREE_DEPTH && !trail.contains(&row.id);
            let expanded = expandable && open.contains(&row.id);
            let (calls, called_by) = if expanded {
                let mut next = trail.to_vec();
                next.push(row.id);
                let nested = call_branches(row.id, open, refs, names, depth + 1, &next);
                (nested.calls, nested.called_by)
            } else {
                (Vec::new(), Vec::new())
            };
            CallBranch {
                id: row.id,
                name: row.name.clone(),
                expandable,
                open: expanded,
                calls,
                called_by,
            }
        })
        .collect()
}

/// One control-flow block. `text` joins the statement lines the compiler stored.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FlowStep {
    pub id: usize,
    pub label: String,
    pub line: i32,
    pub text: String,
    pub succs: Vec<usize>,
}

pub fn flow_steps(cfg: &Value) -> (usize, Vec<FlowStep>) {
    let entry = cfg.get("entry").and_then(|v| v.as_u64()).unwrap_or(0) as usize;
    let Some(blocks) = cfg.get("blocks").and_then(|v| v.as_array()) else {
        return (entry, Vec::new());
    };
    let steps = blocks
        .iter()
        .enumerate()
        .map(|(i, block)| {
            let id = block.get("id").and_then(|v| v.as_u64()).unwrap_or(i as u64) as usize;
            let line = block.get("line").and_then(|v| v.as_i64()).unwrap_or(1).max(1) as i32;
            let text = match block.get("text") {
                Some(Value::String(text)) => text.trim().to_string(),
                Some(Value::Array(lines)) => lines
                    .iter()
                    .filter_map(|line| line.as_str())
                    .filter(|line| !line.is_empty())
                    .collect::<Vec<_>>()
                    .join(" · "),
                _ => String::new(),
            };
            let succs = block
                .get("succs")
                .and_then(|v| v.as_array())
                .map(|edges| edges.iter().filter_map(|v| v.as_u64()).map(|n| n as usize).collect())
                .unwrap_or_default();
            FlowStep {
                id,
                label: str_of(block, "label").to_string(),
                line,
                text,
                succs,
            }
        })
        .collect();
    (entry, steps)
}

/// A block placed in the control-flow graph. Row 0 is the entry.
/// A branch puts its successors side by side on the next row.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FlowNode {
    pub id: usize,
    pub col: i32,
    pub row: i32,
    pub label: String,
    pub line: i32,
    pub text: String,
}

/// An edge. `back` is a loop or a jump to an earlier block.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FlowLink {
    pub from: usize,
    pub to: usize,
    pub back: bool,
}

pub fn flow_layout(entry: usize, steps: &[FlowStep]) -> (Vec<FlowNode>, Vec<FlowLink>) {
    use std::collections::VecDeque;
    if steps.is_empty() {
        return (Vec::new(), Vec::new());
    }
    let mut by_id: BTreeMap<usize, &FlowStep> = BTreeMap::new();
    for step in steps {
        by_id.entry(step.id).or_insert(step);
    }
    let start = if by_id.contains_key(&entry) { entry } else { steps[0].id };
    let mut row_of: BTreeMap<usize, i32> = BTreeMap::new();
    let mut queue = VecDeque::new();
    row_of.insert(start, 0);
    queue.push_back(start);
    while let Some(id) = queue.pop_front() {
        let depth = row_of[&id];
        let Some(step) = by_id.get(&id) else {
            continue;
        };
        for succ in &step.succs {
            if !by_id.contains_key(succ) || row_of.contains_key(succ) {
                continue;
            }
            row_of.insert(*succ, depth + 1);
            queue.push_back(*succ);
        }
    }
    let mut extra = row_of.values().copied().max().unwrap_or(0) + 1;
    for step in steps {
        if !row_of.contains_key(&step.id) {
            row_of.insert(step.id, extra);
            extra += 1;
        }
    }
    let mut bands: BTreeMap<i32, Vec<usize>> = BTreeMap::new();
    for (id, row) in &row_of {
        bands.entry(*row).or_default().push(*id);
    }
    let mut col_of: BTreeMap<usize, i32> = BTreeMap::new();
    for ids in bands.values_mut() {
        ids.sort_unstable();
        for (col, id) in ids.iter().enumerate() {
            col_of.insert(*id, col as i32);
        }
    }
    let nodes = steps
        .iter()
        .map(|step| FlowNode {
            id: step.id,
            col: col_of[&step.id],
            row: row_of[&step.id],
            label: step.label.clone(),
            line: step.line,
            text: step.text.clone(),
        })
        .collect();
    let mut links = Vec::new();
    for step in steps {
        let from_row = row_of[&step.id];
        for succ in &step.succs {
            let Some(to_row) = row_of.get(succ) else {
                continue;
            };
            links.push(FlowLink {
                from: step.id,
                to: *succ,
                back: *to_row <= from_row,
            });
        }
    }
    (nodes, links)
}

/// One entity in the Explore index, after the file header it belongs under.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IndexEntry {
    pub id: i64,
    pub kind: String,
    pub name: String,
    pub qname: String,
    pub complexity: i64,
    pub fan_in: i64,
}

/// Entities of one source file, in kind then name order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IndexGroup {
    pub file: String,
    pub entries: Vec<IndexEntry>,
}

/// Group the index by file. An empty query keeps the form surface. A query
/// also keeps locals and builtins whose name matches.
pub fn index_groups(entities: &[Value], query: &str) -> Vec<IndexGroup> {
    let query = query.to_ascii_lowercase();
    let mut entries = Vec::new();
    for ent in entities {
        let kind = str_of(ent, "kind");
        if kind == "native-file" {
            continue;
        }
        let name = str_of(ent, "name");
        let qname = str_of(ent, "qname");
        let visible = if query.is_empty() {
            matches!(kind, "procedure" | "function" | "native" | "widget" | "form" | "unit")
        } else {
            name.to_ascii_lowercase().contains(&query) || qname.to_ascii_lowercase().contains(&query)
        };
        if !visible {
            continue;
        }
        let file = str_of(ent, "file");
        entries.push((
            if file.is_empty() { "(project)".to_string() } else { file.to_string() },
            IndexEntry {
                id: ent.get("id").and_then(|v| v.as_i64()).unwrap_or(0),
                kind: kind.to_string(),
                name: name.to_string(),
                qname: qname.to_string(),
                complexity: ent.get("complexity").and_then(|v| v.as_i64()).unwrap_or(0),
                fan_in: ent.get("fanIn").and_then(|v| v.as_i64()).unwrap_or(0),
            },
        ));
    }
    entries.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.kind.cmp(&b.1.kind)).then(a.1.qname.cmp(&b.1.qname)));
    let mut groups: Vec<IndexGroup> = Vec::new();
    for (file, entry) in entries {
        if groups.last().map(|group| group.file.as_str()) != Some(file.as_str()) {
            groups.push(IndexGroup { file, entries: Vec::new() });
        }
        groups.last_mut().unwrap().entries.push(entry);
    }
    groups
}

fn mentioned(refs: &[Value], file: &str, line: i64, id: i64) -> bool {
    refs.iter().any(|r| {
        str_of(r, "file") == file
            && r.get("line").and_then(|x| x.as_i64()).unwrap_or(0) == line
            && (r.get("dst").and_then(|x| x.as_i64()) == Some(id) || r.get("src").and_then(|x| x.as_i64()) == Some(id))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn find_is_case_insensitive_and_does_not_overlap() {
        assert_eq!(find_spans("Fib fib FIB", "fib"), vec![(0, 3), (4, 7), (8, 11)]);
        assert_eq!(find_spans("aaa", "aa"), vec![(0, 2)]);
        assert!(find_spans("abc", "").is_empty());
        assert!(find_spans("", "a").is_empty());
    }

    #[test]
    fn find_moves_and_wraps() {
        let spans = [(0, 3), (10, 13), (20, 23)];
        assert_eq!(find_move(&spans, Some(10), 1), Some(20));
        assert_eq!(find_move(&spans, Some(20), 1), Some(0));
        assert_eq!(find_move(&spans, Some(0), -1), Some(20));
        assert_eq!(find_move(&spans, None, 1), Some(0));
        assert_eq!(find_move(&spans, None, -1), Some(20));
        assert_eq!(find_from(&spans, 11), Some(20));
        assert_eq!(find_from(&spans, 21), Some(0));
    }

    #[test]
    fn word_at_reads_the_identifier_under_or_just_after_the_caret() {
        let src = "Call fib(n)\n";
        assert_eq!(word_at(src, 6).unwrap().2, "fib");
        assert_eq!(word_at(src, 8).unwrap().2, "fib");
        assert_eq!(word_at(src, 4).unwrap().2, "Call");
        assert_eq!(word_at(src, 9).unwrap().2, "n");
        assert!(word_at("   ", 1).is_none());
    }

    fn sample() -> (Vec<Value>, Vec<Value>) {
        let entities = vec![
            json!({"id": 1, "kind": "procedure", "name": "FibClick", "qname": "Main.FibClick", "file": "forms/Main.droid", "line": 4, "endLine": 10, "fanIn": 1}),
            json!({"id": 2, "kind": "native", "name": "fib", "qname": "native.fib", "file": "native/fib.nc", "line": 1, "endLine": 6, "fanIn": 2}),
            json!({"id": 3, "kind": "builtin", "name": "IntToStr", "qname": "IntToStr", "file": "", "line": 1, "fanIn": 0}),
            json!({"id": 4, "kind": "procedure", "name": "Foo", "qname": "A.Foo", "file": "forms/A.droid", "line": 1, "endLine": 3, "fanIn": 0}),
            json!({"id": 5, "kind": "procedure", "name": "Foo", "qname": "B.Foo", "file": "forms/B.droid", "line": 1, "endLine": 3, "fanIn": 0}),
        ];
        let refs = vec![json!({"src": 1, "dst": 2, "kind": "call", "file": "forms/Main.droid", "line": 6})];
        (entities, refs)
    }

    #[test]
    fn declaration_prefers_one_native_and_uses_the_caret_when_names_collide() {
        let (entities, refs) = sample();
        assert_eq!(declaration_id(&entities, &refs, "forms/Main.droid", 6, "fib"), Some(2));
        assert_eq!(declaration_id(&entities, &refs, "forms/Main.droid", 6, "IntToStr"), Some(3));
        assert_eq!(declaration_id(&entities, &refs, "forms/Main.droid", 8, "Foo"), None);
        assert_eq!(declaration_id(&entities, &refs, "forms/A.droid", 1, "Foo"), Some(4));
        assert_eq!(context_id(&entities, &refs, "forms/Main.droid", 6, "fib"), Some(2));
        assert_eq!(context_id(&entities, &refs, "forms/Main.droid", 5, ""), Some(1));
    }

    #[test]
    fn quick_open_lists_files_then_routines_and_ranks_a_query() {
        let (entities, _) = sample();
        let files = vec!["forms/Main.droid".into(), "native/fib.nc".into()];
        let idle = quick_items("", &files, &entities);
        let labels: Vec<_> = idle.iter().map(|item| item.label.as_str()).collect();
        assert_eq!(
            labels,
            vec!["Main.droid", "fib.nc", "native.fib", "Main.FibClick", "A.Foo", "B.Foo"]
        );
        let hit = quick_items("fib", &files, &entities);
        assert_eq!(hit[0].label, "native.fib");
        assert_eq!(hit[0].entity_id, Some(2));
        assert!(hit.iter().any(|item| item.kind == "file" && item.label == "fib.nc"));
    }

    #[test]
    fn sites_keep_every_use_and_the_source_line() {
        let refs = vec![
            json!({"src": 1, "dst": 2, "kind": "call", "file": "forms/Main.droid", "line": 5}),
            json!({"src": 7, "dst": 2, "kind": "call", "file": "forms/Main.droid", "line": 2}),
            json!({"src": 1, "dst": 3, "kind": "write", "file": "forms/Main.droid", "line": 3}),
            json!({"src": 1, "dst": 1, "kind": "call", "file": "forms/Main.droid", "line": 4}),
        ];
        let mut names = BTreeMap::new();
        names.insert(1, "Main.ReplaceClick".into());
        names.insert(2, "builtin.Log".into());
        names.insert(3, "Main.Names".into());
        names.insert(7, "Main.Pick".into());
        let mut buffers = BTreeMap::new();
        buffers.insert(
            "forms/Main.droid".into(),
            "procedure Pick(Name: String);\n  Log(Name)\n  Names.Items := LineMemo.Text;\n  ReplaceClick()\n  Log('replaced')\n".into(),
        );
        let calls = sites(1, "call", true, &refs, &names, &buffers);
        assert_eq!(calls.len(), 2);
        assert_eq!(calls[0].other_name, "Main.ReplaceClick");
        assert_eq!(calls[0].context, "ReplaceClick()");
        assert_eq!(calls[1].other_name, "builtin.Log");
        assert_eq!(calls[1].line, 5);
        assert_eq!(calls[1].context, "Log('replaced')");
        let called = sites(2, "call", false, &refs, &names, &buffers);
        assert_eq!(called.len(), 2);
        assert_eq!(called[0].other_name, "Main.Pick");
        assert_eq!(called[0].context, "Log(Name)");
        let writes = sites(1, "write", true, &refs, &names, &buffers);
        assert_eq!(writes.len(), 1);
        assert_eq!(writes[0].other_name, "Main.Names");
        assert_eq!(writes[0].context, "Names.Items := LineMemo.Text;");
        assert!(source_line("  hello\n", 1) == "hello");
        assert!(source_line("x", 4).is_empty());
    }

    #[test]
    fn call_hop_lists_the_next_callers_and_callees() {
        let refs = vec![
            json!({"src": 1, "dst": 2, "kind": "call", "file": "forms/Main.droid", "line": 2}),
            json!({"src": 2, "dst": 3, "kind": "call", "file": "forms/Main.droid", "line": 4}),
            json!({"src": 2, "dst": 3, "kind": "call", "file": "forms/Main.droid", "line": 6}),
            json!({"src": 2, "dst": 2, "kind": "call", "file": "native/fib.nc", "line": 3}),
            json!({"src": 4, "dst": 2, "kind": "call", "file": "forms/Main.droid", "line": 8}),
            json!({"src": 2, "dst": 5, "kind": "write", "file": "forms/Main.droid", "line": 5}),
        ];
        let mut names = BTreeMap::new();
        names.insert(1, "Main.InkClick".into());
        names.insert(2, "Main.ShowColor".into());
        names.insert(3, "Tint.HexByte".into());
        names.insert(4, "Main.ClayClick".into());
        let hop = call_hop(2, &refs, &names);
        let calls: Vec<_> = hop.calls.iter().map(|row| row.name.as_str()).collect();
        let called: Vec<_> = hop.called_by.iter().map(|row| row.name.as_str()).collect();
        assert_eq!(calls, vec!["Main.ShowColor", "Tint.HexByte"]);
        assert_eq!(called, vec!["Main.ClayClick", "Main.InkClick", "Main.ShowColor"]);
        assert_eq!(hop.calls[1].id, 3);
        let leaf = call_hop(3, &refs, &names);
        assert!(leaf.calls.is_empty());
        assert_eq!(leaf.called_by.len(), 1);
        assert_eq!(leaf.called_by[0].name, "Main.ShowColor");
        assert!(call_hop(9, &refs, &names).calls.is_empty());
        assert!(call_hop(9, &refs, &names).called_by.is_empty());
    }

    #[test]
    fn call_branches_follow_open_names_and_stop_on_a_cycle() {
        let refs = vec![
            json!({"src": 1, "dst": 2, "kind": "call"}),
            json!({"src": 2, "dst": 2, "kind": "call"}),
            json!({"src": 2, "dst": 3, "kind": "call"}),
            json!({"src": 3, "dst": 4, "kind": "call"}),
            json!({"src": 4, "dst": 5, "kind": "call"}),
            json!({"src": 5, "dst": 6, "kind": "call"}),
            json!({"src": 6, "dst": 7, "kind": "call"}),
            json!({"src": 2, "dst": 9, "kind": "write"}),
        ];
        let mut names = BTreeMap::new();
        for (id, name) in [(2, "b"), (3, "c"), (4, "d"), (5, "e"), (6, "f"), (7, "g")] {
            names.insert(id, name.into());
        }
        let open = BTreeSet::from([3, 4, 5, 6]);
        let sides = call_branches(2, &open, &refs, &names, 1, &[1, 2]);
        assert_eq!(sides.calls.len(), 2);
        assert_eq!(sides.calls[0].name, "b");
        assert!(!sides.calls[0].expandable);
        assert!(sides.calls[0].calls.is_empty());
        let c = &sides.calls[1];
        assert_eq!(c.name, "c");
        assert!(c.open);
        let e = &c.calls[0].calls[0];
        assert_eq!(e.name, "e");
        assert!(e.open);
        let f = &e.calls[0];
        assert_eq!(f.name, "f");
        assert!(!f.expandable);
        assert!(f.calls.is_empty());
    }

    #[test]
    fn flow_steps_join_statement_lines_and_keep_edges() {
        let cfg = json!({
            "entry": 1,
            "blocks": [
                {"id": 0, "label": "exit", "text": ["exit"], "line": 12, "succs": []},
                {"id": 1, "label": "code", "text": ["Names.Items := LineMemo.Text", "Log('replaced')"], "line": 9, "succs": [0]}
            ]
        });
        let (entry, steps) = flow_steps(&cfg);
        assert_eq!(entry, 1);
        assert_eq!(steps[1].text, "Names.Items := LineMemo.Text · Log('replaced')");
        assert_eq!(steps[1].succs, vec![0]);
        assert_eq!(steps[0].label, "exit");
    }

    fn step(id: usize, label: &str, line: i32, succs: Vec<usize>) -> FlowStep {
        FlowStep {
            id,
            label: label.into(),
            line,
            text: label.into(),
            succs,
        }
    }

    #[test]
    fn flow_layout_places_a_branch_side_by_side_and_marks_a_loop() {
        let straight = vec![step(0, "exit", 12, vec![]), step(1, "code", 9, vec![0])];
        let (nodes, links) = flow_layout(1, &straight);
        let code = nodes.iter().find(|n| n.id == 1).unwrap();
        let exit = nodes.iter().find(|n| n.id == 0).unwrap();
        assert_eq!((code.row, code.col), (0, 0));
        assert_eq!((exit.row, exit.col), (1, 0));
        assert_eq!(links, vec![FlowLink { from: 1, to: 0, back: false }]);

        let branch = vec![
            step(0, "if", 4, vec![1, 2]),
            step(1, "then", 5, vec![3]),
            step(2, "else", 8, vec![3]),
            step(3, "exit", 12, vec![]),
        ];
        let (nodes, links) = flow_layout(0, &branch);
        let at = |id| nodes.iter().find(|n| n.id == id).unwrap();
        assert_eq!((at(0).row, at(0).col), (0, 0));
        assert_eq!((at(1).row, at(1).col), (1, 0));
        assert_eq!((at(2).row, at(2).col), (1, 1));
        assert_eq!((at(3).row, at(3).col), (2, 0));
        assert!(links.iter().all(|link| !link.back));

        let looped = vec![step(0, "while", 3, vec![1, 2]), step(1, "body", 4, vec![0]), step(2, "exit", 9, vec![])];
        let (nodes, links) = flow_layout(0, &looped);
        let at = |id| nodes.iter().find(|n| n.id == id).unwrap();
        assert_eq!(at(0).row, 0);
        assert_eq!(at(1).row, 1);
        assert_eq!(at(2).row, 1);
        assert!(at(1).col != at(2).col);
        assert_eq!(
            links.iter().find(|link| link.from == 1 && link.to == 0).map(|link| link.back),
            Some(true)
        );
    }

    #[test]
    fn index_groups_files_and_hides_locals_until_a_search() {
        let entities = vec![
            json!({"id": 1, "kind": "procedure", "name": "ReplaceClick", "qname": "Main.ReplaceClick", "file": "forms/Main.droid", "complexity": 1, "fanIn": 0}),
            json!({"id": 2, "kind": "widget", "name": "Names", "qname": "Main.Names", "file": "forms/Main.droid", "complexity": 0, "fanIn": 0}),
            json!({"id": 3, "kind": "local", "name": "Name", "qname": "Main.OkClick.Name", "file": "forms/Main.droid", "complexity": 0, "fanIn": 0}),
            json!({"id": 4, "kind": "function", "name": "fib", "qname": "Utils.fib", "file": "units/Utils.droid", "complexity": 4, "fanIn": 1}),
            json!({"id": 5, "kind": "builtin", "name": "Log", "qname": "builtin.Log", "file": "", "complexity": 0, "fanIn": 2}),
            json!({"id": 6, "kind": "native-file", "name": "fib.nc", "qname": "fib.nc", "file": "native/fib.nc", "complexity": 0, "fanIn": 0}),
        ];
        let groups = index_groups(&entities, "");
        assert_eq!(groups.len(), 2);
        assert_eq!(groups[0].file, "forms/Main.droid");
        assert_eq!(
            groups[0].entries.iter().map(|e| e.name.as_str()).collect::<Vec<_>>(),
            vec!["ReplaceClick", "Names"]
        );
        assert_eq!(groups[1].file, "units/Utils.droid");
        assert_eq!(groups[1].entries[0].complexity, 4);
        assert_eq!(groups[1].entries[0].fan_in, 1);
        let found = index_groups(&entities, "log");
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].file, "(project)");
        assert_eq!(found[0].entries[0].name, "Log");
        assert_eq!(found[0].entries[0].fan_in, 2);
    }

    #[test]
    fn project_hits_list_each_line_and_wait_for_two_letters() {
        let mut buffers = BTreeMap::new();
        buffers.insert(
            "forms/Main.droid".into(),
            "procedure Pick(Name: String);\n  Log(Name)\nend;\n".into(),
        );
        buffers.insert("forms/Other.droid".into(), "begin\n  Log('x')\nend;\n".into());
        assert!(project_hits(&buffers, "l").is_empty());
        let hits = project_hits(&buffers, "LoG");
        assert_eq!(hits.len(), 2);
        assert_eq!(hits[0].file, "forms/Main.droid");
        assert_eq!(hits[0].line, 2);
        assert_eq!(hits[0].context, "Log(Name)");
        assert_eq!(hits[1].file, "forms/Other.droid");
        assert_eq!(hits[1].line, 2);
        assert_eq!(hits[1].context, "Log('x')");
    }

    #[test]
    fn routines_follow_the_caret_and_skip_comments() {
        let src = "uses Utils;\n\nprocedure FormCreate(Sender: View);\nbegin\nend;\n\nfunction Fib(N: Integer): Integer;\nbegin\nend;\n\nfun fib(n: i32): i32 {\n}\n";
        let found = routines(src);
        assert_eq!(
            found.iter().map(|r| (r.name.as_str(), r.line)).collect::<Vec<_>>(),
            vec![("FormCreate", 3), ("Fib", 7), ("fib", 11)]
        );
        assert_eq!(routine_index(&found, 1), None);
        assert_eq!(routine_index(&found, 3), Some(0));
        assert_eq!(routine_index(&found, 6), Some(0));
        assert_eq!(routine_index(&found, 8), Some(1));
        assert_eq!(routine_index(&found, 12), Some(2));
        assert!(routines("{ procedure Hidden(); }\n// procedure Also();\n").is_empty());
        assert_eq!(routines("function Fib(N: Integer): Integer;\n")[0].name, "Fib");
        assert_eq!(routines("procedure OkClickExtra();\n")[0].name, "OkClickExtra");
    }
}
