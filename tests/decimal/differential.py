#!/usr/bin/env python3
"""The arithmetic of `Decimal` against another's: Python's `decimal`, set as IEEE 754
sets decimal128, and for what divides, exact fractions, which round nothing until told.

    python3 tests/decimal/differential.py [--cases 200000] [--seed 1]

It builds `tests/decimal/harness.rs`, which is the arithmetic of the support library by
itself, throws sums at it, and compares each answer: the digits, the exponent (so that
`3.30` is not taken for `3.3`), and whether the answer says it was rounded. A run says
its seed, and the same seed throws the same again.

Where Koo's `Decimal` is by design not Python's, the expectation here says so: zero is
never negative; a text of more than 34 digits that count is refused, not rounded (zeros
at its end counting only as far as there is no room for them); `{d:.N}` has no sign
where it is all zeros; and a
quotient is asked for with its number of decimals, and is none if it would need more
than 34 digits.
"""
import decimal, os, random, subprocess, sys, tempfile
from decimal import Decimal
from fractions import Fraction

args = sys.argv[1:]


def option(name, default):
    return args[args.index(name) + 1] if name in args else default


CASES = int(option("--cases", "200000"))
SEED = int(option("--seed", str(random.SystemRandom().randrange(1 << 30))))
rng = random.Random(SEED)
root = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
LEAST, GREATEST, DIGITS = -6176, 6111, 34


def context():
    made = decimal.Context(prec=DIGITS, Emax=6144, Emin=-6143, rounding=decimal.ROUND_HALF_EVEN, clamp=1)
    made.traps = dict.fromkeys(made.traps, False)
    return made


def digits_of(count):
    first = str(rng.randrange(1, 10))
    return first + "".join(rng.choice("0123456789") if rng.random() < 0.8 else "0" for _ in range(count - 1))


def number():
    """A number as text: of the sizes money has, of every length a coefficient can have,
    at the ends of what an exponent can be, and the awkward ones by name."""
    kind = rng.random()
    sign = "-" if rng.random() < 0.3 else ""
    if kind < 0.05:
        return sign + rng.choice(["0", "0.00", "0E+3", "0E-6176", "1", "5", "0.5", "0.05", "10", "9" * 34, "1" + "0" * 33,
                                  "9" * 34 + "E+6111", "1E-6176", "1E+6144", "5E-6177".replace("5E-6177", "5E-6176"),
                                  "4999999999999999999999999999999999", "5000000000000000000000000000000000"])
    if kind < 0.45:
        count = rng.randrange(1, 12)
        exp = -rng.choice([0, 0, 1, 2, 2, 2, 3, 4, 6, 8])
    elif kind < 0.85:
        count = rng.randrange(1, DIGITS + 1)
        exp = rng.randrange(-40, 12)
    else:
        count = rng.randrange(1, DIGITS + 1)
        exp = rng.choice([rng.randrange(LEAST, LEAST + 80), rng.randrange(GREATEST - 80, GREATEST + 1), rng.randrange(-200, 200)])
    return f"{sign}{digits_of(count)}E{exp:+d}" if exp else sign + digits_of(count)


def shown(value):
    """As the harness shows it: zero without a sign."""
    if value.is_zero():
        value = value.copy_abs()
    return str(value)


def of_parts(sign, size, exp):
    return Decimal((sign, tuple(int(d) for d in str(size)), exp))


def rounded(q, mode, negative):
    """A fraction that is not negative, as a whole number, by one of the six ways."""
    whole, left = divmod(q.numerator, q.denominator)
    twice = 2 * left
    if left == 0:
        return whole, False
    up = {0: twice > q.denominator or (twice == q.denominator and whole % 2 == 1),
          1: twice >= q.denominator, 2: False, 3: True, 4: negative, 5: not negative}[mode]
    return whole + (1 if up else 0), True


def operand(text):
    """An operand as a decimal128 holds it: one whose exponent is beyond the greatest is
    written with zeros at its end instead, which the answer's exponent then follows."""
    return context().create_decimal(text)


