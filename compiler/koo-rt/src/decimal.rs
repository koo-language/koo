//! The arithmetic of `Decimal` (Section 4.3): numbers of at most 34 decimal digits, with
//! a power of ten, as IEEE 754-2008 gives them in decimal128.
//!
//! A value is `coefficient × 10^exp`, the coefficient below 10^34 and the exponent from
//! -6176 to 6111, with a sign. There is no infinity and nothing that is not a number:
//! what would be one is said to have failed, and the program panics or is told.
//!
//! Sums, differences and products are exact where the exact result has at most 34
//! digits, and then keep the exponent that schoolbook arithmetic gives (`1.10 + 2.2` is
//! `3.30`); a result of more digits is rounded to 34, half to even. Nothing here divides
//! but `divide`, which is told how many decimals the quotient is to have and how to
//! round it.
//!
//! The work is done on whole numbers in thousand-millions, twelve of them: 108 digits,
//! which is more than the product of two coefficients (68) and more than any sum that
//! is not decided by one of its terms alone. Nothing here allocates, and nothing asks
//! the processor for arithmetic wider than 64 bits.

// (The loops below go over two or three rows of digits by their place, which is what
// they are about; an iterator over one of the rows would say less.)
#![allow(clippy::needless_range_loop)]

/// The most digits a coefficient has.
pub const DIGITS: usize = 34;
/// The least and the greatest exponent of a coefficient written as a whole number.
pub const LEAST: i32 = -6176;
pub const GREATEST: i32 = 6111;

/// What a coefficient is kept in two of: `hi × 10^18 + lo`.
const HALF: u64 = 1_000_000_000_000_000_000;
const LIMBS: usize = 12;
const BASE: u64 = 1_000_000_000;

const TENS: [u32; 10] = [
    1,
    10,
    100,
    1_000,
    10_000,
    100_000,
    1_000_000,
    10_000_000,
    100_000_000,
    1_000_000_000,
];

/// A decimal number. `hi` is below 10^16 and `lo` below 10^18; zero is never negative.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Dec {
    pub neg: bool,
    pub hi: u64,
    pub lo: u64,
    pub exp: i32,
}

/// How a result that cannot be kept whole is rounded.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Round {
    /// To the nearer, and of two that are as near, to the one whose last digit is even.
    HalfEven,
    /// To the nearer, and of two that are as near, to the one further from zero.
    HalfUp,
    /// Toward zero: the digits beyond are dropped.
    Down,
    /// Away from zero.
    Up,
    /// Toward what is less.
    Floor,
    /// Toward what is greater.
    Ceiling,
}

impl Round {
    pub fn of(number: i64) -> Round {
        match number {
            1 => Round::HalfUp,
            2 => Round::Down,
            3 => Round::Up,
            4 => Round::Floor,
            5 => Round::Ceiling,
            _ => Round::HalfEven,
        }
    }
}

/// Why there is no result.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Problem {
    /// The result is greater than a `Decimal` can be.
    TooGreat,
    /// The divisor is zero.
    ByZero,
    /// The result has more than 34 digits where it was to be exact.
    TooLong,
    /// The text is not a number.
    NotANumber,
}

/// A whole number of up to 108 digits, least thousand-million first.
#[derive(Clone, Copy, PartialEq, Eq)]
struct Wide([u32; LIMBS]);

impl Wide {
    const ZERO: Wide = Wide([0; LIMBS]);

    fn of(hi: u64, lo: u64) -> Wide {
        let mut limbs = [0u32; LIMBS];
        limbs[0] = (lo % BASE) as u32;
        limbs[1] = (lo / BASE) as u32;
        limbs[2] = (hi % BASE) as u32;
        limbs[3] = (hi / BASE) as u32;
        Wide(limbs)
    }

    fn small(value: u32) -> Wide {
        let mut limbs = [0u32; LIMBS];
        limbs[0] = value % BASE as u32;
        limbs[1] = value / BASE as u32;
        Wide(limbs)
    }

