# Chapter 2: Types, Operators and Expressions

Variables and constants are the basic data objects manipulated in a program. Declarations give them names and first values. Operators specify what is to be done to them. Expressions combine variables and constants to produce new values. The type of an object determines the set of values it can have and what operations can be performed on it. These building blocks are the topics of this chapter.

## 2.1 Names

Although we did not say so in Chapter 1, there are some restrictions on the names of variables and functions. Names are made up of the letters `a` to `z` and `A` to `Z`, the digits, and the underscore `_`; the first character cannot be a digit.

Upper and lower case mean something. The name of a variable, a constant, a function, a parameter or a field begins with a lower case letter or an underscore and is written in what is called snake case, the words in lower case with underscores between them: `line_count`, `read_line`. The name of a type begins with an upper case letter, and the words in it are run together: `Point`, `OrderLine`. The compiler holds you to this. A name of the wrong shape is an error, and because of that the shape of a name tells you, and the compiler, what kind of thing it names.

The following words are part of the language, and cannot be used as names:

    actor      distinct   fn         is         resource   take
    and        else       for        match      return     type
    as         enum       if         not        scope      unsafe
    break      error      import     or         select     var
    continue   fail       in         pub        spawn      while
    defer      interface

Some of them belong to parts of the language that the compiler does not build yet, and are reserved so that programs written today do not have to change. The words `true`, `false` and `none` are constants, and `self` is the name a method gives to the value it belongs to (Chapter 6).

It is wise to choose names that are related to the purpose of what they name, and that are unlikely to get mixed up typographically. We tend to use short names for the variables of a few lines, especially loop indices, and longer names for constants and functions that are used far from where they are declared.

## 2.2 Types and Sizes

There are only a few basic types in Koo:

| | |
| --- | --- |
| `int` | an integer of 64 bits, with a sign |
| `float` | a floating-point number of 64 bits |
| `bool` | a truth value, `true` or `false` |
| `byte` | an integer from 0 to 255 |
| `string` | text: a sequence of characters, of any length |

There is one size of integer and one of floating-point number. The compiler knows the names that other languages give to other sizes, such as `i32` and `f32`, only well enough to say that it does not have them. An `int` holds the values from −9223372036854775808 to 9223372036854775807. A `float` is a number in the IEEE format that nearly every machine uses, with about sixteen significant decimal digits.

From the basic types the others are built: lists, maps and sets (Chapter 5), tuples, structures and enumerations (Chapter 6), optionals, which were introduced in Chapter 1, and functions, since a function is also a value (Chapter 4).

## 2.3 Constants

An integer constant like `1234` is an `int`. An integer can also be written in hexadecimal, with a leading `0x`, or in binary, with a leading `0b`. Underscores may be put between the digits of any number to make it easier to read, and mean nothing.

A constant with a decimal point (`123.4`) or an exponent (`1e-2`), or both, is a `float`. There must be a digit on each side of the decimal point.

```koo
fn main() {
    print(1_000_000)
    print(0xff)
    print(0b1010)
    print(123.4)
    print(1e-2)
    print(6.02e23)
    print(1e9)
}
```

```text
1000000
255
10
123.4
0.01
6.02e23
1000000000.0
```

A *string constant* is a sequence of characters surrounded by double quotes, as in `"I am a string"`. Certain characters can be represented in strings by *escape sequences* like `\n` (newline); these sequences look like two characters, but represent only one. The complete set of escape sequences is

| | | | |
| --- | --- | --- | --- |
| `\n` | newline | `\\` | backslash |
| `\r` | carriage return | `\"` | double quote |
| `\t` | horizontal tab | `\'` | single quote |
| `\0` | the character whose code is zero | `\{` | opening brace |
| `\u{`*hhhh*`}` | the character with the hexadecimal code *hhhh* | | |

Any other character after a backslash is an error. As we saw in Chapter 1, an opening brace in a string begins an expression, which is why a brace that is meant as a brace is written `\{`.

