//! Source positions and diagnostics.

#[path = "render.rs"]
mod render;

/// A half-open range of byte offsets into one source file.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Span {
    pub start: u32,
    pub end: u32,
}

impl Span {
    pub fn new(start: usize, end: usize) -> Span {
        Span {
            start: start as u32,
            end: end as u32,
        }
    }

    /// The same range `base` bytes further on: a place in a file as a place in the
    /// package the file belongs to.
    pub fn shifted(self, base: u32) -> Span {
        Span {
            start: self.start + base,
            end: self.end + base,
        }
    }

    /// The span from the start of `self` to the end of `other`.
    pub fn to(self, other: Span) -> Span {
        Span {
            start: self.start,
            end: other.end.max(self.end),
        }
    }
}

/// A change to the source that corrects a mistake, shown with the line as it would
/// read afterwards.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Fix {
    pub message: String,
    /// Each edit replaces the text of a span with new text. An empty span inserts and
    /// an empty text removes.
    pub edits: Vec<(Span, String)>,
}

/// One error found while compiling.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Diagnostic {
    /// Where the mistake is.
    pub span: Span,
    /// What is wrong, in one line.
    pub message: String,
    /// What the marks under `span` say.
    pub label: Option<String>,
    /// Other places that explain the mistake, and what each one has to do with it.
    pub related: Vec<(Span, String)>,
    /// Facts that explain the mistake.
    pub notes: Vec<String>,
    /// Advice on correcting it.
    pub help: Vec<String>,
    /// The correction itself, when the compiler can write it.
    pub fix: Option<Fix>,
}

impl Diagnostic {
    pub fn new(span: Span, message: impl Into<String>) -> Diagnostic {
        Diagnostic {
            span,
            message: message.into(),
            label: None,
            related: Vec::new(),
            notes: Vec::new(),
            help: Vec::new(),
            fix: None,
        }
    }

    pub fn with_label(mut self, label: impl Into<String>) -> Diagnostic {
        self.label = Some(label.into());
        self
    }

    pub fn with_related(mut self, span: Span, text: impl Into<String>) -> Diagnostic {
        self.related.push((span, text.into()));
        self
    }

    pub fn with_note(mut self, note: impl Into<String>) -> Diagnostic {
        self.notes.push(note.into());
        self
    }

    pub fn with_help(mut self, help: impl Into<String>) -> Diagnostic {
        self.help.push(help.into());
        self
    }

    /// A correction that replaces the text of `span` with `text`.
    pub fn with_fix(mut self, message: impl Into<String>, span: Span, text: impl Into<String>) -> Diagnostic {
        self.fix = Some(Fix {
            message: message.into(),
            edits: vec![(span, text.into())],
        });
        self
    }

    /// A correction made of several edits.
    pub fn with_edits(mut self, message: impl Into<String>, edits: Vec<(Span, String)>) -> Diagnostic {
        self.fix = Some(Fix {
            message: message.into(),
            edits,
        });
        self
    }

    /// The same diagnostic `base` bytes further on: every place it names moves.
    pub fn shifted(mut self, base: u32) -> Diagnostic {
        self.span = self.span.shifted(base);
        for (span, _) in &mut self.related {
            *span = span.shifted(base);
        }
        if let Some(fix) = &mut self.fix {
            for (span, _) in &mut fix.edits {
                *span = span.shifted(base);
            }
        }
        self
    }

    /// Renders the diagnostic in the format of Section 12.7 of the specification, for a
    /// program of one file.
    pub fn render(&self, path: &str, src: &str, index: &LineIndex) -> String {
        let view = render::View {
            name: path,
            src,
            index,
            base: 0,
        };
        render::render(&[view], self, false)
    }
}

/// Maps byte offsets to lines and columns.
#[derive(Debug, Clone)]
pub struct LineIndex {
    starts: Vec<u32>,
}

impl LineIndex {
    pub fn new(src: &str) -> LineIndex {
        LineIndex::at(src, 0)
    }

    /// The index of a file whose first byte is at offset `base` of its package. Only
    /// `line` is meaningful for it.
    pub fn at(src: &str, base: u32) -> LineIndex {
        let mut starts = vec![base];
        for (i, b) in src.bytes().enumerate() {
            if b == b'\n' {
                starts.push(base + i as u32 + 1);
            }
        }
        LineIndex { starts }
    }

