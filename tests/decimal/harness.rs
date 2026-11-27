//! The arithmetic of `Decimal` by itself, for `differential.py` to drive: one sum to
//! a line on the standard input, its answer on the standard output.
//!
//!     rustc --edition 2024 -O tests/decimal/harness.rs -o harness
//!
//! A line is an operation and its operands: `add a b`, `sub a b`, `mul a b`,
//! `cmp a b`, `div a b scale round`, `rescale a scale round`, `parse text`,
//! `reduce a`, `fixed a places`, `hash a b` (whether the two hash alike). An answer is the number as text and whether it had to be rounded, or
//! the name of what went wrong.
#[path = "../../compiler/koo-rt/src/decimal.rs"]
mod decimal;

use decimal::*;
use std::io::{BufRead, Write};

fn shown(value: &Dec) -> String {
    let mut out = [0u8; 48];
    let len = text(value, &mut out);
    String::from_utf8_lossy(&out[..len]).to_string()
}

fn main() {
    let stdin = std::io::stdin();
    let mut out = std::io::BufWriter::new(std::io::stdout());
    for line in stdin.lock().lines() {
        let line = line.unwrap();
        let words: Vec<&str> = line.split_whitespace().collect();
        let number = |at: usize| parse(words[at].as_bytes());
        let answer = (|| -> Result<String, Problem> {
            let said = |result: (Dec, bool)| format!("{} {}", shown(&result.0), if result.1 { "inexact" } else { "exact" });
            Ok(match words[0] {
                "add" => said(add(&number(1)?, &number(2)?)?),
                "sub" => said(sub(&number(1)?, &number(2)?)?),
                "mul" => said(mul(&number(1)?, &number(2)?)?),
                "cmp" => format!("{:?}", compare(&number(1)?, &number(2)?)),
                "div" => said(divide(
                    &number(1)?,
                    &number(2)?,
                    words[3].parse().unwrap(),
                    Round::of(words[4].parse().unwrap()),
                )?),
                "rescale" => said(rescaled(
                    &number(1)?,
                    words[2].parse().unwrap(),
                    Round::of(words[3].parse().unwrap()),
                )?),
                "parse" => shown(&number(1)?),
                "reduce" => shown(&number(1)?.reduced()),
                "fixed" => {
                    let mut out = [0u8; FIXED_LONGEST];
                    let len = fixed(&number(1)?, words[2].parse().unwrap(), &mut out);
                    String::from_utf8_lossy(&out[..len]).to_string()
                }
                "hash" => {
                    let (a, b) = (number(1)?.reduced(), number(2)?.reduced());
                    let alike = (a.is_zero() && b.is_zero()) || (a.hi, a.lo, a.exp, a.neg) == (b.hi, b.lo, b.exp, b.neg);
                    format!("{alike}")
                }
                other => panic!("no such operation: {other}"),
            })
        })();
        match answer {
            Ok(text) => writeln!(out, "{text}").unwrap(),
            Err(problem) => writeln!(out, "{problem:?}").unwrap(),
        }
    }
}
