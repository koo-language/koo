//! Diagnostics as text, in the format of Section 12.7 of the specification:
//!
//! ```text
//! error: expected `int`, found `string`
//!  --> main.koo:6:10
//!   |
//! 1 | fn area(width: int, height: int) -> int {
//!   |         ---------- the parameter `width` is declared here
//! ...
//! 6 |     area("3", 4)
//!   |          ^^^ this is a `string`
//!   |
//!   = help: ...
//! help: ...
//!   |
//! 6 |     area(3, 4)
//!   |          ~
//! ```
//!
//! The marks under a line are `^` for the mistake and `-` for a place that explains it.
//! A correction is shown as the line would read with it: `+` under what is added, or the
//! old line and the new one when something is replaced or removed.

use std::collections::BTreeMap;

use super::{Diagnostic, Fix, LineIndex, Span};

/// One source file, as the renderer sees it.
pub(super) struct View<'a> {
    pub name: &'a str,
    pub src: &'a str,
    /// The index of `src` alone: its offsets count from the start of the file.
    pub index: &'a LineIndex,
    /// The offset of the file's first byte in the package.
    pub base: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Paint {
    Plain,
    /// The word `error`, and the marks and label of the mistake.
    Error,
    /// The line numbers and the rules beside them, and the marks of a related place.
    Frame,
    Bold,
    Help,
    Added,
    Removed,
}

impl Paint {
    fn code(self) -> &'static str {
        match self {
            Paint::Plain => "",
            Paint::Error => "\x1b[1;31m",
            Paint::Frame => "\x1b[1;34m",
            Paint::Bold => "\x1b[1m",
            Paint::Help => "\x1b[1;36m",
            Paint::Added => "\x1b[32m",
            Paint::Removed => "\x1b[31m",
        }
    }
}

struct Out {
    text: String,
    color: bool,
}

impl Out {
    fn put(&mut self, paint: Paint, text: &str) {
        if !self.color || paint == Paint::Plain || text.is_empty() {
            self.text.push_str(text);
        } else {
            self.text.push_str(paint.code());
            self.text.push_str(text);
            self.text.push_str("\x1b[0m");
        }
    }

    fn plain(&mut self, text: &str) {
        self.text.push_str(text);
    }

    /// A row of cells, without the spaces at its end.
    fn row(&mut self, row: &[(char, Paint)]) {
        let end = row
            .iter()
            .rposition(|(c, _)| *c != ' ' && *c != '\t')
            .map_or(0, |i| i + 1);
        let mut at = 0;
        while at < end {
            let paint = row[at].1;
            let run = row[at..end].iter().take_while(|(_, p)| *p == paint).count();
            let text: String = row[at..at + run].iter().map(|(c, _)| *c).collect();
            self.put(paint, &text);
            at += run;
        }
    }
}

/// A span as a place on one line: the file, the line, and the columns in characters.
#[derive(Debug, Clone, Copy)]
struct Place {
    file: usize,
    line: u32,
    /// The first column, counting from 0.
    col: usize,
    /// The number of columns, at least 1.
    width: usize,
    /// The byte offsets in the file of the part of the span that is on the line.
    start: usize,
    end: usize,
    /// Whether the span goes on past the end of the line.
    cut: bool,
}

struct Mark<'d> {
    place: Place,
    text: Option<&'d str>,
    primary: bool,
}

fn floor_boundary(src: &str, mut at: usize) -> usize {
    at = at.min(src.len());
    while !src.is_char_boundary(at) {
        at -= 1;
    }
    at
}

/// A span is always inside its source; one that is not must still not crash.
fn place(views: &[View], span: Span) -> Place {
    let file = views.iter().rposition(|view| view.base <= span.start).unwrap_or(0);
    let view = &views[file];
    let src = view.src;
    let start = floor_boundary(src, span.start.saturating_sub(view.base) as usize);
    let line = view.index.line(start as u32);
    let line_start = view.index.line_start(line) as usize;
    let line_end = line_start + view.index.line_text(src, line).len();
    let start = start.clamp(line_start, line_end);
    let wanted = span.end.saturating_sub(view.base) as usize;
    let end = floor_boundary(src, wanted).clamp(start, line_end);
    Place {
        file,
        line,
        col: src[line_start..start].chars().count(),
        width: src[start..end].chars().count().max(1),
        start,
        end,
        cut: wanted > line_end + 1,
    }
}