    /// The two halves of a coefficient, for a number of at most 34 digits.
    fn halves(&self) -> (u64, u64) {
        let lo = self.0[1] as u64 * BASE + self.0[0] as u64;
        let hi = self.0[3] as u64 * BASE + self.0[2] as u64;
        (hi, lo)
    }

    fn is_zero(&self) -> bool {
        self.0.iter().all(|limb| *limb == 0)
    }

    /// How many digits it has; none for zero.
    fn digits(&self) -> usize {
        for at in (0..LIMBS).rev() {
            if self.0[at] != 0 {
                let mut count = 0;
                let mut limb = self.0[at];
                while limb != 0 {
                    limb /= 10;
                    count += 1;
                }
                return at * 9 + count;
            }
        }
        0
    }

    /// The digit that counts `10^at`.
    fn digit(&self, at: usize) -> u32 {
        if at >= LIMBS * 9 {
            return 0;
        }
        self.0[at / 9] / TENS[at % 9] % 10
    }

    /// Whether any digit below the one that counts `10^at` is not zero.
    fn any_below(&self, at: usize) -> bool {
        let (limb, within) = (at / 9, at % 9);
        if limb >= LIMBS {
            return !self.is_zero();
        }
        self.0[..limb].iter().any(|low| *low != 0) || !self.0[limb].is_multiple_of(TENS[within])
    }

    fn compare(&self, other: &Wide) -> core::cmp::Ordering {
        for at in (0..LIMBS).rev() {
            if self.0[at] != other.0[at] {
                return self.0[at].cmp(&other.0[at]);
            }
        }
        core::cmp::Ordering::Equal
    }

    fn add(&self, other: &Wide) -> Wide {
        let mut out = [0u32; LIMBS];
        let mut carry = 0u64;
        for at in 0..LIMBS {
            let sum = self.0[at] as u64 + other.0[at] as u64 + carry;
            out[at] = (sum % BASE) as u32;
            carry = sum / BASE;
        }
        Wide(out)
    }

    /// `self - other`, where `self` is not the less.
    fn sub(&self, other: &Wide) -> Wide {
        let mut out = [0u32; LIMBS];
        let mut borrow = 0i64;
        for at in 0..LIMBS {
            let mut difference = self.0[at] as i64 - other.0[at] as i64 - borrow;
            borrow = 0;
            if difference < 0 {
                difference += BASE as i64;
                borrow = 1;
            }
            out[at] = difference as u32;
        }
        Wide(out)
    }

    /// Times a number below a thousand million.
    fn times_small(&self, by: u32) -> Wide {
        let mut out = [0u32; LIMBS];
        let mut carry = 0u64;
        for at in 0..LIMBS {
            let product = self.0[at] as u64 * by as u64 + carry;
            out[at] = (product % BASE) as u32;
            carry = product / BASE;
        }
        Wide(out)
    }

    /// Times `10^count`. What is asked for fits: the callers see to it.
    fn shifted_up(&self, count: usize) -> Wide {
        let (limbs, within) = (count / 9, count % 9);
        let mut out = [0u32; LIMBS];
        for at in (limbs..LIMBS).rev() {
            out[at] = self.0[at - limbs];
        }
        Wide(out).times_small(TENS[within])
    }

    /// Without its last `count` digits, which are dropped and not rounded.
    fn shifted_down(&self, count: usize) -> Wide {
        let (limbs, within) = (count / 9, count % 9);
        if limbs >= LIMBS {
            return Wide::ZERO;
        }
        let mut out = [0u32; LIMBS];
        out[..LIMBS - limbs].copy_from_slice(&self.0[limbs..]);
        if within > 0 {
            let by = TENS[within] as u64;
            let mut rest = 0u64;
            for at in (0..LIMBS).rev() {
                let all = rest * BASE + out[at] as u64;
                out[at] = (all / by) as u32;
                rest = all % by;
            }
        }
        Wide(out)
    }

