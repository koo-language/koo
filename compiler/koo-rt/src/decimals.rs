//! What a program asks of `Decimal` (Section 4.3). The arithmetic is `decimal.rs`; this
//! is how the prelude reaches it.
//!
//! A `Decimal` of a program is a struct of four fields, the two halves of its
//! coefficient, its exponent and its sign, and each function here takes those of its
//! operands. What it makes is left with the thread, for the prelude to take a field at a
//! time (`koo_dec_hi` and the others), and what it returns says how it went: 0 for a
//! result that is exact, 1 for one that had to be rounded, and from 2 the ways there is
//! none, which the prelude makes a panic or an error of as the operation asks.

use crate::decimal::{self, Dec, Problem, Round};
use crate::{Header, bytes, new_str, thread};

fn of(hi: i64, lo: i64, exp: i64, neg: bool) -> Dec {
    Dec {
        neg,
        hi: hi as u64,
        lo: lo as u64,
        exp: exp as i32,
    }
}

/// Keeps a result for the calling thread and says how it went.
unsafe fn kept(result: Result<(Dec, bool), Problem>) -> i64 {
    match result {
        Ok((value, inexact)) => {
            unsafe { thread::me().decimal = value };
            inexact as i64
        }
        Err(Problem::TooGreat) => 2,
        Err(Problem::ByZero) => 3,
        Err(Problem::TooLong) => 4,
        Err(Problem::NotANumber) => 5,
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn koo_dec_add(
    ahi: i64,
    alo: i64,
    aexp: i64,
    aneg: bool,
    bhi: i64,
    blo: i64,
    bexp: i64,
    bneg: bool,
) -> i64 {
    unsafe { kept(decimal::add(&of(ahi, alo, aexp, aneg), &of(bhi, blo, bexp, bneg))) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn koo_dec_sub(
    ahi: i64,
    alo: i64,
    aexp: i64,
    aneg: bool,
    bhi: i64,
    blo: i64,
    bexp: i64,
    bneg: bool,
) -> i64 {
    unsafe { kept(decimal::sub(&of(ahi, alo, aexp, aneg), &of(bhi, blo, bexp, bneg))) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn koo_dec_mul(
    ahi: i64,
    alo: i64,
    aexp: i64,
    aneg: bool,
    bhi: i64,
    blo: i64,
    bexp: i64,
    bneg: bool,
) -> i64 {
    unsafe { kept(decimal::mul(&of(ahi, alo, aexp, aneg), &of(bhi, blo, bexp, bneg))) }
}

/// -1, 0 or 1 as the first is less than, equal to or greater than the second, by
/// what they are worth.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn koo_dec_cmp(
    ahi: i64,
    alo: i64,
    aexp: i64,
    aneg: bool,
    bhi: i64,
    blo: i64,
    bexp: i64,
    bneg: bool,
) -> i64 {
    match decimal::compare(&of(ahi, alo, aexp, aneg), &of(bhi, blo, bexp, bneg)) {
        core::cmp::Ordering::Less => -1,
        core::cmp::Ordering::Equal => 0,
        core::cmp::Ordering::Greater => 1,
    }
}

/// The quotient with `scale` digits after the point, rounded in the way of `round`'s
/// number (`decimal::Round::of`).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn koo_dec_div(
    ahi: i64,
    alo: i64,
    aexp: i64,
    aneg: bool,
    bhi: i64,
    blo: i64,
    bexp: i64,
    bneg: bool,
    scale: i64,
    round: i64,
) -> i64 {
    let (a, b) = (of(ahi, alo, aexp, aneg), of(bhi, blo, bexp, bneg));
    unsafe { kept(decimal::divide(&a, &b, scale, Round::of(round))) }
}

/// The same number with `scale` digits after the point.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn koo_dec_rescale(hi: i64, lo: i64, exp: i64, neg: bool, scale: i64, round: i64) -> i64 {
    unsafe { kept(decimal::rescaled(&of(hi, lo, exp, neg), scale, Round::of(round))) }
}

/// The same number without the zeros at the end of its digits, which is one form for
/// all that are equal: what a hash is made of.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn koo_dec_reduce(hi: i64, lo: i64, exp: i64, neg: bool) -> i64 {
    unsafe { thread::me().decimal = of(hi, lo, exp, neg).reduced() };
    0
}

/// The number a text is.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn koo_dec_parse(text: *const Header) -> i64 {
    unsafe { kept(decimal::parse(bytes(text)).map(|value| (value, false))) }
}

/// A whole number as a `Decimal`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn koo_dec_of(value: i64) -> i64 {
    unsafe { thread::me().decimal = Dec::of(value) };
    0
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn koo_dec_hi() -> i64 {
    unsafe { thread::me().decimal.hi as i64 }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn koo_dec_lo() -> i64 {
    unsafe { thread::me().decimal.lo as i64 }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn koo_dec_exp() -> i64 {
    unsafe { thread::me().decimal.exp as i64 }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn koo_dec_neg() -> bool {
    unsafe { thread::me().decimal.neg }
}

/// The number as text (`decimal::text`).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn koo_dec_text(hi: i64, lo: i64, exp: i64, neg: bool) -> *mut Header {
    let mut out = [0u8; 48];
    let len = decimal::text(&of(hi, lo, exp, neg), &mut out);
    unsafe { new_str(&out[..len]) }
}