/// A row as wide as `width` that lines up under `text`: a tab under a tab, and spaces.
fn blank(text: &str, width: usize) -> Vec<(char, Paint)> {
    let mut row: Vec<(char, Paint)> = text
        .chars()
        .map(|c| (if c == '\t' { '\t' } else { ' ' }, Paint::Plain))
        .collect();
    row.resize(width.max(row.len()), (' ', Paint::Plain));
    row
}

fn write_at(row: &mut Vec<(char, Paint)>, col: usize, text: &str, paint: Paint) {
    let end = col + text.chars().count();
    if row.len() < end {
        row.resize(end, (' ', Paint::Plain));
    }
    for (i, c) in text.chars().enumerate() {
        row[col + i] = (c, paint);
    }
}

/// The rows under one line of source: the marks, and the label of each.
fn rows_under(text: &str, marks: &[&Mark]) -> Vec<Vec<(char, Paint)>> {
    // Marks that overlap cannot share a row. The mistake itself goes on the first.
    let mut ordered: Vec<&Mark> = marks.to_vec();
    ordered.sort_by_key(|mark| (!mark.primary, mark.place.col));
    let mut layers: Vec<Vec<&Mark>> = Vec::new();
    for mark in ordered {
        let (from, to) = (mark.place.col, mark.place.col + mark.place.width);
        let free = layers.iter_mut().find(|layer| {
            layer
                .iter()
                .all(|other| to <= other.place.col || other.place.col + other.place.width <= from)
        });
        match free {
            Some(layer) => layer.push(mark),
            None => layers.push(vec![mark]),
        }
    }
    let mut rows = Vec::new();
    for mut layer in layers {
        layer.sort_by_key(|mark| mark.place.col);
        let paint = |mark: &Mark| if mark.primary { Paint::Error } else { Paint::Frame };
        let mut row = blank(text, 0);
        for mark in &layer {
            let sign = if mark.primary { "^" } else { "-" };
            let mut line = sign.repeat(mark.place.width);
            if mark.place.cut {
                line.push_str("...");
            }
            write_at(&mut row, mark.place.col, &line, paint(mark));
        }
        // The label of the last mark goes beside it; the others hang below theirs.
        let last = layer.pop().expect("a layer has a mark");
        if let Some(label) = last.text {
            let at = last.place.col + last.place.width + if last.place.cut { 3 } else { 0 } + 1;
            write_at(&mut row, at, label, paint(last));
        }
        rows.push(row);
        let hanging: Vec<&Mark> = layer.into_iter().filter(|mark| mark.text.is_some()).collect();
        if hanging.is_empty() {
            continue;
        }
        let stems = |upto: usize| {
            let mut row = blank(text, 0);
            for mark in &hanging[..upto] {
                write_at(&mut row, mark.place.col, "|", paint(mark));
            }
            row
        };
        rows.push(stems(hanging.len()));
        for (i, mark) in hanging.iter().enumerate().rev() {
            let mut row = stems(i);
            write_at(&mut row, mark.place.col, mark.text.unwrap_or(""), paint(mark));
            rows.push(row);
        }
    }
    rows
}

/// One edit of a correction, on one line of one file.
struct Edit<'d> {
    /// Byte offsets in the line.
    start: usize,
    end: usize,
    text: &'d str,
}

/// The edits of a correction by line, or nothing if it cannot be shown as lines: an
/// edit that spans lines, edits in several files, or edits that overlap.
fn edits_by_line<'d>(views: &[View], fix: &'d Fix) -> Option<(usize, BTreeMap<u32, Vec<Edit<'d>>>)> {
    let mut file = None;
    let mut lines: BTreeMap<u32, Vec<Edit>> = BTreeMap::new();
    for (span, text) in &fix.edits {
        let at = place(views, *span);
        if at.cut || text.contains('\n') || *file.get_or_insert(at.file) != at.file {
            return None;
        }
        let line_start = views[at.file].index.line_start(at.line) as usize;
        lines.entry(at.line).or_default().push(Edit {
            start: at.start - line_start,
            end: at.end - line_start,
            text,
        });
    }
    for edits in lines.values_mut() {
        edits.sort_by_key(|edit| (edit.start, edit.end));
        if edits.windows(2).any(|pair| pair[1].start < pair[0].end) {
            return None;
        }
    }
    Some((file?, lines))
}

