//! Every `koo` code block in SPEC.md must parse.
//!
//! A block that begins with `import` is a complete file and is parsed strictly. Every other
//! block is a fragment: declarations and statements in any order, where a function may be a
//! signature without a body. The specification writes `{ ... }` for a body it leaves out;
//! that is read as an empty block.

use koo_syntax::{LineIndex, parse_file, parse_fragment};

struct Example {
    line: usize,
    source: String,
}

fn examples() -> Vec<Example> {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../SPEC.md");
    let spec = std::fs::read_to_string(path).expect("SPEC.md is at the repository root");
    let mut examples = Vec::new();
    let mut current: Option<Example> = None;
    for (i, line) in spec.lines().enumerate() {
        match &mut current {
            None if line.trim_end() == "```koo" => {
                current = Some(Example {
                    line: i + 2,
                    source: String::new(),
                })
            }
            None => {}
            Some(example) if line.trim_end() == "```" => {
                let _ = example;
                examples.push(current.take().unwrap());
            }
            Some(example) => {
                example.source.push_str(line);
                example.source.push('\n');
            }
        }
    }
    examples
}

#[test]
fn every_koo_block_in_the_spec_parses() {
    let examples = examples();
    assert!(
        examples.len() >= 30,
        "found only {} koo blocks in SPEC.md",
        examples.len()
    );
    let mut failures = String::new();
    let mut files = 0;
    for example in &examples {
        let source = example.source.replace("{ ... }", "{ }");
        let complete = source.starts_with("import ");
        files += complete as usize;
        let parsed = if complete {
            parse_file(&source)
        } else {
            parse_fragment(&source)
        };
        if parsed.diagnostics.is_empty() {
            continue;
        }
        let index = LineIndex::new(&source);
        failures.push_str(&format!("\nSPEC.md line {}:\n", example.line));
        for diagnostic in &parsed.diagnostics {
            failures.push_str(&diagnostic.render("SPEC.md", &source, &index));
        }
    }
    assert!(files >= 3, "expected the complete-file examples to be found");
    assert!(failures.is_empty(), "{failures}");
}