def expect(line):
    words = line.split()
    op = words[0]
    ctx = context()
    if op in ("add", "sub", "mul"):
        a, b = operand(words[1]), operand(words[2])
        result = {"add": ctx.add, "sub": ctx.subtract, "mul": ctx.multiply}[op](a, b)
        if result.is_infinite():
            return "TooGreat"
        return f"{shown(result)} {'inexact' if ctx.flags[decimal.Inexact] else 'exact'}"
    if op == "cmp":
        a, b = operand(words[1]), operand(words[2])
        return {-1: "Less", 0: "Equal", 1: "Greater"}[int(a.compare(b))]
    if op in ("div", "rescale"):
        a = operand(words[1])
        b = operand(words[2]) if op == "div" else Decimal(1)
        scale, mode = (int(words[3]), int(words[4])) if op == "div" else (int(words[2]), int(words[3]))
        if b.is_zero():
            return "ByZero"
        if scale > -LEAST or scale < -GREATEST:
            return "TooLong"
        if a.is_zero():
            return f"{shown(of_parts(0, 0, -scale))} exact"
        negative = a.is_signed() != b.is_signed()
        q = abs(Fraction(a) / Fraction(b)) * Fraction(10) ** scale
        size, inexact = rounded(q, mode, negative)
        if size >= 10 ** DIGITS:
            return "TooLong"
        sign = 1 if negative and size else 0
        return f"{shown(of_parts(sign, size, -scale))} {'inexact' if inexact else 'exact'}"
    if op == "parse":
        text = words[1]
        exact = Decimal(text)
        counted = len(("".join(map(str, exact.as_tuple().digits)).lstrip("0")).rstrip("0"))
        if not exact.is_zero() and counted > DIGITS:
            return "TooLong"
        made = ctx.create_decimal(text)
        if made.is_infinite():
            return "TooGreat"
        if ctx.flags[decimal.Inexact]:
            return "TooLong"
        return shown(made)
    if op == "reduce":
        return shown(ctx.normalize(ctx.create_decimal(words[1])))
    if op == "fixed":
        a, places = operand(words[1]), int(words[2])
        wide = decimal.Context(prec=10000, Emax=10**6, Emin=-10**6, rounding=decimal.ROUND_HALF_EVEN)
        written = format(wide.quantize(a, Decimal(1).scaleb(-places)), "f")
        return written.lstrip("-") if set(written) <= set("-0.") else written
    if op == "hash":
        a, b = operand(words[1]), operand(words[2])
        return "true" if a == b else "false"
    raise ValueError(op)


def case():
    kind = rng.random()
    if kind < 0.25:
        return f"add {number()} {number()}"
    if kind < 0.40:
        return f"sub {number()} {number()}"
    if kind < 0.65:
        return f"mul {number()} {number()}"
    if kind < 0.72:
        a = number()
        return f"cmp {a} {a if rng.random() < 0.2 else number()}"
    if kind < 0.88:
        return f"div {number()} {number()} {rng.choice([0, 2, 2, 2, 4, 6, 10, 20, 33, rng.randrange(-5, 40)])} {rng.randrange(6)}"
    if kind < 0.94:
        return f"rescale {number()} {rng.choice([0, 1, 2, 2, 3, 6, rng.randrange(-10, 40)])} {rng.randrange(6)}"
    if kind < 0.955:
        return f"parse {number()}"
    if kind < 0.965:
        # Digits with zeros at their end, more of them than there may be room for.
        digits = digits_of(rng.randrange(1, DIGITS + 1)) + "0" * rng.randrange(0, 12)
        point = rng.randrange(len(digits) + 1)
        text = digits[:point] + ("." + digits[point:] if point < len(digits) and rng.random() < 0.5 else digits[point:])
        return f"parse {text}E{rng.randrange(-30, 30):+d}" if rng.random() < 0.5 else f"parse {text}"
    if kind < 0.98:
        return f"fixed {number()} {rng.choice([0, 1, 2, 2, 2, 4, 8, rng.randrange(0, 101)])}"
    if kind < 0.99:
        a = number()
        same = context().create_decimal(a)
        twin = str(same.quantize(Decimal(1).scaleb(same.as_tuple().exponent - rng.randrange(0, 4)), context=decimal.Context(prec=60, Emax=10**6, Emin=-10**6))) if not same.is_infinite() else a
        return f"hash {a} {twin if rng.random() < 0.6 else number()}"
    return f"reduce {number()}"


def in_range(line):
    """Whether every operand is a number at all: the generator makes some that are too
    great or too small to be one, which only `parse` is asked about."""
    words = line.split()
    count = {"add": 2, "sub": 2, "mul": 2, "cmp": 2, "div": 2, "rescale": 1, "parse": 0, "reduce": 1, "fixed": 1, "hash": 2}[words[0]]
    for text in words[1:1 + count]:
        if expect(f"parse {text}") in ("TooGreat", "TooLong"):
            return False
    return True


build = tempfile.mkdtemp(prefix="koo-decimal-")
harness = os.path.join(build, "harness")
built = subprocess.run(["rustc", "--edition", "2024", "-O", os.path.join(root, "tests/decimal/harness.rs"), "-o", harness],
                       capture_output=True, text=True)
if built.returncode != 0:
    print("FAIL  the harness was not built\n" + built.stderr)
    sys.exit(1)

lines = []
while len(lines) < CASES:
    line = case()
    if in_range(line):
        lines.append(line)
answers = subprocess.run([harness], input="\n".join(lines) + "\n", capture_output=True, text=True)
if answers.returncode != 0:
    print("FAIL  the harness ended with status", answers.returncode, "\n" + answers.stderr[-2000:])
    sys.exit(1)
got = answers.stdout.splitlines()
wrong = []
kinds = {}
for line, answer in zip(lines, got):
    expected = expect(line)
    kinds[line.split()[0]] = kinds.get(line.split()[0], 0) + 1
    if answer != expected:
        wrong.append((line, expected, answer))
print(f"seed {SEED}: {len(lines)} sums ({', '.join(f'{count} {kind}' for kind, count in sorted(kinds.items()))})")
if len(got) != len(lines):
    print(f"FAIL  {len(lines)} were asked and {len(got)} answered")
    sys.exit(1)
if wrong:
    print(f"FAIL  {len(wrong)} are not what Python's are:")
    for line, expected, answer in wrong[:12]:
        print(f"    {line}\n        expected {expected}\n        found    {answer}")
    sys.exit(1)
print("all as Python's")