pub(super) fn render(views: &[View], diagnostic: &Diagnostic, color: bool) -> String {
    let mut marks = vec![Mark {
        place: place(views, diagnostic.span),
        text: diagnostic.label.as_deref(),
        primary: true,
    }];
    for (span, text) in &diagnostic.related {
        marks.push(Mark {
            place: place(views, *span),
            text: Some(text),
            primary: false,
        });
    }
    let fix = diagnostic.fix.as_ref().map(|fix| (fix, edits_by_line(views, fix)));

    let mut last_line = marks.iter().map(|mark| mark.place.line).max().unwrap_or(1);
    if let Some((_, Some((_, lines)))) = &fix {
        last_line = last_line.max(lines.keys().copied().max().unwrap_or(1));
    }
    let gutter = last_line.to_string().len();
    let pad = " ".repeat(gutter);

    let mut out = Out {
        text: String::new(),
        color,
    };
    out.put(Paint::Error, "error");
    out.put(Paint::Bold, &format!(": {}", diagnostic.message));
    out.plain("\n");

    // The file of the mistake first, then the others in the order they are named.
    let primary = marks[0].place;
    let mut files = vec![primary.file];
    for mark in &marks {
        if !files.contains(&mark.place.file) {
            files.push(mark.place.file);
        }
    }
    for file in files {
        let view = &views[file];
        let mut in_file: Vec<&Mark> = marks.iter().filter(|mark| mark.place.file == file).collect();
        in_file.sort_by_key(|mark| mark.place.line);
        let (arrow, at) = if file == primary.file {
            ("-->", primary)
        } else {
            (":::", in_file[0].place)
        };
        out.plain(&pad);
        out.put(Paint::Frame, arrow);
        out.plain(&format!(" {}:{}:{}\n", view.name, at.line, at.col + 1));
        out.put(Paint::Frame, &format!("{pad} |"));
        out.plain("\n");
        let mut shown: Option<u32> = None;
        let mut lines: Vec<u32> = in_file.iter().map(|mark| mark.place.line).collect();
        lines.dedup();
        for line in lines {
            match shown {
                // One line between two that are shown is shown too.
                Some(before) if before + 2 == line => source_line(&mut out, view, before + 1, gutter),
                Some(before) if before + 2 < line => {
                    out.put(Paint::Frame, "...");
                    out.plain("\n");
                }
                _ => {}
            }
            source_line(&mut out, view, line, gutter);
            let text = view.index.line_text(view.src, line);
            let on_line: Vec<&Mark> = in_file.iter().copied().filter(|mark| mark.place.line == line).collect();
            for row in rows_under(text, &on_line) {
                out.put(Paint::Frame, &format!("{pad} |"));
                out.plain(" ");
                out.row(&row);
                out.plain("\n");
            }
            shown = Some(line);
        }
    }

    if !diagnostic.notes.is_empty() || !diagnostic.help.is_empty() {
        out.put(Paint::Frame, &format!("{pad} |"));
        out.plain("\n");
    }
    for (word, texts) in [("note", &diagnostic.notes), ("help", &diagnostic.help)] {
        for text in texts {
            out.put(Paint::Frame, &format!("{pad} ="));
            out.put(Paint::Bold, &format!(" {word}:"));
            out.plain(&format!(" {text}\n"));
        }
    }

    if let Some((fix, lines)) = fix {
        if diagnostic.notes.is_empty() && diagnostic.help.is_empty() {
            out.put(Paint::Frame, &format!("{pad} |"));
            out.plain("\n");
        }
        out.put(Paint::Help, "help");
        out.plain(&format!(": {}\n", fix.message));
        if let Some((file, lines)) = lines {
            out.put(Paint::Frame, &format!("{pad} |"));
            out.plain("\n");
            for (line, edits) in lines {
                fixed_line(&mut out, &views[file], line, &edits, gutter);
            }
        }
    }
    out.text
}