    /// The product of two numbers of four thousand-millions each at the most.
    fn times(&self, other: &Wide) -> Wide {
        let mut out = [0u64; LIMBS];
        for a in 0..4 {
            let mut carry = 0u64;
            for b in 0..4 {
                let all = out[a + b] + self.0[a] as u64 * other.0[b] as u64 + carry;
                out[a + b] = all % BASE;
                carry = all / BASE;
            }
            out[a + 4] += carry;
        }
        let mut limbs = [0u32; LIMBS];
        for at in 0..LIMBS {
            limbs[at] = out[at] as u32;
        }
        Wide(limbs)
    }

    /// Without its last `count` digits, rounded as `round` says for a number whose
    /// sign is `neg`. Says also whether anything but zeros was dropped.
    fn rounded(&self, count: usize, round: Round, neg: bool) -> (Wide, bool) {
        if count == 0 {
            return (*self, false);
        }
        let kept = self.shifted_down(count);
        let (first, rest) = (self.digit(count - 1), self.any_below(count - 1));
        let inexact = first != 0 || rest;
        let odd = kept.0[0] % 2 == 1;
        let more = match round {
            Round::HalfEven => first > 5 || (first == 5 && (rest || odd)),
            Round::HalfUp => first >= 5,
            Round::Down => false,
            Round::Up => inexact,
            Round::Floor => inexact && neg,
            Round::Ceiling => inexact && !neg,
        };
        (if more { kept.add(&Wide::small(1)) } else { kept }, inexact)
    }

    /// `self / by` and what is left over, a digit of the quotient at a time, as it
    /// is done on paper. `by` is not zero and has at most 98 digits.
    fn divided(&self, by: &Wide) -> (Wide, Wide) {
        let mut quotient = Wide::ZERO;
        let mut left = Wide::ZERO;
        for at in (0..self.digits()).rev() {
            left = left.times_small(10).add(&Wide::small(self.digit(at)));
            let mut digit = 0;
            while left.compare(by) != core::cmp::Ordering::Less {
                left = left.sub(by);
                digit += 1;
            }
            quotient = quotient.times_small(10).add(&Wide::small(digit));
        }
        (quotient, left)
    }
}

impl Dec {
    pub const ZERO: Dec = Dec {
        neg: false,
        hi: 0,
        lo: 0,
        exp: 0,
    };

    /// A whole number.
    pub fn of(value: i64) -> Dec {
        let size = value.unsigned_abs();
        Dec {
            neg: value < 0,
            hi: size / HALF,
            lo: size % HALF,
            exp: 0,
        }
    }

    pub fn is_zero(&self) -> bool {
        self.hi == 0 && self.lo == 0
    }

    fn wide(&self) -> Wide {
        Wide::of(self.hi, self.lo)
    }

    /// How many digits the coefficient has: at least one.
    pub fn digits(&self) -> usize {
        self.wide().digits().max(1)
    }

    pub fn negated(&self) -> Dec {
        Dec {
            neg: !self.neg && !self.is_zero(),
            ..*self
        }
    }

    /// The same number with no zeros at the end of its coefficient: two numbers that
    /// are equal are the same after this, whatever their exponents were.
    pub fn reduced(&self) -> Dec {
        if self.is_zero() {
            return Dec::ZERO;
        }
        let mut wide = self.wide();
        let mut exp = self.exp;
        while wide.digit(0) == 0 && exp < GREATEST {
            wide = wide.shifted_down(1);
            exp += 1;
        }
        let (hi, lo) = wide.halves();
        Dec {
            neg: self.neg,
            hi,
            lo,
            exp,
        }
    }
}

