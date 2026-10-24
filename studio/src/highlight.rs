//! Syntax color and block rails for the code view.
//!
//! Pascal `{...}` and `(*...*)` are comments, so a scope is `begin`/`end`.
//! Native `.nc` and C use `{` and `}`. Strings and comments never open a scope.

use eframe::egui::{self, Color32, FontId, Galley, Pos2, Rect, Stroke};

const INK: Color32 = Color32::from_rgb(0x1E, 0x1D, 0x1B);
const KW: Color32 = Color32::from_rgb(0x1D, 0x4E, 0x89);
const STR: Color32 = Color32::from_rgb(0x1D, 0x6B, 0x42);
const CMT: Color32 = Color32::from_rgb(0x9A, 0x94, 0x8C);
const NUM: Color32 = Color32::from_rgb(0x8A, 0x4B, 0x12);
const RAIL: Color32 = Color32::from_rgb(0xC4, 0xBE, 0xB4);
const NUM_MUTED: Color32 = Color32::from_rgb(0xB7, 0xB1, 0xA8);

const RAIL_LEFT: f32 = 8.0;
const RAIL_PITCH: f32 = 8.0;
const RAIL_COLS: usize = 6;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Tone {
    Text,
    Kw,
    Str,
    Cmt,
    Num,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Scope {
    pub open: usize,
    pub close: usize,
    pub depth: usize,
}

/// Which source the code view is showing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Lang {
    Pascal,
    Native,
    C,
}

pub fn source_lang(path: &str) -> Lang {
    match std::path::Path::new(path).extension().and_then(|ext| ext.to_str()) {
        Some("nc") => Lang::Native,
        Some("c" | "cc" | "cpp" | "h" | "hh" | "hpp") => Lang::C,
        _ => Lang::Pascal,
    }
}

/// Character index where a 1-based line begins. A line past the end is the end of the text.
pub fn char_index_of_line(text: &str, line: usize) -> usize {
    if line <= 1 {
        return 0;
    }
    let mut current = 1usize;
    for (index, ch) in text.chars().enumerate() {
        if ch == '\n' {
            current += 1;
            if current == line {
                return index + 1;
            }
        }
    }
    text.chars().count()
}

/// Character index of a 1-based line and column. The column stays on that line.
pub fn char_index_at(text: &str, line: usize, col: usize) -> usize {
    let start = char_index_of_line(text, line);
    let col = col.max(1);
    if col == 1 {
        return start;
    }
    let mut index = start;
    let mut at = 1usize;
    for ch in text.chars().skip(start) {
        if ch == '\n' || at == col {
            break;
        }
        index += 1;
        at += 1;
    }
    index
}

pub fn byte_of_char(text: &str, char_index: usize) -> usize {
    text.chars().take(char_index).map(|ch| ch.len_utf8()).sum()
}

pub fn layout_job(text: &str, lang: Lang, font: FontId, line_h: f32) -> egui::text::LayoutJob {
    layout_job_marked(text, lang, font, line_h, &[], None)
}

/// `hits` and `current` are byte ranges. A find match is painted behind the syntax color.
pub fn layout_job_marked(
    text: &str,
    lang: Lang,
    font: FontId,
    line_h: f32,
    hits: &[(usize, usize)],
    current: Option<(usize, usize)>,
) -> egui::text::LayoutJob {
    let spans = color_spans(text, lang);
    let mut job = egui::text::LayoutJob::default();
    job.break_on_newline = true;
    job.keep_trailing_whitespace = true;
    job.wrap.max_width = f32::INFINITY;
    if text.is_empty() {
        return job;
    }
    let mut at = 0usize;
    for (start, end, tone) in spans {
        if start > at {
            append_piece(&mut job, text, at, start, &font, Tone::Text, line_h, hits, current);
        }
        append_piece(&mut job, text, start, end, &font, tone, line_h, hits, current);
        at = end;
    }
    if at < text.len() {
        append_piece(&mut job, text, at, text.len(), &font, Tone::Text, line_h, hits, current);
    }
    job
}

fn append_piece(
    job: &mut egui::text::LayoutJob,
    text: &str,
    start: usize,
    end: usize,
    font: &FontId,
    tone: Tone,
    line_h: f32,
    hits: &[(usize, usize)],
    current: Option<(usize, usize)>,
) {
    for (a, b, bg) in pieces(start, end, hits, current) {
        job.append(&text[a..b], 0.0, text_format(font.clone(), tone, line_h, bg));
    }
}