There are two more ways to write a string. A *raw string* is enclosed in backquotes. Nothing in it is special: a backslash is a backslash and a brace is a brace, which is convenient for text that is full of them. A *text block* is enclosed by lines of three double quotes. It may extend over many lines, and the indentation of the closing quotes is removed from each of them, so that a block can be indented with the program around it. Escape sequences and expressions in braces work in a text block as in an ordinary string.

```koo
fn main() {
    name := "world"
    print("hello, {name}\n\tindented \{name}")
    print(`a raw string: \n and {name} stay as they are`)
    report := """
        Dear {name},
            all is well.
        """
    print(report)
    print("caf\u{e9}")
}
```

```text
hello, world
	indented {name}
a raw string: \n and {name} stay as they are
Dear world,
    all is well.
café
```

The constants `true` and `false` are the two values of type `bool`. The constant `none` is the absent value of every optional type.

There is no constant of type `byte`. A byte is made from an integer by `byte.from`, of which more in Section 2.7. There is no character type either: a character is a string that happens to have one character in it.

## 2.4 Declarations

All variables must be declared before use, and a declaration always gives a value. There is no way to say "a variable `n`, of some value to be decided later", and so there is no such thing as a variable that was read before it was set.

    lower := 0
    var total := 0.0
    line := "now is the time"

The type is that of the value. When you want to state the type, it goes between the name and an `=`:

```koo
fn main() {
    limit: int = 100
    var ratio: float = 0.5
    var names: [string] = []
    answer: int? = none

    ratio = ratio * 2.0
    names.push("ada")
    print("{limit} {ratio} {names} {answer}")
}
```

```text
100 1.0 ["ada"] none
```

Writing the type is necessary when the value alone does not determine it. An empty list `[]` could be a list of anything, and `none` could be the absence of anything, so `names` and `answer` above could not have been declared without their types.

A name declared without `var` cannot be changed after its declaration. This is not a way of declaring constants on the cheap: it is the usual way to declare, and most names in most programs never change. The compiler tells you if you forgot the `var`, and shows where to put it:

```koo
fn main() {
    count := 0
    count += 1
    print(count)
}
```

```error
error: cannot assign to `count`, which is not `var`
 --> main.koo:3:5
  |
2 |     count := 0
  |     ----- `count` is declared here, without `var`
3 |     count += 1
  |     ^^^^^
  |
help: declare it with `var`
  |
2 |     var count := 0
  |     ++++

1 error
```

Declarations outside functions are constants for the whole program (Section 1.4). Their values are literal: a number, a string, `true` or `false`.

## 2.5 Arithmetic Operators

The binary arithmetic operators are `+`, `-`, `*`, `/`, and the remainder operator `%`. Both operands have the same type, `int` or `float`, and that is the type of the result. Integer division truncates any fractional part. The expression

    x % y

produces the remainder when `x` is divided by `y`, and thus is zero when `y` divides `x` exactly. For example, a year is a leap year if it is divisible by 4 but not by 100, except that years divisible by 400 *are* leap years. Therefore

```koo
fn main() {
    for year in [1900, 1996, 2000, 2023] {
        if (year % 4 == 0 and year % 100 != 0) or year % 400 == 0 {
            print("{year} is a leap year")
        } else {
            print("{year} is not a leap year")
        }
    }
}
```

```text
1900 is not a leap year
1996 is a leap year
2000 is a leap year
2023 is not a leap year
```

For negative operands, the direction of truncation and the sign of the remainder are defined, and are the same on every machine: division truncates toward zero, and the remainder has the sign of the dividend.

```koo
fn main() {
    print("{7 / 2} {-7 / 2} {7 / -2}")
    print("{7 % 2} {-7 % 2} {7 % -2}")
    print("{7.0 / 2.0} {7.5 % 2.0}")
}
```