/// A number of a sign, digits and an exponent as a `Dec`: rounded to 34 digits if it
/// has more, and its exponent brought into range if that can be done. Says whether
/// anything was lost.
fn finished(neg: bool, mut wide: Wide, mut exp: i64, round: Round) -> Result<(Dec, bool), Problem> {
    let mut inexact = false;
    // What is dropped is dropped once: the digits beyond 34, or, for a number too
    // small for all its digits to be kept, those below the least exponent, whichever
    // are more. (Rounding twice would round some numbers the wrong way.)
    let count = wide.digits();
    let mut drop = count.saturating_sub(DIGITS) as i64;
    if !wide.is_zero() && exp + drop < LEAST as i64 {
        drop = LEAST as i64 - exp;
    }
    if drop > 0 {
        let (kept, lost) = wide.rounded(drop.min(200) as usize, round, neg);
        wide = kept;
        exp += drop;
        inexact = lost;
        // (Rounded up to a one and thirty-four zeros.)
        if wide.digits() > DIGITS {
            wide = wide.shifted_down(1);
            exp += 1;
        }
    }
    if wide.is_zero() {
        let exp = exp.clamp(LEAST as i64, GREATEST as i64) as i32;
        return Ok((
            Dec {
                neg: false,
                hi: 0,
                lo: 0,
                exp,
            },
            inexact,
        ));
    }
    if exp > GREATEST as i64 {
        // Zeros at the end of the coefficient stand for the powers of ten that the
        // exponent has no room for, while the coefficient has room for them.
        let over = exp - GREATEST as i64;
        if over > DIGITS as i64 || wide.digits() + over as usize > DIGITS {
            return Err(Problem::TooGreat);
        }
        wide = wide.shifted_up(over as usize);
        exp = GREATEST as i64;
    }
    let (hi, lo) = wide.halves();
    Ok((
        Dec {
            neg,
            hi,
            lo,
            exp: exp as i32,
        },
        inexact,
    ))
}

/// The sum, and whether it had to be rounded.
pub fn add(a: &Dec, b: &Dec) -> Result<(Dec, bool), Problem> {
    // The one with the greater exponent is written out to the exponent of the other,
    // so that both count the same power of ten.
    let (high, low) = if a.exp >= b.exp { (a, b) } else { (b, a) };
    let apart = (high.exp as i64 - low.exp as i64) as usize;
    let (mut high_wide, low_wide) = (high.wide(), low.wide());
    let mut exp = low.exp as i64;
    let mut dust = false;
    if high_wide.is_zero() {
        // (Nothing to write out; and the sum keeps the lesser exponent.)
    } else if high_wide.digits() + apart <= LIMBS * 9 - 2 {
        high_wide = high_wide.shifted_up(apart);
    } else {
        // The other is so far below that not one of its digits reaches the 34 that
        // are kept: it is there only as what makes the result not exact, and which
        // way it is rounded. One at the place 38 digits below says as much.
        let room = DIGITS + 4;
        high_wide = high_wide.shifted_up(room);
        exp = high.exp as i64 - room as i64;
        dust = !low_wide.is_zero();
    }
    let low_wide = if dust { Wide::small(1) } else { low_wide };
    let (neg, wide) = if high.neg == low.neg {
        (high.neg, high_wide.add(&low_wide))
    } else {
        match high_wide.compare(&low_wide) {
            core::cmp::Ordering::Less => (low.neg, low_wide.sub(&high_wide)),
            _ => (high.neg, high_wide.sub(&low_wide)),
        }
    };
    finished(neg, wide, exp, Round::HalfEven)
}

pub fn sub(a: &Dec, b: &Dec) -> Result<(Dec, bool), Problem> {
    add(a, &b.negated())
}

/// The product, and whether it had to be rounded.
pub fn mul(a: &Dec, b: &Dec) -> Result<(Dec, bool), Problem> {
    let wide = a.wide().times(&b.wide());
    finished(a.neg != b.neg, wide, a.exp as i64 + b.exp as i64, Round::HalfEven)
}