fn pieces(
    start: usize,
    end: usize,
    hits: &[(usize, usize)],
    current: Option<(usize, usize)>,
) -> Vec<(usize, usize, Color32)> {
    if start >= end {
        return Vec::new();
    }
    let mut bounds = vec![start, end];
    for (a, b) in hits.iter().copied().chain(current) {
        if b <= start || a >= end {
            continue;
        }
        let a = a.max(start);
        let b = b.min(end);
        if a > start && a < end {
            bounds.push(a);
        }
        if b > start && b < end {
            bounds.push(b);
        }
    }
    bounds.sort_unstable();
    bounds.dedup();
    let mut out = Vec::new();
    for pair in bounds.windows(2) {
        let (a, b) = (pair[0], pair[1]);
        if a >= b {
            continue;
        }
        let bg = if current.is_some_and(|(s, e)| s <= a && a < e) {
            Color32::from_rgb(0xE2, 0xC0, 0x4A)
        } else if hits.iter().any(|(s, e)| *s <= a && a < *e) {
            Color32::from_rgb(0xF3, 0xE2, 0xA8)
        } else {
            Color32::TRANSPARENT
        };
        out.push((a, b, bg));
    }
    out
}

pub fn gutter_width(ui: &egui::Ui, font: &FontId, lines: usize) -> f32 {
    let digits = lines.max(1).to_string().len().max(2) as f32;
    let char_w = ui.fonts_mut(|fonts| fonts.glyph_width(font, '8'));
    let rails = RAIL_LEFT + RAIL_COLS as f32 * RAIL_PITCH + 2.0;
    let nums = digits * char_w + 16.0;
    rails + nums
}

pub fn line_of_char(text: &str, index: usize) -> usize {
    let mut line = 1usize;
    for (i, ch) in text.chars().enumerate() {
        if i >= index {
            break;
        }
        if ch == '\n' {
            line += 1;
        }
    }
    line
}

pub fn paint_gutter(
    painter: &egui::Painter,
    rect: Rect,
    galley: &Galley,
    galley_pos: Pos2,
    text: &str,
    lang: Lang,
    cursor_line: Option<usize>,
    font: FontId,
) {
    let ppp = painter.ctx().pixels_per_point();
    let rows = &galley.rows;
    let cursor0 = cursor_line.map(|line| line.saturating_sub(1));
    if let Some(line) = cursor0 {
        if let Some(row) = rows.get(line) {
            let y = galley_pos.y + row.pos.y - galley.rect.top();
            let h = row.rect().height().max(1.0);
            painter.rect_filled(
                Rect::from_min_size(egui::pos2(rect.left(), y), egui::vec2(rect.width(), h)),
                0.0,
                Color32::from_rgba_unmultiplied(0x1D, 0x4E, 0x89, 22),
            );
        }
    }

    let scopes = block_scopes(text, lang);
    let hot = cursor0.and_then(|line| {
        scopes
            .iter()
            .filter(|scope| scope.open <= line && line <= scope.close)
            .map(|scope| scope.depth)
            .max()
    });
    for scope in &scopes {
        if scope.depth >= RAIL_COLS {
            continue;
        }
        let Some((open_y, open_h)) = row_box(galley, galley_pos, scope.open) else {
            continue;
        };
        let (close_y, close_h) = row_box(galley, galley_pos, scope.close).unwrap_or((open_y, open_h));
        let y_open = open_y + open_h * 0.5;
        let y_close = close_y + close_h * 0.5;
        let x = crisp(rect.left() + RAIL_LEFT + scope.depth as f32 * RAIL_PITCH, ppp);
        let active = hot == Some(scope.depth)
            && cursor0.is_some_and(|line| scope.open <= line && line <= scope.close);
        let color = if active { KW } else { RAIL };
        let stroke = Stroke::new(1.0, color);
        let tick = x + 5.0;
        if scope.open == scope.close {
            painter.hline(x..=tick, crisp(y_open, ppp), stroke);
        } else {
            painter.vline(x, crisp(y_open, ppp)..=crisp(y_close, ppp), stroke);
            painter.hline(x..=tick, crisp(y_open, ppp), stroke);
            painter.hline(x..=tick, crisp(y_close, ppp), stroke);
        }
    }

    let count = rows.len().max(1);
    for i in 0..count {
        let (y, h) = if let Some(row) = rows.get(i) {
            let y = galley_pos.y + row.pos.y - galley.rect.top();
            (y, row.rect().height().max(1.0))
        } else {
            (galley_pos.y, 16.0)
        };
        if y + h < rect.top() || y > rect.bottom() {
            continue;
        }
        let here = cursor0 == Some(i);
        painter.text(
            egui::pos2(rect.right() - 10.0, y + h * 0.5),
            egui::Align2::RIGHT_CENTER,
            format!("{}", i + 1),
            font.clone(),
            if here { INK } else { NUM_MUTED },
        );
    }

    painter.vline(
        crisp(rect.right() - 1.0, ppp),
        rect.top()..=rect.bottom(),
        Stroke::new(1.0, Color32::from_rgb(0xE0, 0xDC, 0xD4)),
    );
}

