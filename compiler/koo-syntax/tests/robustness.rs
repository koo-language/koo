//! The parser must return on any input: no panic, no hang (Section 13.1: the parser is
//! error-tolerant). This test damages the code blocks of SPEC.md in random ways, with a
//! fixed seed, and parses the results.

use koo_syntax::{parse_file, parse_fragment};

struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        // xorshift64
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }

    fn below(&mut self, n: usize) -> usize {
        (self.next() % n.max(1) as u64) as usize
    }
}

const FRAGMENTS: [&str; 40] = [
    "{", "}", "(", ")", "[", "]", "\"", "\"\"\"", "`", "'", "\n", " ", ",", ":", ":=", "=", "->", "=>", "!", "|", "?",
    ".", "..", "...", "<", ">", "@", ";", "-", "else", "if", "match", "fn", "import", "x", "T", "1", "1.5d", "\\",
    "{x:",
];

fn sources() -> Vec<String> {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../SPEC.md");
    let spec = std::fs::read_to_string(path).expect("SPEC.md is at the repository root");
    let mut sources = Vec::new();
    let mut current: Option<String> = None;
    for line in spec.lines() {
        match &mut current {
            None if line.trim_end() == "```koo" => current = Some(String::new()),
            None => {}
            Some(_) if line.trim_end() == "```" => sources.push(current.take().unwrap()),
            Some(source) => {
                source.push_str(line);
                source.push('\n');
            }
        }
    }
    sources
}

fn damage(source: &str, rng: &mut Rng) -> String {
    let mut chars: Vec<char> = source.chars().collect();
    for _ in 0..1 + rng.below(4) {
        let at = rng.below(chars.len() + 1);
        match rng.below(4) {
            0 if at < chars.len() => {
                let len = 1 + rng.below(12);
                chars.drain(at..(at + len).min(chars.len()));
            }
            1 => {
                let piece: Vec<char> = FRAGMENTS[rng.below(FRAGMENTS.len())].chars().collect();
                chars.splice(at..at, piece);
            }
            2 if at < chars.len() => {
                let other = rng.below(chars.len());
                chars.swap(at, other);
            }
            _ => chars.truncate(at),
        }
    }
    chars.into_iter().collect()
}

#[test]
fn damaged_sources_never_panic_or_hang() {
    let sources = sources();
    let mut rng = Rng(0x9E37_79B9_7F4A_7C15);
    for round in 0..300 {
        for source in &sources {
            let damaged = damage(source, &mut rng);
            let parsed = if round % 2 == 0 {
                parse_file(&damaged)
            } else {
                parse_fragment(&damaged)
            };
            // Every diagnostic must point inside the source.
            for diagnostic in &parsed.diagnostics {
                assert!(
                    diagnostic.span.start <= diagnostic.span.end && diagnostic.span.end as usize <= damaged.len(),
                    "bad span {:?} for {:?} in:\n{damaged}",
                    diagnostic.span,
                    diagnostic.message
                );
                let index = koo_syntax::LineIndex::new(&damaged);
                let _ = diagnostic.render("damaged.koo", &damaged, &index);
            }
        }
    }
}

#[test]
fn deep_nesting_is_an_error_not_a_crash() {
    let n = 100_000;
    let inputs = [
        format!("x := {}1{}", "(".repeat(n), ")".repeat(n)),
        format!("x := {}1{}", "[".repeat(n), "]".repeat(n)),
        format!("x := {}1", "-".repeat(n).replace("-", "- ")),
        format!("x := {}true", "not ".repeat(n)),
        format!("x := a{}", " else a".repeat(n)),
        format!("x := {}1", "y -> ".repeat(n)),
        format!("x: {}int{} = y", "[".repeat(n), "]".repeat(n)),
        format!("x: {}int{} = y", "(".repeat(n), ")".repeat(n)),
        format!("x: int{} = y", "?".repeat(n)),
        format!("fn f() {}{{ }}{}", "{ if a ".repeat(n), "}".repeat(n)),
        format!("fn f() {{ {} }}", "if a { } else ".repeat(n)),
        format!("match v {{\n    {}x{} => f()\n}}", "(".repeat(n), ")".repeat(n)),
        format!("x := {}", "\"{".repeat(n)),
        format!("x := f{}", "(g".repeat(n)),
        format!("x := a{}", ".b".repeat(n)),
        format!("x := a{}", " + a".repeat(n)),
    ];
    for input in &inputs {
        let parsed = parse_fragment(input);
        let _ = parse_file(input);
        assert!(!parsed.diagnostics.is_empty(), "accepted: {}", &input[..40]);
    }
}

#[test]
fn ordinary_nesting_is_accepted() {
    let n = 200;
    let inputs = [
        format!("x := {}1{}", "(".repeat(n), ")".repeat(n)),
        format!("x := a{}", ".b()".repeat(n)),
        format!("x := a{}", " + a * b".repeat(n)),
        format!("x: {}int{} = y", "[".repeat(n), "]".repeat(n)),
        format!("fn f() {}{{ }}{}", "{ if a ".repeat(n), "}".repeat(n)),
        r#"x := "{"{"{a}"}"}""#.to_string(),
    ];
    for input in &inputs {
        let parsed = parse_fragment(input);
        assert!(
            parsed.diagnostics.is_empty(),
            "{:?} for {}",
            parsed.diagnostics[0],
            &input[..18]
        );
    }
}