fn source_line(out: &mut Out, view: &View, line: u32, gutter: usize) {
    out.put(Paint::Frame, &format!("{line:>gutter$} |"));
    let text = view.index.line_text(view.src, line);
    if !text.is_empty() {
        out.plain(" ");
        out.plain(text);
    }
    out.plain("\n");
}

/// A line as a correction leaves it. Text that is only added is marked with `+` under
/// it; a line that loses text is shown as it is and as it would be.
fn fixed_line(out: &mut Out, view: &View, line: u32, edits: &[Edit], gutter: usize) {
    let old = view.index.line_text(view.src, line);
    let pad = " ".repeat(gutter);
    let mut new = String::new();
    // The columns of the new line that hold added text.
    let mut added: Vec<(usize, usize)> = Vec::new();
    let mut at = 0;
    for edit in edits {
        let (start, end) = (floor_boundary(old, edit.start), floor_boundary(old, edit.end));
        new.push_str(&old[at..start]);
        let col = new.chars().count();
        new.push_str(edit.text);
        added.push((col, edit.text.chars().count()));
        at = end;
    }
    new.push_str(&old[at..]);
    if edits.iter().all(|edit| edit.start == edit.end) {
        out.put(Paint::Frame, &format!("{line:>gutter$} |"));
        out.plain(&format!(" {new}\n"));
        let mut row = blank(&new, 0);
        for (col, width) in added {
            write_at(&mut row, col, &"+".repeat(width), Paint::Added);
        }
        out.put(Paint::Frame, &format!("{pad} |"));
        out.plain(" ");
        out.row(&row);
        out.plain("\n");
    } else {
        out.put(Paint::Frame, &format!("{line:>gutter$} "));
        out.put(Paint::Removed, &format!("- {old}"));
        out.plain("\n");
        // A line that is removed whole has nothing to show in its place.
        if !new.trim().is_empty() {
            out.put(Paint::Frame, &format!("{line:>gutter$} "));
            out.put(Paint::Added, &format!("+ {new}"));
            out.plain("\n");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::{Diagnostic, SourceMap, Span};

    fn span(src: &str, text: &str) -> Span {
        let start = src.find(text).unwrap_or_else(|| panic!("no {text:?} in the source"));
        Span::new(start, start + text.len())
    }

    #[test]
    fn a_mistake_alone() {
        let src = "fn main() {\n    var a = 0\n}\n";
        let diagnostic = Diagnostic::new(span(src, "="), "expected `:=`, found `=`").with_help("a help");
        assert_eq!(
            SourceMap::single("main.koo", src).render(&diagnostic),
            "error: expected `:=`, found `=`\n \
             --> main.koo:2:11\n  \
             |\n\
             2 |     var a = 0\n  \
             |           ^\n  \
             |\n  \
             = help: a help\n"
        );
    }

    #[test]
    fn a_label_and_a_related_place_far_above() {
        let src = "fn area(width: int) -> int {\n    return width\n}\n\n\n\n\n\n\nfn main() {\n    area(\"3\")\n}\n";
        let diagnostic = Diagnostic::new(span(src, "\"3\""), "expected `int`, found `string`")
            .with_label("this is a `string`")
            .with_related(span(src, "width: int"), "the parameter is declared here");
        assert_eq!(
            SourceMap::single("main.koo", src).render(&diagnostic),
            "error: expected `int`, found `string`\n  \
             --> main.koo:11:10\n   \
             |\n \
             1 | fn area(width: int) -> int {\n   \
             |         ---------- the parameter is declared here\n\
             ...\n\
             11 |     area(\"3\")\n   \
             |          ^^^ this is a `string`\n"
        );
    }

    #[test]
    fn two_marks_on_a_line_and_a_line_between() {
        let src = "a := 1\nb := 2\nc := a * 0.5\n";
        let diagnostic = Diagnostic::new(span(src, "0.5"), "no")
            .with_label("a `float`")
            .with_related(span(src, "a *"), "an `int`")
            .with_related(span(src, "a := 1"), "bound here");
        let mut at = diagnostic.clone();
        at.related[0].0.end -= 2;
        assert_eq!(
            SourceMap::single("m.koo", src).render(&at),
            "error: no\n \
             --> m.koo:3:10\n  \
             |\n\
             1 | a := 1\n  \
             | ------ bound here\n\
             2 | b := 2\n\
             3 | c := a * 0.5\n  \
             |      -   ^^^ a `float`\n  \
             |      |\n  \
             |      an `int`\n"
        );
    }

    #[test]
    fn marks_that_overlap_take_a_row_each() {
        let src = "x := if n { 1 } else { \"a\" }\n";
        let diagnostic = Diagnostic::new(span(src, "if n { 1 } else { \"a\" }"), "no")
            .with_related(span(src, "1"), "an `int`")
            .with_related(span(src, "\"a\""), "a `string`");
        assert_eq!(
            SourceMap::single("m.koo", src).render(&diagnostic),
            "error: no\n \
             --> m.koo:1:6\n  \
             |\n\
             1 | x := if n { 1 } else { \"a\" }\n  \
             |      ^^^^^^^^^^^^^^^^^^^^^^^\n  \
             |             -          --- a `string`\n  \
             |             |\n  \
             |             an `int`\n"
        );
    }

    #[test]
    fn corrections() {
        let src = "fn main() {\n    fill(names, 3)\n    show(var xs)\n}\n";
        let names = span(src, "names");
        let insert = Diagnostic::new(names, "no").with_fix(
            "write `var` before the argument",
            Span::new(names.start as usize, names.start as usize),
            "var ",
        );
        assert_eq!(
            SourceMap::single("m.koo", src).render(&insert),
            "error: no\n \
             --> m.koo:2:10\n  \
             |\n\
             2 |     fill(names, 3)\n  \
             |          ^^^^^\n  \
             |\n\
             help: write `var` before the argument\n  \
             |\n\
             2 |     fill(var names, 3)\n  \
             |          ++++\n"
        );
        let remove = Diagnostic::new(span(src, "xs"), "no").with_fix("remove `var`", span(src, "var "), "");
        assert_eq!(
            SourceMap::single("m.koo", src).render(&remove),
            "error: no\n \
             --> m.koo:3:14\n  \
             |\n\
             3 |     show(var xs)\n  \
             |              ^^\n  \
             |\n\
             help: remove `var`\n  \
             |\n\
             3 -     show(var xs)\n\
             3 +     show(xs)\n"
        );
    }

    #[test]
    fn a_related_place_in_another_file() {
        let sources = SourceMap::new(vec![
            ("a.koo".to_string(), "fn area() {\n}\n".to_string()),
            ("b.koo".to_string(), "fn area() {\n}\n".to_string()),
        ]);
        let second = sources.files()[1].base;
        let diagnostic = Diagnostic::new(Span::new(3, 7).shifted(second), "`area` is already declared")
            .with_related(Span::new(3, 7), "first declared here");
        assert_eq!(
            sources.render(&diagnostic),
            "error: `area` is already declared\n \
             --> b.koo:1:4\n  \
             |\n\
             1 | fn area() {\n  \
             |    ^^^^\n \
             ::: a.koo:1:4\n  \
             |\n\
             1 | fn area() {\n  \
             |    ---- first declared here\n"
        );
    }

    #[test]
    fn color_and_spans_outside_the_source_do_not_crash() {
        let src = "x := 1\n\ty := \u{e9}\n";
        let sources = SourceMap::single("m.koo", src);
        for start in 0..40u32 {
            for len in [0, 1, 5, 90] {
                let diagnostic = Diagnostic::new(
                    Span {
                        start,
                        end: start + len,
                    },
                    "m",
                )
                .with_label("l")
                .with_related(
                    Span {
                        start: start / 2,
                        end: start,
                    },
                    "r",
                )
                .with_fix(
                    "f",
                    Span {
                        start,
                        end: start + len / 2,
                    },
                    "t",
                );
                let plain = sources.render(&diagnostic);
                let colored = sources.render_with(&diagnostic, true);
                assert!(colored.contains("\x1b[1;31merror\x1b[0m"));
                assert!(!plain.contains('\x1b'));
            }
        }
    }
}