fn row_box(galley: &Galley, galley_pos: Pos2, index: usize) -> Option<(f32, f32)> {
    let row = galley.rows.get(index)?;
    let y = galley_pos.y + row.pos.y - galley.rect.top();
    Some((y, row.rect().height().max(1.0)))
}

fn crisp(v: f32, ppp: f32) -> f32 {
    (v * ppp).floor() / ppp + 0.5 / ppp
}

fn text_format(font: FontId, tone: Tone, line_h: f32, background: Color32) -> egui::TextFormat {
    egui::TextFormat {
        font_id: font,
        color: match tone {
            Tone::Text => INK,
            Tone::Kw => KW,
            Tone::Str => STR,
            Tone::Cmt => CMT,
            Tone::Num => NUM,
        },
        line_height: Some(line_h),
        background,
        ..Default::default()
    }
}

fn color_spans(text: &str, lang: Lang) -> Vec<(usize, usize, Tone)> {
    let mut spans = Vec::new();
    scan(text, lang, &mut spans, &mut Vec::new());
    spans
}

pub fn block_scopes(text: &str, lang: Lang) -> Vec<Scope> {
    let mut scopes = Vec::new();
    scan(text, lang, &mut Vec::new(), &mut scopes);
    scopes
}

fn scan(text: &str, lang: Lang, spans: &mut Vec<(usize, usize, Tone)>, scopes: &mut Vec<Scope>) {
    let bytes = text.as_bytes();
    let n = bytes.len();
    let mut i = 0usize;
    let mut line = 0usize;
    let mut bol = true;
    let mut stack: Vec<(usize, usize)> = Vec::new();
    let braces = matches!(lang, Lang::Native | Lang::C);

    while i < n {
        let c = bytes[i];
        if c == b'\n' {
            line += 1;
            i += 1;
            bol = true;
            continue;
        }
        if c == b' ' || c == b'\t' || c == b'\r' {
            i += 1;
            continue;
        }
        if lang == Lang::C && bol && c == b'#' {
            let start = i;
            i += 1;
            while i < n {
                if bytes[i] == b'\\' && i + 1 < n && bytes[i + 1] == b'\n' {
                    i += 2;
                    line += 1;
                    continue;
                }
                if bytes[i] == b'\n' {
                    break;
                }
                i += 1;
            }
            push_span(spans, start, i, Tone::Cmt);
            bol = false;
            continue;
        }
        bol = false;
        if c == b'/' && i + 1 < n && bytes[i + 1] == b'/' {
            let start = i;
            while i < n && bytes[i] != b'\n' {
                i += 1;
            }
            push_span(spans, start, i, Tone::Cmt);
            continue;
        }
        if braces && c == b'/' && i + 1 < n && bytes[i + 1] == b'*' {
            let start = i;
            i += 2;
            while i + 1 < n && !(bytes[i] == b'*' && bytes[i + 1] == b'/') {
                if bytes[i] == b'\n' {
                    line += 1;
                    bol = true;
                } else if bytes[i] != b' ' && bytes[i] != b'\t' && bytes[i] != b'\r' {
                    bol = false;
                }
                i += 1;
            }
            if i + 1 < n {
                i += 2;
                bol = false;
            } else {
                i = n;
            }
            push_span(spans, start, i, Tone::Cmt);
            continue;
        }
        if lang == Lang::Pascal && c == b'{' {
            let start = i;
            i += 1;
            while i < n && bytes[i] != b'}' {
                if bytes[i] == b'\n' {
                    line += 1;
                }
                i += 1;
            }
            if i < n {
                i += 1;
            }
            push_span(spans, start, i, Tone::Cmt);
            continue;
        }
        if lang == Lang::Pascal && c == b'(' && i + 1 < n && bytes[i + 1] == b'*' {
            let start = i;
            i += 2;
            while i + 1 < n && !(bytes[i] == b'*' && bytes[i + 1] == b')') {
                if bytes[i] == b'\n' {
                    line += 1;
                }
                i += 1;
            }
            if i + 1 < n {
                i += 2;
            } else {
                i = n;
            }
            push_span(spans, start, i, Tone::Cmt);
            continue;
        }
        if c == b'\'' || c == b'"' {
            let quote = c;
            let start = i;
            i += 1;
            while i < n && bytes[i] != b'\n' {
                if lang == Lang::C && bytes[i] == b'\\' && i + 1 < n && bytes[i + 1] != b'\n' {
                    i += 2;
                    continue;
                }
                if bytes[i] == quote {
                    if lang == Lang::Pascal && quote == b'\'' && i + 1 < n && bytes[i + 1] == b'\'' {
                        i += 2;
                        continue;
                    }
                    i += 1;
                    break;
                }
                i += 1;
            }
            push_span(spans, start, i, Tone::Str);
            continue;
        }
        if braces && (c == b'{' || c == b'}') {
            if c == b'{' {
                stack.push((line, stack.len()));
            } else if let Some((open, depth)) = stack.pop() {
                scopes.push(Scope { open, close: line, depth });
            }
            i += 1;
            continue;
        }
        if is_ident_start(c) {
            let start = i;
            i += 1;
            while i < n && is_ident(bytes[i]) {
                i += 1;
            }
            let word = &text[start..i];
            let keyword = match lang {
                Lang::Native => native_kw(&word.to_ascii_lowercase()),
                Lang::Pascal => pascal_kw(&word.to_ascii_lowercase()),
                Lang::C => c_kw(word),
            };
            if keyword {
                push_span(spans, start, i, Tone::Kw);
                if lang == Lang::Pascal && word.eq_ignore_ascii_case("begin") {
                    stack.push((line, stack.len()));
                } else if lang == Lang::Pascal && word.eq_ignore_ascii_case("end") {
                    if let Some((open, depth)) = stack.pop() {
                        scopes.push(Scope { open, close: line, depth });
                    }
                }
            }
            continue;
        }
        if c.is_ascii_digit() || (lang == Lang::Pascal && c == b'$') {
            let start = i;
            i += 1;
            while i < n && is_num(bytes[i]) {
                i += 1;
            }
            push_span(spans, start, i, Tone::Num);
            continue;
        }
        i += 1;
    }

    let last = line;
    while let Some((open, depth)) = stack.pop() {
        scopes.push(Scope {
            open,
            close: last,
            depth,
        });
    }
}