    /// The 1-based line that contains `offset`.
    pub fn line(&self, offset: u32) -> u32 {
        match self.starts.binary_search(&offset) {
            Ok(i) => i as u32 + 1,
            Err(i) => i as u32,
        }
    }

    pub fn line_start(&self, line: u32) -> u32 {
        self.starts[line as usize - 1]
    }

    /// The 1-based line and column (in characters) of `offset`.
    pub fn line_col(&self, src: &str, offset: u32) -> (u32, u32) {
        let line = self.line(offset);
        let start = self.line_start(line) as usize;
        let offset = (offset as usize).min(src.len());
        let col = src[start..offset].chars().count() as u32 + 1;
        (line, col)
    }

    /// The text of a line, without its line terminator.
    pub fn line_text<'a>(&self, src: &'a str, line: u32) -> &'a str {
        let start = self.line_start(line) as usize;
        let end = self
            .starts
            .get(line as usize)
            .map(|&s| s as usize - 1)
            .unwrap_or(src.len());
        src[start..end].trim_end_matches('\r')
    }
}

/// One source file of a package.
#[derive(Debug, Clone)]
pub struct SourceFile {
    /// The path as it is shown in messages.
    pub name: String,
    pub src: String,
    /// The offset of the file's first byte in the package: spans count from the start of
    /// the package, so that a span says which file it is in.
    pub base: u32,
    index: LineIndex,
    /// Which package of the program the file is of: empty for the program's own, and
    /// for a package it imports from a directory (Section 8.2) the name that sets the
    /// package's names apart from every other's.
    pub package: String,
    /// The packages the file imports from directories: each path as the file writes
    /// it, and the name of the package found there.
    pub imports: Vec<(String, String)>,
}

/// A source file as it is given to a `SourceMap`: its path, its text, the package it is
/// of, and the packages it imports from directories.
pub struct Sourced {
    pub name: String,
    pub src: String,
    pub package: String,
    pub imports: Vec<(String, String)>,
}

/// The source files of a program: one file, or every file of a package (Section 8.1).
#[derive(Debug, Clone)]
pub struct SourceMap {
    files: Vec<SourceFile>,
}

impl SourceMap {
    /// `files` are pairs of a path and the text of the file. There is at least one.
    pub fn new(files: Vec<(String, String)>) -> SourceMap {
        SourceMap::of(
            files
                .into_iter()
                .map(|(name, src)| Sourced {
                    name,
                    src,
                    package: String::new(),
                    imports: Vec::new(),
                })
                .collect(),
        )
    }

    /// The files of a program that may be of several packages.
    pub fn of(files: Vec<Sourced>) -> SourceMap {
        assert!(!files.is_empty(), "a program has a source file");
        let mut base = 0u32;
        let mut out = Vec::new();
        for Sourced {
            name,
            src,
            package,
            imports,
        } in files
        {
            let len = src.len() as u32;
            out.push(SourceFile {
                name,
                index: LineIndex::new(&src),
                src,
                base,
                package,
                imports,
            });
            // One byte between files, so that the end of one is not the start of the next.
            base += len + 1;
        }
        SourceMap { files: out }
    }

    pub fn single(name: &str, src: &str) -> SourceMap {
        SourceMap::new(vec![(name.to_string(), src.to_string())])
    }

    pub fn files(&self) -> &[SourceFile] {
        &self.files
    }

    /// The number of the file that contains `offset`.
    pub fn file_index(&self, offset: u32) -> usize {
        self.files.iter().rposition(|file| file.base <= offset).unwrap_or(0)
    }

    /// The name of the file that contains `offset`, and the 1-based line in it.
    pub fn location(&self, offset: u32) -> (&str, u32) {
        let file = &self.files[self.file_index(offset)];
        let local = offset.saturating_sub(file.base).min(file.src.len() as u32);
        (&file.name, file.index.line(local))
    }

    pub fn render(&self, diagnostic: &Diagnostic) -> String {
        self.render_with(diagnostic, false)
    }

    /// Renders a diagnostic, with the colors of a terminal if `color` is set.
    pub fn render_with(&self, diagnostic: &Diagnostic, color: bool) -> String {
        let views: Vec<render::View> = self
            .files
            .iter()
            .map(|file| render::View {
                name: &file.name,
                src: &file.src,
                index: &file.index,
                base: file.base,
            })
            .collect();
        render::render(&views, diagnostic, color)
    }
}
