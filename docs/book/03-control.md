# Chapter 3: Control Flow

The control-flow statements of a language specify the order in which computations are performed. We have already met the most common control-flow constructions in earlier examples; here we will complete the set, and be more precise about the ones discussed before.

## 3.1 Statements and Blocks

A statement is a declaration, an assignment, a call, or one of the control-flow statements of this chapter. A statement occupies a line, and there is nothing to end it with but the end of the line.

An expression is not by itself a statement, unless it is a call, or ends in one of the error-handling forms of Chapter 7. The line `x + 1`, alone, computes a value and does nothing with it, and the compiler says so; it does not ignore the line.

Braces `{` and `}` group declarations and statements into a *block*, and every body of a function, of a loop and of a conditional is a block. A name declared in a block ends with it.

## 3.2 If-Else

The `if-else` statement is used to express decisions. Formally, the syntax is

    if condition {
        statements
    } else {
        statements
    }

where the `else` part is optional. The condition is evaluated; if it is true, the first block is executed. If it is false, and if there is an `else` part, the second block is executed instead.

The condition is an expression of type `bool`, and nothing else will do: a number that is not zero does not count as true. In place of an expression there may be a binding condition, `if name := expression`, where the expression has an optional type. Then the first block is executed if the value is there, with the name bound to it, and the `else` part if it is absent.

```koo
fn main() {
    for text in ["12", "", "seven"] {
        if text == "" {
            print("nothing was typed")
        } else {
            print("typed: {text}")
        }
    }

    words := ["now", "is", "the", "time"]
    if last := words.last() {
        print("the last word is {last}")
    } else {
        print("there are no words")
    }
}
```

```text
typed: 12
nothing was typed
typed: seven
the last word is time
```

`words.last()` is the last element of a list, as an optional, since a list may be empty.

Because the braces are required, there is never any doubt about which `if` an `else` belongs to. The ambiguity that C resolves by a rule, and that programmers resolve by getting it wrong, cannot be written.

## 3.3 Else-If

The construction

    if condition {
        statements
    } else if condition {
        statements
    } else if condition {
        statements
    } else {
        statements
    }

occurs so often that it is worth a brief separate discussion. This sequence of `if` statements is the most general way of writing a multi-way decision. The conditions are evaluated in order; if any is true, the block associated with it is executed, and this terminates the whole chain. The last `else` part handles the "none of the above" or default case where none of the other conditions is satisfied. It can be omitted if there is nothing to do in that case.

To illustrate a three-way decision, here is a binary search function that decides if a particular value `x` occurs in the sorted list `v`. The elements of `v` must be in increasing order. The function returns the position (a number between 0 and `v.len - 1`) if `x` occurs in `v`, and `none` if not.

```koo
// Find x in v, where v[0] <= v[1] <= ... <= v[v.len - 1].
fn bin_search(x: int, v: [int]) -> int? {
    var low := 0
    var high := v.len - 1
    while low <= high {
        mid := (low + high) / 2
        if x < v[mid] {
            high = mid - 1
        } else if x > v[mid] {
            low = mid + 1
        } else {
            return mid      // found a match
        }
    }
    return none             // no match
}

fn main() {
    primes := [2, 3, 5, 7, 11, 13, 17, 19]
    print(bin_search(7, primes))
    print(bin_search(8, primes))
    if at := bin_search(17, primes) {
        print("17 is at position {at}")
    }
}
```

```text
3
none
17 is at position 6
```

The fundamental decision is whether `x` is less than, greater than, or equal to the middle element `v[mid]` at each step; this is a natural for `else if`.

The result type `int?` deserves a word. The C version of this function returns −1 for "not found", a value that is an `int` like any other, and that a careless caller will use as a subscript. Here "not found" is not an integer at all. A caller who wants the position must ask whether there is one, as `main` does, and the compiler will accept nothing less. Inside the function, `return mid` returns an `int` where an `int?` is wanted, which is allowed: a value is always acceptable where a value-or-nothing is expected.

**Exercise 3-1.** Our binary search makes two tests inside the loop, when one would suffice (at the price of more tests outside). Write a version with only one test inside the loop.

## 3.4 Match

The `match` statement is a multi-way decision that tests a value against a number of *patterns*, and branches accordingly.

    match expression {
        pattern => statement
        pattern => statement
        _ => statement
    }