fn push_span(spans: &mut Vec<(usize, usize, Tone)>, start: usize, end: usize, tone: Tone) {
    if start >= end {
        return;
    }
    if let Some(last) = spans.last_mut() {
        if last.2 == tone && last.1 == start {
            last.1 = end;
            return;
        }
    }
    spans.push((start, end, tone));
}

fn is_ident_start(c: u8) -> bool {
    c.is_ascii_alphabetic() || c == b'_'
}

fn is_ident(c: u8) -> bool {
    c.is_ascii_alphanumeric() || c == b'_'
}

fn is_num(c: u8) -> bool {
    c.is_ascii_hexdigit() || c == b'x' || c == b'X'
}

fn pascal_kw(word: &str) -> bool {
    matches!(
        word,
        "unit"
            | "uses"
            | "procedure"
            | "function"
            | "var"
            | "begin"
            | "end"
            | "if"
            | "then"
            | "else"
            | "while"
            | "do"
            | "and"
            | "or"
            | "not"
            | "div"
            | "mod"
            | "exit"
            | "true"
            | "false"
            | "integer"
            | "string"
            | "boolean"
            | "view"
    )
}

fn native_kw(word: &str) -> bool {
    matches!(word, "fun" | "return" | "if" | "else" | "while" | "var" | "i32")
}