```text
3 -3 -3
1 -1 1
3.5 1.5
```

The binary `+` and `-` operators have the same precedence, which is lower than the precedence of `*`, `/` and `%`, which is in turn lower than unary `-`. Arithmetic operators associate left to right. A method call binds more tightly than any of them, so `-2.pow(2)` is the negation of `2.pow(2)`. Section 2.12 has the table for all operators.

Integer arithmetic is *checked*. If the true result of an addition, a subtraction, a multiplication or a negation does not fit in an `int`, the program does not go on with some other number. It stops:

```koo
fn main() {
    var n := 9223372036854775807
    n += 1
    print(n)
}
```

```panic
panic: integer overflow
  at main.koo:3
```

A program that ends this way has *panicked*. A panic writes a message and the place it happened, and ends the program with an exit status of 101. Dividing an integer by zero is a panic too. The checks cost little, and the bugs they turn from wrong answers into messages are among the most expensive there are.

On the rare occasion when arithmetic is meant to wrap around, as in the computation of a hash value, the methods `wrapping_add`, `wrapping_sub` and `wrapping_mul` do so, and say so.

Floating-point arithmetic follows the rules of IEEE and never panics. A result too large is infinite, and a quotient of zeros is the special value "not a number".

```koo
fn main() {
    big := 9223372036854775807
    print(big.wrapping_add(1))
    print(1.0 / 0.0)
    print(0.0 / 0.0)
    print(0.1 + 0.2)
}
```

```text
-9223372036854775808
inf
NaN
0.30000000000000004
```

The last line is a reminder that a `float` is a binary fraction and that most decimal fractions are not exactly representable. It is not peculiar to Koo.

Besides the operators there are a few methods on numbers: `abs`, `min`, `max` and `clamp` on both types; `pow` on `int`; `sqrt` on `float`. Appendix B describes them.

```koo
fn main() {
    print(2.pow(10))
    print((-5).abs())
    print(3.min(8))
    print(15.clamp(0, 10))
    print(2.0.sqrt())
}
```

```text
1024
5
3
10
1.4142135623730951
```

## 2.6 Relational and Logical Operators

The relational operators are

    >   >=   <   <=

and the equality operators are

    ==   !=

All six have the same precedence, which is lower than that of the arithmetic operators, so an expression like `i < lim - 1` is taken as `i < (lim - 1)`, as would be expected.

Relational and equality operators do not associate. In mathematics `a < b < c` means that `b` lies between; in C it compiles, and means something else. In Koo it does not compile:

```koo
fn main() {
    age := 30
    print(18 <= age < 65)
}
```

```error
error: comparisons do not chain
 --> main.koo:3:21
  |
3 |     print(18 <= age < 65)
  |                     ^
  |
  = help: write `a < b and b < c`, or parenthesize one comparison

1 error
```

The equality operators apply to nearly every type, not only to numbers, and they compare *values*: two strings are equal if they have the same characters, two lists if they have equal elements in the same order, two structures if their fields are equal. It never matters where a value is stored or how it was arrived at. (The one exception is functions, and types that contain one, which cannot be compared.) The two sides of a comparison are of the same type. The relational operators apply to `int`, `float`, `byte` and `string`. Strings are ordered by the codes of their characters, so that upper case letters come before lower case ones.

```koo
fn main() {
    print([1, 2, 3] == [1, 2, 3])
    print("apple" < "banana")
    print("Zebra" < "apple")
    print((1, "one") == (1, "uno"))
}
```

```text
true
true
true
false
```

More interesting are the logical operators `and` and `or`. Expressions connected by `and` or `or` are evaluated left to right, and evaluation stops as soon as the truth or falsehood of the result is known. Most Koo programs rely on these properties. For example, here is a function that tells whether the element of a list at an index is positive, for any index at all:

```koo
fn positive_at(numbers: [int], i: int) -> bool {
    return i >= 0 and i < numbers.len and numbers[i] > 0
}

fn main() {
    numbers := [4, -2, 7]
    print(positive_at(numbers, 2))
    print(positive_at(numbers, 1))
    print(positive_at(numbers, 3))
}
```

```text
true
false
false
```

Before reading `numbers[i]` it is necessary to check that there is such an element. If the tests fail the element must not be read, and it is not.

The precedence of `and` is higher than that of `or`, and both are lower than relational and equality operators, so the expression above needs no parentheses. The operands of `and` and `or` are of type `bool`, and so is the condition of an `if` or a `while`. A number is not a truth value in Koo: `if n` is an error, and one writes `if n != 0`.

The unary operator `not` converts `true` into `false` and `false` into `true`. Its precedence is unusual, and convenient: it is *lower* than that of the relational operators, so that

    not i < numbers.len

means `not (i < numbers.len)`, as one would say it aloud.

**Exercise 2-1.** Write a loop equivalent to the one in the function above without using `and` or `or`.

## 2.7 Type Conversions

When an operator has operands of different types, C converts them to a common type by a small number of rules. Koo has no such rules, because it never converts a value from one type to another unless the program asks it to. A conversion is a function call, and it is spelled the same way for every pair of types: the name of the type wanted, a dot, and `from`, `parse`, or the like.

| To get | from | write | |
| --- | --- | --- | --- |
| a `float` | an `int` | `float.from(n)` | |
| an `int` | a `float` | `x.round()`, `x.floor()`, `x.ceil()`, `x.trunc()` | can fail |
| an `int` | a `byte` | `int.from(b)` | |
| a `byte` | an `int` | `byte.from(n)` | can fail |
| an `int` | a `string` | `int.parse(s)` | can fail |
| a `float` | a `string` | `float.parse(s)` | can fail |
| a `string` | anything | `value.to_string()`, or `"{value}"` | |

A conversion marked "can fail" does not always have an answer: there is no byte for 300, no integer for the text `"twelve"` or for a floating-point number too large to be one. Such a function returns either a value or an error, and Koo does not allow the caller to proceed as if the error could not happen. The full treatment is in Chapter 7; for now the operator `else`, which we met in Chapter 1, is enough. It supplies the value to use when there is none.

```koo
fn main() {
    n := 7
    x := 2.75
    print(float.from(n) / 2.0)
    print(x.round() else 0)
    print(x.floor() else 0)
    print((-x).trunc() else 0)

    print(int.parse("42") else -1)
    print(int.parse("forty-two") else -1)
    print(float.parse("2.5") else 0.0)

    b := byte.from(65) else 0
    print(int.from(b) + 1)
    print(byte.from(300) else 0)

    print(n.to_string() + "!")
}
```

```text
3.5
3
2
-2
42
-1
2.5
66
0
7!
```

`round` goes to the nearest integer, and when the fraction is exactly one half, away from zero. `floor` goes down, `ceil` up, and `trunc` toward zero.

The library conversions are the ones to use, but to see what one of them does, here is an implementation of the simplest case of `int.parse`, in the manner of the function `atoi` of C, which converts a string of digits into its numeric equivalent.

```koo
// Convert the digits at the beginning of s to an integer.
fn atoi(s: string) -> int {
    var n := 0
    for b in s.bytes() {
        digit := int.from(b) - 48
        if digit < 0 or digit > 9 {
            break
        }
        n = 10 * n + digit
    }
    return n
}

fn main() {
    print(atoi("1976"))
    print(atoi("12 monkeys"))
    print(atoi("none"))
}
```

```text
1976
12
0
```

`s.bytes()` is the bytes of the string, as a list of type `[byte]`. The digits have the codes 48 to 57, so `int.from(b) - 48` is the value of the digit that `b` is the code of, if it is a digit at all. The statement `break` leaves the loop; Chapter 3 says more about it.