/// Which is the greater: by what they are worth, so that `1.0` and `1.00` are equal.
pub fn compare(a: &Dec, b: &Dec) -> core::cmp::Ordering {
    use core::cmp::Ordering::*;
    match (a.is_zero(), b.is_zero()) {
        (true, true) => return Equal,
        (true, false) => return if b.neg { Greater } else { Less },
        (false, true) => return if a.neg { Less } else { Greater },
        _ => {}
    }
    if a.neg != b.neg {
        return if a.neg { Less } else { Greater };
    }
    // By where the first digit of each stands, and then digit for digit.
    let (first_a, first_b) = (a.digits() as i64 + a.exp as i64, b.digits() as i64 + b.exp as i64);
    let sizes = if first_a != first_b {
        first_a.cmp(&first_b)
    } else {
        let (wide_a, wide_b) = (a.wide(), b.wide());
        if a.exp >= b.exp {
            wide_a.shifted_up((a.exp - b.exp) as usize).compare(&wide_b)
        } else {
            wide_a.compare(&wide_b.shifted_up((b.exp - a.exp) as usize))
        }
    };
    if a.neg { sizes.reverse() } else { sizes }
}

/// The same number with `scale` digits after the point, rounded as said if it had
/// more. One that would need more than 34 digits to be written so has no such form.
pub fn rescaled(a: &Dec, scale: i64, round: Round) -> Result<(Dec, bool), Problem> {
    divide(a, &Dec::of(1), scale, round)
}

/// The quotient with `scale` digits after the point, rounded as said.
pub fn divide(a: &Dec, b: &Dec, scale: i64, round: Round) -> Result<(Dec, bool), Problem> {
    if b.is_zero() {
        return Err(Problem::ByZero);
    }
    if scale > -(LEAST as i64) || scale < -(GREATEST as i64) {
        return Err(Problem::TooLong);
    }
    let neg = a.neg != b.neg;
    if a.is_zero() {
        return Ok((
            Dec {
                neg: false,
                hi: 0,
                lo: 0,
                exp: -scale as i32,
            },
            false,
        ));
    }
    // The quotient counts `10^-scale`: the dividend is written out by as many powers
    // of ten as that asks, or the divisor is, whichever it comes to.
    let by = a.exp as i64 - b.exp as i64 + scale;
    let (mut above, mut below) = (a.wide(), b.wide());
    if by >= 0 {
        // A quotient of more than 34 digits is none: with a dividend of at least one
        // digit and a divisor of at most 34, `by` beyond 68 is always such a one.
        if by > 2 * DIGITS as i64 {
            return Err(Problem::TooLong);
        }
        above = above.shifted_up(by as usize);
    } else {
        // A divisor more than 36 digits longer than the dividend makes a quotient
        // that is nothing but what is rounded.
        let down = (-by).min(40) as usize;
        below = below.shifted_up(down);
    }
    let (quotient, left) = above.divided(&below);
    // What is left over against half the divisor says which way to round.
    let twice = left.add(&left);
    let inexact = !left.is_zero();
    let half = twice.compare(&below);
    let odd = quotient.0[0] % 2 == 1;
    use core::cmp::Ordering::*;
    let more = match round {
        Round::HalfEven => half == Greater || (half == Equal && odd),
        Round::HalfUp => half != Less,
        Round::Down => false,
        Round::Up => inexact,
        Round::Floor => inexact && neg,
        Round::Ceiling => inexact && !neg,
    };
    let quotient = if more { quotient.add(&Wide::small(1)) } else { quotient };
    if quotient.digits() > DIGITS {
        return Err(Problem::TooLong);
    }
    let (hi, lo) = quotient.halves();
    Ok((
        Dec {
            neg: neg && !quotient.is_zero(),
            hi,
            lo,
            exp: -scale as i32,
        },
        inexact,
    ))
}