fn c_kw(word: &str) -> bool {
    matches!(
        word,
        "auto"
            | "break"
            | "case"
            | "char"
            | "const"
            | "continue"
            | "default"
            | "do"
            | "double"
            | "else"
            | "enum"
            | "extern"
            | "float"
            | "for"
            | "goto"
            | "if"
            | "inline"
            | "int"
            | "long"
            | "register"
            | "return"
            | "short"
            | "signed"
            | "sizeof"
            | "static"
            | "struct"
            | "switch"
            | "typedef"
            | "union"
            | "unsigned"
            | "void"
            | "volatile"
            | "while"
            | "bool"
            | "true"
            | "false"
            | "nullptr"
            | "wchar_t"
            | "class"
            | "namespace"
            | "template"
            | "typename"
            | "public"
            | "private"
            | "protected"
            | "virtual"
            | "override"
            | "final"
            | "new"
            | "delete"
            | "this"
            | "try"
            | "catch"
            | "throw"
            | "using"
            | "constexpr"
            | "explicit"
            | "friend"
            | "mutable"
            | "operator"
            | "int32_t"
            | "uint32_t"
            | "int64_t"
            | "uint64_t"
            | "size_t"
            | "DROIDE_EXPORT"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pairs(text: &str, lang: Lang) -> Vec<(usize, usize, usize)> {
        let mut rows: Vec<_> = block_scopes(text, lang)
            .into_iter()
            .map(|scope| (scope.open + 1, scope.close + 1, scope.depth))
            .collect();
        rows.sort();
        rows
    }

    fn tone_at(text: &str, lang: Lang, needle: &str) -> Tone {
        let at = text.find(needle).expect(needle);
        color_spans(text, lang)
            .into_iter()
            .find(|(start, end, _)| *start <= at && at < *end)
            .map(|(_, _, tone)| tone)
            .unwrap_or(Tone::Text)
    }

    #[test]
    fn pascal_scopes_skip_comments_and_strings() {
        let src = "\
begin
  Log('begin''end');
  // end
  { begin }
  (* begin *)
  else begin
    Log('hi')
  end
end
";
        assert_eq!(pairs(src, Lang::Pascal), vec![(1, 9, 0), (6, 8, 1)]);
    }

    #[test]
    fn pascal_brace_is_not_a_block() {
        assert_eq!(pairs("begin\n  { }\nend\n", Lang::Pascal), vec![(1, 3, 0)]);
    }

    #[test]
    fn begin_is_case_insensitive() {
        assert_eq!(pairs("Begin\nEnd;\n", Lang::Pascal), vec![(1, 2, 0)]);
    }

    #[test]
    fn unmatched_end_is_ignored_and_unclosed_runs_to_eof() {
        assert_eq!(pairs("end\nbegin\nend\n", Lang::Pascal), vec![(2, 3, 0)]);
        assert_eq!(pairs("begin\n  begin\n", Lang::Pascal), vec![(1, 3, 0), (2, 3, 1)]);
    }

    #[test]
    fn native_block_comment_is_not_a_brace() {
        let src = "fun f(): i32 {\n  /* { } */\n  return 1;\n}\n";
        assert_eq!(pairs(src, Lang::Native), vec![(1, 4, 0)]);
    }

    #[test]
    fn native_braces_skip_strings_and_comments() {
        let src = "fun fib(n: i32): i32 {\n  if n < 2 {\n    return n;\n  }\n  // {\n  return fib(n - 1);\n}\n";
        assert_eq!(pairs(src, Lang::Native), vec![(1, 7, 0), (2, 4, 1)]);
        assert_eq!(pairs("fun f(): i32 {\n  var s = \"{\";\n}\n", Lang::Native), vec![(1, 3, 0)]);
    }

    #[test]
    fn hello_and_fib_samples() {
        let main = include_str!("../../examples/Hello/forms/Main.droid");
        assert_eq!(
            pairs(main, Lang::Pascal),
            vec![
                (4, 7, 0),
                (12, 20, 0),
                (16, 19, 1),
                (23, 25, 0),
                (30, 40, 0),
                (33, 39, 1),
                (43, 44, 0),
                (47, 48, 0),
                (51, 52, 0),
            ]
        );
        let fib = include_str!("../../examples/Hello/native/fib.nc");
        assert_eq!(pairs(fib, Lang::Native), vec![(1, 6, 0), (2, 4, 1)]);
    }

    #[test]
    fn colors_match_the_web_editor() {
        assert_eq!(tone_at("begin\n", Lang::Pascal, "begin"), Tone::Kw);
        assert_eq!(tone_at("procedure FormCreate", Lang::Pascal, "procedure"), Tone::Kw);
        assert_eq!(tone_at("N := $2A;\n", Lang::Pascal, "$"), Tone::Num);
        assert_eq!(tone_at("  // hi\n", Lang::Pascal, "h"), Tone::Cmt);
        assert_eq!(tone_at("Log('Droide')\n", Lang::Pascal, "D"), Tone::Str);
        assert_eq!(tone_at("fun fib(n: i32)", Lang::Native, "i32"), Tone::Kw);
        assert_eq!(tone_at("fun fib(n: i32)", Lang::Native, "fib"), Tone::Text);
    }

    #[test]
    fn c_colors_exports_and_ignores_directives() {
        let src = include_str!("../../examples/Ndk/native/mix.c");
        assert_eq!(source_lang("native/mix.c"), Lang::C);
        assert_eq!(source_lang("native/span.cc"), Lang::C);
        assert_eq!(source_lang("native/echo.nc"), Lang::Native);
        assert_eq!(tone_at(src, Lang::C, "DROIDE_EXPORT"), Tone::Kw);
        assert_eq!(tone_at(src, Lang::C, "int32_t"), Tone::Kw);
        assert_eq!(tone_at(src, Lang::C, "strlen"), Tone::Text);
        assert_eq!(tone_at(src, Lang::C, "include"), Tone::Cmt);
        assert_eq!(tone_at("fun f() {\n}\n", Lang::C, "fun"), Tone::Text);
        assert_eq!(tone_at("Int x;\n", Lang::C, "Int"), Tone::Text);
        assert_eq!(pairs(src, Lang::C), vec![(5, 7, 0)]);
        assert_eq!(
            pairs("#define BODY { return 1; }\nint main() {\n  return 0;\n}\n", Lang::C),
            vec![(2, 4, 0)]
        );
        assert_eq!(pairs("void f() {\n  const char *s = \"{\";\n}\n", Lang::C), vec![(1, 3, 0)]);
        assert_eq!(char_index_of_line("a\nbc\n", 1), 0);
        assert_eq!(char_index_of_line("a\nbc\n", 2), 2);
        assert_eq!(char_index_of_line("a\nbc\n", 3), 5);
        assert_eq!(char_index_at("ab\ncd\n", 2, 1), 3);
        assert_eq!(char_index_at("ab\ncd\n", 2, 2), 4);
        assert_eq!(char_index_at("ab\ncd\n", 2, 40), 5);
    }

    #[test]
    fn layout_job_covers_every_byte() {
        let src = include_str!("../../examples/Hello/forms/Main.droid");
        let job = layout_job(src, Lang::Pascal, FontId::monospace(13.0), 16.0);
        assert_eq!(job.text, src);
        assert_eq!(job.sections.first().unwrap().byte_range.start.0, 0);
        assert_eq!(job.sections.last().unwrap().byte_range.end.0, src.len());
        let fib = include_str!("../../examples/Hello/native/fib.nc");
        let job = layout_job(fib, Lang::Native, FontId::monospace(13.0), 16.0);
        assert_eq!(job.text, fib);
        assert_eq!(job.wrap.max_width, f32::INFINITY);
        let mix = include_str!("../../examples/Ndk/native/mix.c");
        let job = layout_job(mix, Lang::C, FontId::monospace(13.0), 16.0);
        assert_eq!(job.text, mix);
        assert_eq!(job.sections.last().unwrap().byte_range.end.0, mix.len());
    }

    #[test]
    fn find_hit_paints_a_background() {
        let src = "begin\n";
        let job = layout_job_marked(src, Lang::Pascal, FontId::monospace(13.0), 16.0, &[(0, 5)], Some((0, 5)));
        assert_eq!(job.text, src);
        assert!(job.sections.iter().any(|section| section.format.background != Color32::TRANSPARENT));
    }
}
