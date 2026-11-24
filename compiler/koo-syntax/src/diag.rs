//! Source positions and diagnostics.

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

/// One error found while lexing or parsing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Diagnostic {
    pub span: Span,
    pub message: String,
    pub help: Option<String>,
}

impl Diagnostic {
    pub fn new(span: Span, message: impl Into<String>) -> Diagnostic {
        Diagnostic {
            span,
            message: message.into(),
            help: None,
        }
    }

    pub fn with_help(mut self, help: impl Into<String>) -> Diagnostic {
        self.help = Some(help.into());
        self
    }

    /// Renders the diagnostic in the format of Section 12.7 of the specification.
    pub fn render(&self, path: &str, src: &str, index: &LineIndex) -> String {
        // A span is always inside its source; one that is not must still not crash.
        let start = self.span.start.min(src.len() as u32);
        let (line, col) = index.line_col(src, start);
        let text = index.line_text(src, line);
        let gutter = line.to_string().len();
        let pad = " ".repeat(gutter);
        let before: String = text
            .chars()
            .take(col as usize - 1)
            .map(|c| if c == '\t' { '\t' } else { ' ' })
            .collect();
        let line_end = index.line_start(line) + text.len() as u32;
        let start = start.min(line_end);
        let end = self.span.end.clamp(start, line_end);
        let width = src
            .get(start as usize..end as usize)
            .map(|s| s.chars().count())
            .unwrap_or(0)
            .max(1);
        let mut out = format!("error: {}\n", self.message);
        out.push_str(&format!("{pad}--> {path}:{line}:{col}\n"));
        out.push_str(&format!("{pad} |\n"));
        out.push_str(&format!("{line} | {text}\n"));
        out.push_str(&format!("{pad} | {before}{}\n", "^".repeat(width)));
        if let Some(help) = &self.help {
            out.push_str(&format!("{pad} |\n{pad} = help: {help}\n"));
        }
        out
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
}

/// The source files of a program: one file, or every file of a package (Section 8.1).
#[derive(Debug, Clone)]
pub struct SourceMap {
    files: Vec<SourceFile>,
}

impl SourceMap {
    /// `files` are pairs of a path and the text of the file. There is at least one.
    pub fn new(files: Vec<(String, String)>) -> SourceMap {
        assert!(!files.is_empty(), "a program has a source file");
        let mut base = 0u32;
        let mut out = Vec::new();
        for (name, src) in files {
            let len = src.len() as u32;
            out.push(SourceFile {
                name,
                index: LineIndex::new(&src),
                src,
                base,
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
        let file = &self.files[self.file_index(diagnostic.span.start)];
        let mut local = diagnostic.clone();
        local.span = Span {
            start: diagnostic.span.start.saturating_sub(file.base),
            end: diagnostic.span.end.saturating_sub(file.base),
        };
        local.render(&file.name, &file.src, &file.index)
    }
}