/// The number that a text is: digits with at most one point, a sign before them if
/// any, and after them, if any, `e` or `E` and a power of ten. More than 34 digits
/// that count are not rounded: such a text is no `Decimal`.
pub fn parse(text: &[u8]) -> Result<Dec, Problem> {
    let (neg, rest) = match text {
        [b'-', rest @ ..] => (true, rest),
        [b'+', rest @ ..] => (false, rest),
        rest => (false, rest),
    };
    let mut wide = Wide::ZERO;
    let (mut seen, mut point, mut after, mut count) = (false, false, 0i64, 0usize);
    let mut at = 0;
    while at < rest.len() {
        match rest[at] {
            digit @ b'0'..=b'9' => {
                seen = true;
                if count > 0 || digit != b'0' {
                    count += 1;
                    if count > DIGITS {
                        return Err(Problem::TooLong);
                    }
                }
                wide = wide.times_small(10).add(&Wide::small((digit - b'0') as u32));
                if point {
                    after += 1;
                }
            }
            b'.' if !point => point = true,
            b'e' | b'E' => break,
            _ => return Err(Problem::NotANumber),
        }
        at += 1;
    }
    if !seen {
        return Err(Problem::NotANumber);
    }
    let mut power = 0i64;
    if at < rest.len() {
        let (minus, digits) = match &rest[at + 1..] {
            [b'-', digits @ ..] => (true, digits),
            [b'+', digits @ ..] => (false, digits),
            digits => (false, digits),
        };
        if digits.is_empty() || digits.len() > 7 {
            return Err(Problem::NotANumber);
        }
        for digit in digits {
            if !digit.is_ascii_digit() {
                return Err(Problem::NotANumber);
            }
            power = power * 10 + (digit - b'0') as i64;
        }
        if minus {
            power = -power;
        }
    }
    // What is too great is not a number here; what is too small to be told from zero
    // without loss is not either, a text being something to read exactly.
    match finished(neg, wide, power - after, Round::HalfEven) {
        Ok((value, false)) => Ok(value),
        Ok((_, true)) => Err(Problem::TooLong),
        Err(problem) => Err(problem),
    }
}

/// The text of a number, and how long it is: its digits, with a point as many from
/// their end as the exponent is below zero, so that `1.50` is written with its zero.
/// A number whose exponent is above zero, or that is less than a millionth, is
/// written with its first digit, the rest after a point, and `E` and a power of ten,
/// as the standard for decimal arithmetic writes them.
pub fn text(value: &Dec, out: &mut [u8; 48]) -> usize {
    let mut digits = [0u8; DIGITS];
    let count = value.digits();
    let wide = value.wide();
    for at in 0..count {
        digits[at] = b'0' + wide.digit(count - 1 - at) as u8;
    }
    let mut len = 0;
    let mut push = |byte: u8, len: &mut usize| {
        out[*len] = byte;
        *len += 1;
    };
    if value.neg {
        push(b'-', &mut len);
    }
    // Where the first digit stands, as a power of ten.
    let first = value.exp as i64 + count as i64 - 1;
    if value.exp <= 0 && first >= -6 {
        let before = count as i64 + value.exp as i64;
        if before <= 0 {
            push(b'0', &mut len);
            push(b'.', &mut len);
            for _ in 0..-before {
                push(b'0', &mut len);
            }
            for digit in &digits[..count] {
                push(*digit, &mut len);
            }
        } else {
            for (at, digit) in digits[..count].iter().enumerate() {
                if at as i64 == before {
                    push(b'.', &mut len);
                }
                push(*digit, &mut len);
            }
        }
        return len;
    }
    push(digits[0], &mut len);
    if count > 1 {
        push(b'.', &mut len);
        for digit in &digits[1..count] {
            push(*digit, &mut len);
        }
    }
    push(b'E', &mut len);
    push(if first < 0 { b'-' } else { b'+' }, &mut len);
    let mut power = [0u8; 5];
    let mut size = first.unsigned_abs();
    let mut places = 0;
    loop {
        power[places] = b'0' + (size % 10) as u8;
        places += 1;
        size /= 10;
        if size == 0 {
            break;
        }
    }
    for at in (0..places).rev() {
        push(power[at], &mut len);
    }
    len
}