Each line of the body is an *arm*: a pattern, the sign `=>`, and what to do, which is one simple statement or a block in braces. The arms are tried in order, and the first whose pattern matches the value is executed. Then the `match` is finished: control does not fall through into the next arm, as it does in the `switch` of C, so there is nothing to forget at the end of an arm.

In Chapter 1 we wrote a program to count the occurrences of each digit, white space, and all other characters, using a sequence of `if ... else if ... else`. Here is the same program with a `match`:

```koo
import std/io

// Count digits, white space, and other characters.
fn main() {
    var digits := List<int>.with_size(10)
    var white := 0
    var other := 0
    while line := io.read_line() {
        white += 1
        for c in line.chars() {
            match c {
                " " | "\t" => white += 1
                d if d >= "0" and d <= "9" => digits[int.parse(d) else 0] += 1
                _ => other += 1
            }
        }
    }
    print("digits = {digits}, white space = {white}, other = {other}")
}
```

```input
On 4 July 1976, at 12:00,
200 years had passed.
```

```text
digits = [4, 2, 2, 0, 1, 0, 1, 1, 0, 1], white space = 10, other = 26
```

The three arms show most of what a pattern can be.

- A constant, such as `" "`, matches a value equal to it. Several patterns separated by `|` make a pattern that matches if any of them does.
- A name, such as `d`, matches any value, and is bound to it in the arm.
- After a pattern, `if` and a condition make a *guard*: the arm is chosen only if the pattern matches and the condition is true as well.
- The pattern `_` matches anything, and binds nothing.

For integers, a range is a pattern too: `1..=9` matches the integers from 1 to 9, and `1..9` those from 1 to 8.

A `match` must be *exhaustive*: some arm must match whatever the value is. If the compiler cannot see that every possible value is covered, it rejects the program. For a number or a string, that means there must be an arm that takes whatever is left:

```koo
fn main() {
    month := 2
    match month {
        1 => print("January")
        2 => print("February")
    }
}
```

```error
error: this `match` does not cover the remaining values: add a `_` arm
 --> main.koo:3:11
  |
3 |     match month {
  |           ^^^^^ this is an `int`
  |
  = note: a `match` covers every case (Section 6.6)

1 error
```

This is a restriction that pays its way as programs grow. In Chapter 6 we will meet types whose values are one of a fixed set of alternatives. When an alternative is added to such a type, every `match` on it that has not been brought up to date stops compiling, and so the compiler finds for you all the places that have to change.

Like `if`, a `match` may be used for its value. Then each arm is a single expression, and all the arms have the same type:

```koo
fn describe(n: int) -> string {
    return match n {
        0 => "zero"
        1..=9 => "small"
        10 | 100 | 1000 => "round"
        _ if n < 0 => "negative"
        _ => "large"
    }
}

fn main() {
    for n in [0, 7, 100, -3, 12345] {
        print("{n} is {describe(n)}")
    }
}
```

```text
0 is zero
7 is small
100 is round
-3 is negative
12345 is large
```

A tuple of patterns matches a tuple of values, element by element, which makes a `match` a good way to write a decision that depends on two things at once:

```koo
// The classic exercise: Fizz for multiples of 3, Buzz for multiples of 5.
fn main() {
    for n in 1..=15 {
        match (n % 3, n % 5) {
            (0, 0) => print("FizzBuzz")
            (0, _) => print("Fizz")
            (_, 0) => print("Buzz")
            _ => print(n)
        }
    }
}
```

```text
1
2
Fizz
4
Buzz
Fizz
7
8
Fizz
Buzz
11
Fizz
13
14
FizzBuzz
```

**Exercise 3-2.** Write a function `escape(s)` that converts characters like newline and tab into visible escape sequences like `\n` and `\t` as it copies the string `s`. Use a `match`. Write a function for the other direction as well, converting escape sequences into the real characters.

## 3.5 Loops: While and For

We have already encountered the `while` and `for` loops. In

    while condition {
        statements
    }

the condition is evaluated. If it is true, the block is executed and the condition is re-evaluated. This cycle continues until the condition becomes false, at which point execution resumes after the block. The condition may be a binding condition, in which case the loop goes on for as long as there is a value.

The `for` statement

    for name in sequence {
        statements
    }

executes the block once for each element of a sequence, with the name bound to the element. The sequence may be

- a range of integers, `low..high` or `low..=high`;
- a list, whose elements are taken in order;
- a map, whose entries are taken in the order they were first added, each as a pair `(key, value)`;
- a set, whose members are taken in the order they were first added.