**Exercise 2-2.** Write the function `htoi(s)`, which converts a string of hexadecimal digits (including an optional `0x` or `0X`) into its equivalent integer value. The allowable digits are `0` through `9`, `a` through `f`, and `A` through `F`.

## 2.8 Increment and Decrement

C provides two unusual operators for incrementing and decrementing variables, `++` and `--`. Koo does not. To add one to `n`, write

    n += 1

The compiler knows that programmers' fingers write `n++` unbidden, and answers accordingly:

```koo
fn main() {
    var n := 0
    n++
    print(n)
}
```

```error
error: `++` does not exist
 --> main.koo:3:6
  |
3 |     n++
  |      ^^
  |
help: write `+= 1`
  |
3 -     n++
3 +     n += 1

1 error
```

In C the operators earn their keep in expressions like `s[j++] = s[i]`, where a variable is used and stepped at once. Such code is rarer in Koo, because a loop usually runs over the elements of a thing and not over its indices, and a string is usually built by adding to it. Consider the function `squeeze(s, c)`, which removes all occurrences of the character `c` from the string `s`:

```koo
// Delete all c from s.
fn squeeze(s: string, c: string) -> string {
    var out := ""
    for ch in s.chars() {
        if ch != c {
            out += ch
        }
    }
    return out
}

fn main() {
    print(squeeze("mississippi", "s"))
}
```

```text
miiippi
```

**Exercise 2-3.** Write an alternate version of `squeeze(s1, s2)` that deletes each character in `s1` that matches any character in the string `s2`.

**Exercise 2-4.** Write the function `any(s1, s2)`, which returns the first position in the string `s1` where any character from the string `s2` occurs. What should it return if `s1` contains no characters from `s2`? (Consider an optional.)

## 2.9 Bitwise Operations

C provides six operators for bit manipulation. In Koo these operations are methods of `int`, with names in place of signs:

| | |
| --- | --- |
| `a.bit_and(b)` | bitwise AND |
| `a.bit_or(b)` | bitwise inclusive OR |
| `a.bit_xor(b)` | bitwise exclusive OR |
| `a.shl(n)` | `a` shifted left by `n` bits |
| `a.shr(n)` | `a` shifted right by `n` bits |

The right shift is arithmetic: the bits that come in at the left are copies of the sign bit, so that a negative number stays negative.

The bitwise AND is often used to mask off some set of bits; for example,

    n = n.bit_and(127)

sets to zero all but the low-order 7 bits of `n`. The bitwise OR is used to turn bits on. As an illustration, here is the function `bit_count`, which counts the number of 1-bits in its integer argument.

```koo
// Count the 1 bits in x.
fn bit_count(x: int) -> int {
    var count := 0
    for i in 0..64 {
        count += x.shr(i).bit_and(1)
    }
    return count
}

fn main() {
    print(bit_count(0))
    print(bit_count(255))
    print(bit_count(0b1011))
    print(bit_count(-1))
}
```

```text
0
8
3
64
```

The last line shows that −1 is represented by 64 one-bits: integers are in two's complement form.

**Exercise 2-5.** Write a function `get_bits(x, p, n)` that returns the `n`-bit field of `x` that begins at position `p`, counting the rightmost bit as position 0.

**Exercise 2-6.** In two's complement, `x.bit_and(x - 1)` deletes the rightmost 1-bit in `x`. Explain why. Use this observation to write a faster version of `bit_count` for arguments that are not negative.

## 2.10 Assignment

An assignment is a statement. Expressions such as

    i = i + 2

in which the variable on the left side is repeated immediately on the right, can be written in the compressed form

    i += 2

The operators `+=`, `-=`, `*=` and `/=` are the *assignment operators*. `+=` also adds to the end of a string, as in `squeeze` above. The left side of an assignment is a variable declared with `var`, or a part of one: an element of a list, a field of a structure, an entry of a map.

