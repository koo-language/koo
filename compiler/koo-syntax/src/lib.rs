//! The syntax of the Koo language: lexer, syntax tree and parser.
//!
//! The grammar is `spec/grammar.ebnf`; Section 3 of `SPEC.md` states the rules in prose.
//!
//! ```
//! let parsed = koo_syntax::parse_file("fn main() {\n    print(\"hello\")\n}\n");
//! assert!(parsed.diagnostics.is_empty());
//! assert_eq!(parsed.file.items.len(), 1);
//! ```

pub mod ast;
pub mod diag;
pub mod dump;
pub mod lexer;
mod parser;
pub mod token;

pub use diag::{Diagnostic, Fix, LineIndex, SourceFile, SourceMap, Span};
pub use parser::{Parsed, parse_file, parse_file_at, parse_fragment, parse_library};