The name may be a pattern that takes a tuple apart, as in `for (key, value) in stock`. If the elements are not needed, the pattern `_` will do in place of a name.

```koo
fn main() {
    for i in 0..3 {
        print("i = {i}")
    }

    primes := [2, 3, 5]
    for p in primes {
        print("{p} squared is {p * p}")
    }

    for (i, c) in "abc".chars().enumerate() {
        print("character {i} is {c}")
    }

    stock := ["pens": 120, "ink": 8]
    for (item, count) in stock {
        print("{count} {item}")
    }

    for _ in 0..2 {
        print("again")
    }
}
```

```text
i = 0
i = 1
i = 2
2 squared is 4
3 squared is 9
5 squared is 25
character 0 is a
character 1 is b
character 2 is c
120 pens
8 ink
again
again
```

`enumerate` is a method of lists that pairs each element with its position.

The sequence of a `for` is evaluated once, before the first time round. A range such as `0..v.len` does not grow if `v` does. And a loop over a list is a loop over the *value* that the list had when the loop began: the body may change the list, and the loop neither sees the changes nor is upset by them.

```koo
fn main() {
    var numbers := [1, 2, 3]
    for n in numbers {
        numbers.push(n * 10)
    }
    print(numbers)
}
```

```text
[1, 2, 3, 10, 20, 30]
```

In many languages this program would loop for ever, or fail in some worse way. Here it could not be simpler: the loop runs over 1, 2, 3.

Whether to use `while` or `for` is largely a matter of personal preference. A `for` is preferable whenever the loop runs over something that exists before the loop begins, because it says so in one line and cannot get the counting wrong. A range always counts up by one. For a loop that counts down, or steps by some other amount, or whose bound changes as it goes, use `while`.

Our first larger example is a Shell sort for sorting a list of integers. The basic idea of this sorting algorithm, which was invented in 1959 by D. L. Shell, is that in early stages, far-apart elements are compared, rather than adjacent ones as in simpler interchange sorts. This tends to eliminate large amounts of disorder quickly, so later stages have less work to do. The interval between compared elements is gradually decreased to one, at which point the sort effectively becomes an adjacent interchange method.

```koo
// Sort v into increasing order.
fn shell_sort(v: var [int]) {
    var gap := v.len / 2
    while gap > 0 {
        for i in gap..v.len {
            var j := i - gap
            while j >= 0 and v[j] > v[j + gap] {
                v.swap(j, j + gap)
                j -= gap
            }
        }
        gap /= 2
    }
}

fn main() {
    var v := [31, 41, 59, 26, 53, 58, 97, 93, 23, 84]
    shell_sort(var v)
    print(v)
}
```

```text
[23, 26, 31, 41, 53, 58, 59, 84, 93, 97]
```

There are three nested loops. The outermost controls the gap between compared elements, shrinking it from `v.len / 2` by a factor of two each pass until it becomes zero. The middle loop steps along the elements. The innermost loop compares each pair of elements that is separated by `gap` and reverses any that are out of order; `v.swap(i, j)` exchanges two elements of a list. Since `gap` is eventually reduced to one, all elements are eventually ordered correctly. Notice how the outer and inner loops, which count down, are `while` loops, and the middle one, which runs over a range, is a `for`.

(You will seldom write a sort. Lists have a method `sort`, and Chapter 5 shows how to sort by any ordering you care to give.)

Reversing a string, which in C is a loop with two indices, is three method calls: the characters, reversed, joined together again.

```koo
// Reverse the string s.
fn reverse(s: string) -> string {
    return s.chars().reversed().join("")
}

fn main() {
    print(reverse("Koo, a language"))
}
```

```text
egaugnal a ,ooK
```

**Exercise 3-3.** Write a function `expand(s)` that expands shorthand notations like `a-z` in the string `s` into the equivalent complete list `abc...xyz`. Allow for letters of either case and digits, and be prepared to handle cases like `a-b-c` and `a-z0-9` and `-a-z`. Arrange that a leading or trailing `-` is taken literally.

## 3.6 Loops That Test at the Bottom

The `while` and `for` loops test their condition at the top. C has a third loop, the `do-while`, which tests at the bottom, after making each pass through the loop body. Koo has no third loop. When the body must be executed at least once, write a loop that does not end by itself, and leave it from the bottom:

    while true {
        statements
        if condition {
            break
        }
    }