In C an assignment is an expression with a value, which makes it possible to write `while ((c = getchar()) != EOF)`, and also to write `if (x = 5)` when `if (x == 5)` was meant. In Koo an assignment has no value and cannot appear inside an expression, so the second cannot be written at all. The first is written with a binding condition, as in Chapter 1.

## 2.11 Conditional Expressions

The statements

    if a > b {
        z = a
    } else {
        z = b
    }

compute in `z` the maximum of `a` and `b`. For choices like this one, an `if` can be used as an expression. The value of

    if a > b { a } else { b }

is `a` if `a > b` is true, and `b` otherwise. Each branch holds one expression, the two are of the same type, and there must be an `else`, since the expression has to have a value either way. So the maximum can be declared in one line, and need not be a `var`:

```koo
fn main() {
    a := 12
    b := 7
    z := if a > b { a } else { b }
    print(z)

    for n in 0..4 {
        print("{n} item{if n == 1 { "" } else { "s" }}")
    }
}
```

```text
12
0 items
1 item
2 items
3 items
```

The second use is the typical one: a small variation inside a larger expression.

A block of several statements never has a value in Koo. A computation that takes more than one expression to produce its value belongs in a function.

The operator `else`, when it stands between two expressions and not after an `if`, is a different conditional: `x else y` is `x` if `x` is there, and `y` if it is not. We have used it for optionals and for conversions that fail. It is the lowest in precedence of all operators, and it associates to the right, so that alternatives can be tried in turn:

```koo
fn main() {
    typed := ""
    saved: int? = none
    print(int.parse(typed) else saved else 80)
}
```

```text
80
```

## 2.12 Precedence and Order of Evaluation

The table below summarizes the rules for precedence and associativity of all operators. Operators on the same line have the same precedence; rows are in order of decreasing precedence, so, for example, `*`, `/` and `%` all have the same precedence, which is higher than that of binary `+` and `-`.

| Operators | Associativity |
| --- | --- |
| `x.f`  `x.f()`  `f()`  `x[i]`  `x?` | left to right |
| `-` (unary) | |
| `*`  `/`  `%` | left to right |
| `+`  `-` | left to right |
| `..`  `..=` | none |
| `==`  `!=`  `<`  `<=`  `>`  `>=` | none |
| `not` | |
| `and` | left to right |
| `or` | left to right |
| `else` | right to left |

The first row is the operations that follow a value: selecting a field, calling a method or a function, subscripting, and the `?` of Chapter 7. The ranges `a..b` and `a..=b` were met in the `for` loops of Chapter 1.

The operands of an operator are evaluated from left to right, and so are the arguments of a function:

```koo
fn show(n: int) -> int {
    print("evaluating {n}")
    return n
}

fn main() {
    print(show(1) - show(2) * show(3))
}
```

```text
evaluating 1
evaluating 2
evaluating 3
-5
```

Precedence decides which operator each operand belongs to; it does not change the order in which the operands are computed. Even so, a program whose result depends on that order is hard to read, and since a function cannot change its caller's variables without a `var` at the call, such programs are harder to write by accident in Koo than in most languages.

A statement ends at the end of its line, and a long expression needs more than one. There are three ways for it to go on. Inside parentheses and square brackets the end of a line means nothing, so the arguments of a call or the elements of a list may be spread over as many lines as you like, and the last may be followed by a comma. A line that begins with a dot continues the line before it, which is how a long chain of method calls is written. And a line that begins with a binary operator, or with `else`, continues the line before it, with one exception: a line that begins with `-` is a new statement, because `-` may also be the beginning of an expression. A line never continues because of the way it *ends*.

```koo
fn main() {
    names := ["grace", "ada", "edsger"]
    print(
        names.len,
    )
    shouted := names
        .sorted()
        .map(name -> name.to_upper())
    print(shouted)
    all_short := names[0].len < 8
        and names[1].len < 8
        and names[2].len < 8
    print(all_short)
}
```

```text
3
["ADA", "EDSGER", "GRACE"]
true
```