Experience shows that this is needed much less often than `while` and `for`. Nonetheless, from time to time it is valuable, as in the following function `itoa`, which converts a number to a string of digits (the inverse of `atoi`). The job is slightly more complicated than might be thought at first, because the easy methods of generating the digits generate them in the wrong order. We have chosen to put each digit in front of the ones generated before it.

```koo
// Convert n to characters.
fn itoa(n: int) -> string {
    var digits := ""
    var left := n.abs()
    while true {                            // generate digits in reverse order
        digits = "{left % 10}" + digits     // get the next digit
        left /= 10                          // delete it
        if left == 0 {
            break
        }
    }
    return if n < 0 { "-" + digits } else { digits }
}

fn main() {
    print(itoa(1976))
    print(itoa(-42))
    print(itoa(0))
}
```

```text
1976
-42
0
```

The test at the bottom is necessary, or at least convenient, since at least one character must be produced even if `n` is zero.

Of course `itoa` is only an illustration: `n.to_string()` does this job, and so does `"{n}"`.

**Exercise 3-4.** Our version of `itoa` does not handle the largest negative number, that is, the value of `n` equal to −2<sup>63</sup>. Explain why not, and find out what happens. Modify it to print that value correctly.

**Exercise 3-5.** Write the function `itob(n, b)` that converts the integer `n` into a base `b` character representation. In particular, `itob(n, 16)` formats `n` as a hexadecimal integer.

**Exercise 3-6.** Write a version of `itoa` that accepts two arguments instead of one. The second argument is a minimum field width; the converted number must be padded with blanks on the left if necessary to make it wide enough.

## 3.7 Break and Continue

It is sometimes convenient to be able to exit from a loop other than by testing at the top or bottom. The `break` statement provides an early exit from `for` and `while`. A `break` causes the innermost enclosing loop to be exited immediately.

The following function, `trim_right`, removes trailing blanks and tabs from the end of a string, using a `break` to exit from a loop when the rightmost character that is not a blank or a tab is found.

```koo
// Remove trailing blanks and tabs.
fn trim_right(s: string) -> string {
    chars := s.chars()
    var n := chars.len
    while n > 0 {
        if chars[n - 1] != " " and chars[n - 1] != "\t" {
            break
        }
        n -= 1
    }
    return chars.take(n).join("")
}

fn main() {
    print("[{trim_right("fields \t ")}]")
    print("[{trim_right("   ")}]")
}
```

```text
[fields]
[]
```

`chars.take(n)` is the first `n` elements of the list. The loop scans backwards, looking for the first character that is not a blank or tab. It is broken when one is found, or when `n` becomes zero (that is, when the entire string has been scanned). You should verify that this is correct behavior even when the string is empty or contains only white space characters.

The `continue` statement is related to `break`, but less often used; it causes the next iteration of the enclosing `for` or `while` loop to begin. In the `while`, this means that the test part is executed immediately; in the `for`, the loop moves on to the next element. As an example, this fragment of a program sums only the positive elements of a list; negative values are skipped.

```koo
fn main() {
    numbers := [4, -7, 12, -1, 9]
    var sum := 0
    for n in numbers {
        if n < 0 {      // skip negative elements
            continue
        }
        sum += n
    }
    print(sum)
}
```

```text
25
```

The `continue` statement is often used when the part of the loop that follows is complicated, so that reversing a test and indenting another level would nest the program too deeply.

**Exercise 3-7.** Write a loop that prints the elements of a list of integers, ten per line, with each column lined up. There should be a newline after the last element, whether or not the last line is full.

## 3.8 Leaving Several Loops

C provides the infinitely-abusable `goto` statement, and labels to branch to. Koo has neither. The one place where C programmers still defend `goto` is the abandonment of processing in some deeply nested structure, such as breaking out of two or more loops at once, where `break` cannot be used directly since it only exits from the innermost loop.

In Koo this job is done by a function. A `return` leaves the function, however many loops it is in the middle of, and the function's result says what was found. Here is the determination of whether two lists have an element in common:

```koo
// The first element of a that is also in b.
fn common(a: [int], b: [int]) -> int? {
    for x in a {
        for y in b {
            if x == y {
                return x
            }
        }
    }
    return none
}

fn main() {
    if x := common([3, 8, 15, 23], [4, 15, 16, 23]) {
        print("both have {x}")
    } else {
        print("nothing in common")
    }
}
```

```text
both have 15
```

The code that in C would follow the label `found:` follows the call, and the two cases, found and not found, cannot be confused, since they are the two cases of an optional.
