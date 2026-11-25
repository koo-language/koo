# Chapter 1: A Tutorial Introduction

Let us begin with a quick introduction to Koo. Our aim is to show the essential elements of the language in real programs, but without getting bogged down in details, rules and exceptions. At this point we are not trying to be complete or even precise, save that the examples are meant to be correct. We want to get you as quickly as possible to the point where you can write useful programs, and to do that we have to concentrate on the basics: variables and constants, arithmetic, control flow, functions, and the rudiments of input and output. We leave out of this chapter much that matters for larger programs: structures, errors, actors, and most of the standard library.

This approach has its drawbacks. No single feature is told in full here, and the tutorial, by being brief, may also mislead. Later chapters repeat some of this chapter in order to do it properly. We hope the repetition helps more than it annoys.

## 1.1 Getting Started

The only way to learn a new programming language is by writing programs in it. The first program to write is the same for all languages: print the words `hello, world`.

In Koo, the program is

```koo
fn main() {
    print("hello, world")
}
```

and it prints

```text
hello, world
```

How to run it depends on nothing but the compiler. Put the program in a file whose name ends in `.koo`, such as `hello.koo`, and say

```sh
koo run hello.koo
```

which builds the program and runs it. To keep the executable instead, say `koo build hello.koo`, which leaves a program named `hello` beside you.

Now for some explanations about the program itself. A Koo program consists of *functions* and other declarations. A function contains *statements* that say what is to be done. Here the function is named `main`. Normally you are at liberty to give functions whatever names you like, but `main` is special: your program begins executing at the beginning of `main`, so every program must have a `main` somewhere.

`fn` introduces a function. After its name comes the list of its *parameters* in parentheses, which is empty here, and then its statements, enclosed in braces. `main` contains one statement,

    print("hello, world")

A function is *called* by naming it, followed by a parenthesized list of *arguments*. `print` is a function that writes its argument and then ends the line. A sequence of characters in double quotes, like `"hello, world"`, is a *string*.

There are no semicolons. A statement ends where its line ends, and two statements cannot share a line. The opening brace of a block goes on the same line as what the block belongs to.

`print` always ends the line. To write text without ending the line, use `write` from the package `std/io`, which a program asks for with `import`:

```koo
import std/io

fn main() {
    io.write("hello, ")
    io.write("world")
    io.write("\n")
}
```

```text
hello, world
```

A function of a package is called by the package's last name, a dot, and its own name. The sequence `\n` in a string stands for the *newline* character, which ends a line. Among the others are `\t` for tab, `\"` for the double quote and `\\` for the backslash itself.

**Exercise 1-1.** Run the `hello, world` program on your system. Experiment with leaving out parts of the program, to see what the compiler says.

**Exercise 1-2.** Experiment to find out what happens when the string given to `print` contains `\c`, where *c* is some character not listed above.

## 1.2 Variables and Arithmetic

The next program uses the formula °C = (5/9)(°F − 32) to print the following table of Fahrenheit temperatures and their Celsius equivalents:

```koo
// Print a Fahrenheit-Celsius table for fahr = 0, 20, ..., 300.
fn main() {
    lower := 0      // the lowest temperature of the table
    upper := 300    // the highest
    step := 20      // the step between two lines

    var fahr := lower
    while fahr <= upper {
        celsius := 5 * (fahr - 32) / 9
        print("{fahr}\t{celsius}")
        fahr += step
    }
}
```

```text
0	-17
20	-6
40	4
60	15
80	26
100	37
120	48
140	60
160	71
180	82
200	93
220	104
240	115
260	126
280	137
300	148
```

The program is still one function, `main`. It is longer than the one that printed `hello, world`, but not complicated. It introduces several new ideas: comments, variables, arithmetic, loops, and formatted output.

The first line,

    // Print a Fahrenheit-Celsius table for fahr = 0, 20, ..., 300.

is a *comment*. Everything from `//` to the end of its line is ignored by the compiler, and may be used freely to make a program easier to understand.

A name is given to a value with `:=`. The statement

    lower := 0

declares `lower` and gives it the value 0. A name declared so cannot be given another value afterwards. A name that is to change is declared with `var`:

    var fahr := lower

and is given a new value with `=`, or with one of `+=`, `-=`, `*=` and `/=`, which combine an operation with the assignment: `fahr += step` adds `step` to `fahr`.

Every value has a *type*. We did not write any, because the compiler works out the type of a name from the value it is given: `lower`, `upper`, `step` and `fahr` are of type `int`, an integer of 64 bits. Koo has several other basic types, including

| | |
| --- | --- |
| `float` | a floating-point number of 64 bits |
| `bool` | `true` or `false` |
| `string` | a sequence of characters |
| `byte` | an integer from 0 to 255 |

There are also *lists*, *maps*, *tuples*, *structures* and *enumerations* of these, all of which we will meet in due course.

Each line of the table is computed the same way, so we use a loop that repeats once per line; this is the purpose of the `while`:

    while fahr <= upper {
        ...
    }

The `while` works as follows. The condition is tested. If it is true (`fahr` is less than or equal to `upper`), the body of the loop, the statements in braces, is executed. Then the condition is tested again, and if it is true the body is executed again. When the test becomes false (`fahr` exceeds `upper`) the loop ends, and execution continues at the statement that follows the loop. There are no further statements in this program, so it ends.

The braces of a body are required, even when the body is a single statement, and the condition needs no parentheses. We indent the statements of a body by four spaces, so that you can see at a glance what is inside what. The compiler does not care how a program looks, but people do.

Most of the work is done in the body of the loop. The Celsius temperature is computed and given the name `celsius` by

    celsius := 5 * (fahr - 32) / 9

`celsius` is declared anew each time round the loop, so it needs no `var`. The reason for multiplying by 5 and then dividing by 9, and not just multiplying by 5/9, is that in Koo, as in many other languages, integer division *truncates*: any fractional part is discarded. Since 5 and 9 are integers, 5/9 would be truncated to zero, and so all the Celsius temperatures would be reported as zero.

This example also shows a bit more of how `print` works. Inside a string, text in braces is an *expression*: it is evaluated, and its value takes its place. So

    print("{fahr}\t{celsius}")

prints the value of `fahr`, a tab, and the value of `celsius`. To put an opening brace in a string, write `\{`.

There are a couple of problems with the temperature program. The simpler one is that the output is not very pretty, because the numbers are not right-justified. That is easy to fix: after the expression, a colon and a number give the width to print it in. And the more serious problem is that because we have used integer arithmetic, the Celsius temperatures are not very accurate; for instance, 0°F is actually about −17.8°C, not −17. To get more accurate answers, we should use floating-point arithmetic. Here is a second version:

```koo
// Print a Fahrenheit-Celsius table for fahr = 0, 20, ..., 300, in floating point.
fn main() {
    lower := 0.0
    upper := 300.0
    step := 20.0

    var fahr := lower
    while fahr <= upper {
        celsius := (5.0 / 9.0) * (fahr - 32.0)
        print("{fahr:3.0} {celsius:6.1}")
        fahr += step
    }
}
```

```text
  0  -17.8
 20   -6.7
 40    4.4
 60   15.6
 80   26.7
100   37.8
120   48.9
140   60.0
160   71.1
180   82.2
200   93.3
220  104.4
240  115.6
260  126.7
280  137.8
300  148.9
```

This is much the same as before, except that `fahr` and `celsius` are of type `float`, because the names were given floating-point values: a number written with a decimal point is a `float`. We were unable to use 5/9 in the previous version because integer division would truncate it to zero. A decimal point in a constant says that it is floating point, however, so `5.0 / 9.0` is not truncated, because it is the ratio of two floating-point values.

The format `{fahr:3.0}` says that `fahr` is to be printed at least three characters wide, with no digits after the decimal point. `{celsius:6.1}` says that `celsius` is to be printed at least six characters wide, with one digit after the decimal point. A number is pushed to the right of its width. The width and the precision may each be left out: `{celsius:.2}` asks for two digits after the decimal point and whatever width that takes.

An `int` and a `float` do not mix. If an arithmetic operator has one of each, Koo does not quietly convert the integer, as some languages do: it refuses the program. Suppose we had kept `fahr` an integer:

```koo
fn main() {
    fahr := 100
    celsius := (5.0 / 9.0) * (fahr - 32)
    print(celsius)
}
```

The compiler says

```error
error: `*` cannot be applied to `float` and `int`
 --> main.koo:3:30
  |
3 |     celsius := (5.0 / 9.0) * (fahr - 32)
  |                -----------   ^^^^^^^^^^^ this is an `int`
  |                |
  |                this is a `float`
  |
  = note: an `int` never becomes a `float` by itself (Section 4.2)
help: convert the `int` to a `float`
  |
3 |     celsius := (5.0 / 9.0) * float.from(fahr - 32)
  |                              ++++++++++

1 error
```

and it has told us the cure: the function `float.from` makes a `float` of an `int`, so the line should read `celsius := (5.0 / 9.0) * float.from(fahr - 32)`. (The section that a message cites is a section of the language's specification.) You will find that the compiler's messages are worth reading. They point at the place, say what is wrong in the terms of your program, and, where only one thing can have been meant, show the corrected line.

**Exercise 1-3.** Modify the temperature conversion program to print a heading above the table.

**Exercise 1-4.** Write a program to print the corresponding Celsius to Fahrenheit table.

## 1.3 The For Statement

There are plenty of different ways to write a program for a particular task. Let us try a variation on the temperature converter.

```koo
// Print a Fahrenheit-Celsius table.
fn main() {
    for i in 0..=15 {
        fahr := 20 * i
        print("{fahr:3} {(5.0 / 9.0) * float.from(fahr - 32):6.1}")
    }
}
```

```text
  0  -17.8
 20   -6.7
 40    4.4
 60   15.6
 80   26.7
100   37.8
120   48.9
140   60.0
160   71.1
180   82.2
200   93.3
220  104.4
240  115.6
260  126.7
280  137.8
300  148.9
```

This produces the same answers, but it certainly looks different. One major change is the elimination of most of the variables, and of `var` altogether. The other is that the expression that computes the Celsius temperature now appears inside the string, where the name `celsius` used to be. This is an instance of a general rule: where the value of a name may be used, so may a more complicated expression of the same type.

The `for` statement is a loop over the values of a *range*. The range `0..=15` is the integers from 0 to 15, both included, and `0..15` is the same without the 15. The name after `for` is declared by the loop: it takes each value of the range in turn, and the body is executed once for each. It cannot be assigned to in the body.

The choice between `while` and `for` is arbitrary, based on which seems clearer. The `for` is usually right when the loop steps through something that is known beforehand, and it is more compact than `while`, since it keeps the counting in one place. We will see that a `for` steps through the elements of a list, and of other things, just as it steps through a range.

**Exercise 1-5.** Modify the temperature conversion program to print the table in reverse order, that is, from 300 degrees to 0.

## 1.4 Constants

A final observation before we leave temperature conversion forever. It is bad practice to bury "magic numbers" like 300 and 20 in a program; they convey little information to someone who might have to read the program later, and they are hard to change in a systematic way. One way to deal with magic numbers is to give them meaningful names. A name declared outside every function is a *constant* for the whole program:

```koo
lower := 0      // the lowest temperature of the table
upper := 300    // the highest
step := 20      // the step between two lines

// Print a Fahrenheit-Celsius table.
fn main() {
    var fahr := lower
    while fahr <= upper {
        print("{fahr:3} {(5.0 / 9.0) * float.from(fahr - 32):6.1}")
        fahr += step
    }
}
```

```text
  0  -17.8
 20   -6.7
 40    4.4
 60   15.6
 80   26.7
100   37.8
120   48.9
140   60.0
160   71.1
180   82.2
200   93.3
220  104.4
240  115.6
260  126.7
280  137.8
300  148.9
```

The value of a constant is written out: a number, a string, or `true` or `false`. A constant cannot be `var`. Koo has no variables outside functions at all; Section 1.10 says why, and what is used in their place.

## 1.5 Lines of Input

We are going to consider a family of related programs for processing text. You will find that many programs are just expanded versions of the prototypes that we discuss here.

The model of input and output is simple. Text input or output, regardless of where it originates or where it goes to, is dealt with as a stream of *lines*. The package `std/io` provides `read_line`, which reads the next line of the *standard input* each time it is called, and returns it without the newline that ended it. `print`, as we know, writes a line to the *standard output*. Unless you arrange otherwise, the standard input is the keyboard and the standard output the screen, but either may be a file or another program.

### 1.5.1 Copying Input to Output

Given `read_line` and `print`, you can write a surprising amount of useful code without knowing anything more about input and output. The simplest example is a program that copies its input to its output one line at a time:

```koo
import std/io

// Copy input to output.
fn main() {
    while line := io.read_line() {
        print(line)
    }
}
```

Given the input

```input
now is the time
for all good people
```

it prints

```text
now is the time
for all good people
```

The problem is to tell the end of the input from a line of it. Koo's answer is that `read_line` does not return a string. It returns a *string that may be absent*, which is written `string?`, and at the end of the input the value it returns is absent. Such a value is called an *optional*. The compiler will not let a `string?` be used where a `string` is wanted, because there might be nothing there: one must first find out.

The condition

    line := io.read_line()

does exactly that. It calls `read_line`; if there is a line, it gives it the name `line`, which in the body of the loop is an ordinary string, and the body is executed. If there is none, the loop ends. A condition of this form, called a *binding condition*, may be used in an `if` as well as in a `while`.

**Exercise 1-6.** Write a program that copies its input to its output with the number of each line before it.

### 1.5.2 Counting Lines and Characters

The next program counts lines and characters; it is a small elaboration of the copy program.

```koo
import std/io

// Count the lines and the characters of the input.
fn main() {
    var lines := 0
    var chars := 0
    while line := io.read_line() {
        lines += 1
        chars += line.count() + 1
    }
    print("{lines} {chars}")
}
```

```input
now is the time
for all good people
```

```text
2 36
```

`line.count()` is the number of characters in `line`. A function that belongs to a value is called a *method*, and is called by writing the value, a dot, and the name of the method. Strings have about twenty methods; the commonest appear in this chapter, and Appendix B lists them all. We add one for each line, for the newline that `read_line` removed.

A string also has a *length*, written `line.len` with no parentheses, which is the number of bytes it occupies. The two differ whenever the text is not plain English: a string in Koo is text in the encoding called UTF-8, in which the letter `a` is one byte and the letter `é` is two.

```koo
fn main() {
    word := "café"
    print("{word.count()} characters, {word.len} bytes")
}
```

```text
4 characters, 5 bytes
```

**Exercise 1-7.** Write a program to count blanks and tabs in its input.

**Exercise 1-8.** Write a program to copy its input to its output, leaving out lines that are empty.

### 1.5.3 Counting Words

The fourth in our series of useful programs counts lines, words, and characters, with the loose definition that a word is any sequence of characters that does not contain a blank or a tab. This is a bare-bones version of the UNIX program `wc`.

```koo
import std/io

// Count the lines, words and characters of the input.
fn main() {
    var lines := 0
    var words := 0
    var chars := 0
    while line := io.read_line() {
        lines += 1
        chars += line.count() + 1
        var inside := false
        for c in line.chars() {
            if c == " " or c == "\t" {
                inside = false
            } else if not inside {
                inside = true
                words += 1
            }
        }
    }
    print("{lines} {words} {chars}")
}
```

```input
now is the time
for all good people
```

```text
2 8 36
```

`line.chars()` gives the characters of the line, each as a string of its own, and the `for` loop steps through them. Every time the program meets the first character of a word, it counts one more word. The variable `inside` records whether the program is currently in a word or not; at the start of each line it is `false`, since the end of a line ends a word.

The line

    if c == " " or c == "\t" {

says "if `c` is a blank or `c` is a tab". The operator `==` means "is equal to", and applies to strings as it does to numbers. The operator `or` means OR, `and` means AND, and `not` means NOT. Expressions connected by `and` or `or` are evaluated left to right, and evaluation stops as soon as the truth or falsehood of the whole is known: if `c` is a blank, there is no need to test whether it is a tab, so that test is not made.

The example also shows an `else`, which specifies an alternative action if the condition part of an `if` statement is false. The general form is

    if condition {
        statements
    } else {
        statements
    }

One and only one of the two groups of statements is executed. After `else` there may be another `if` in place of a block, as in the word-count program, and so a chain of tests may be written without the statements creeping to the right.

**Exercise 1-9.** How would you test the word count program? What kinds of input are most likely to uncover bugs if there are any?

**Exercise 1-10.** Write a program that prints its input one word per line.

## 1.6 Lists

Let us write a program to count the number of occurrences of each digit, of white space characters (blank, tab, and the end of a line), and of all other characters. This is artificial, but it permits us to illustrate several aspects of Koo in one program.

There are twelve categories of input, so it is convenient to use a list to hold the number of occurrences of each digit, rather than ten individual variables. Here is one version of the program:

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
            if c >= "0" and c <= "9" {
                digits[int.parse(c) else 0] += 1
            } else if c == " " or c == "\t" {
                white += 1
            } else {
                other += 1
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

A *list* is a sequence of values of one type. The type of a list of integers is written `[int]`. The expression

    List<int>.with_size(10)

makes a list of ten integers, each zero. Its elements are numbered from zero, so they are `digits[0]`, `digits[1]`, ..., `digits[9]`. A subscript can be any integer expression. A subscript that is not the number of an element, 10 for this list, or −1, does not read or change some other part of memory, as it may in C: it stops the program with a message that says so and gives the line.

A list can also be written out, as `[31, 28, 31]`, and it grows: `digits.push(0)` would add an eleventh element. `digits.len` is the number of elements.

This particular program relies on the properties of the digits as text. The test

    if c >= "0" and c <= "9" {

determines whether the character in `c` is a digit. Strings are compared character by character, by the codes of their characters, and the codes of the digits are consecutive and in order. If `c` is a digit, its numeric value is

    int.parse(c) else 0

`int.parse` reads an integer from a string. It can fail, since not every string is a number, and Koo will not let the failure go unremarked: `else` gives the value to use in that case. Here it cannot happen, because we have just seen that `c` is a digit, but the compiler does not know that. The operator `else` is the general way of saying "or, failing that": it also supplies a value for an optional that is absent.

`print` writes a list as its elements between brackets. In fact `print` writes a value of any type in a sensible form, and so does `{...}` in a string.

**Exercise 1-11.** Write a program to print a histogram of the lengths of words in its input. It is easy to draw the histogram with the bars horizontal.

**Exercise 1-12.** Write a program to print a histogram of the frequencies of different characters in its input.

## 1.7 Functions

In Koo, a function is equivalent to a subroutine or function in Fortran, or a procedure or function in Pascal. A function provides a convenient way to encapsulate some computation, which can then be used without worrying about its implementation. With properly designed functions, it is possible to ignore *how* a job is done; knowing *what* is done is sufficient. Koo makes the use of functions easy, convenient and efficient; you will often see a short function defined and called only once, just because it clarifies some piece of code.

So far we have used only functions like `print`, `io.read_line` and `int.parse` that have been provided for us; now it is time to write a few of our own. Since Koo's exponentiation is a method, `x.pow(n)`, and not an operator, let us illustrate the mechanics of function definition by writing a function `power(base, n)` to raise an integer `base` to the positive integer power `n`. That is, the value of `power(2, 5)` is 32.

Here is the function `power` and a main program to exercise it, so you can see the whole structure at once.

```koo
// Test the power function.
fn main() {
    for i in 0..10 {
        print("{i} {power(2, i)} {power(-3, i)}")
    }
}

// Raise base to the n-th power; n >= 0.
fn power(base: int, n: int) -> int {
    var p := 1
    for i in 0..n {
        p *= base
    }
    return p
}
```

```text
0 1 1
1 2 -3
2 4 9
3 8 -27
4 16 81
5 32 -243
6 64 729
7 128 -2187
8 256 6561
9 512 -19683
```

A function definition has this form:

    fn name(parameter declarations) -> return-type {
        statements
    }

Function definitions can appear in any order: `main` calls `power` here before the compiler has seen it, and nothing need be declared in advance.

The first line of `power` itself,

    fn power(base: int, n: int) -> int {

declares the names and types of the parameters, and the type of the result that the function returns. The types of parameters and results are always written; it is the types of the names inside a function that the compiler works out. The names used by `power` for its parameters are local to `power`, and are not visible to any other function: other functions can use the same names without conflict. This is also true of the variables `i` and `p`: the `i` in `power` is unrelated to the `i` in `main`.

The value that `power` computes is returned to `main` by the `return` statement. Any expression of the function's return type may follow `return`. A function need not return a value: one that has no `->` in its first line returns nothing, and a `return` with no expression makes it return to its caller at once, as does "falling off the end" by reaching the closing brace. `main` is such a function.

**Exercise 1-13.** Rewrite the temperature conversion program of Section 1.2 to use a function for conversion.

## 1.8 Arguments: Values

One aspect of Koo functions may be unfamiliar to programmers who are used to some other languages. In Koo, a function cannot alter a variable of the function that called it, unless both of them say so in so many words. Whatever is passed to a function, a number or a list of a million numbers, the function is given the *value*.

Koo goes one step further than C in this direction: a parameter is not even a local variable that the function may change. Here is a version of `power` that tries to use its parameter `n` as a counter that runs down to zero, as the version in Kernighan and Ritchie's book does:

```koo
fn power(base: int, n: int) -> int {
    var p := 1
    while n > 0 {
        p *= base
        n -= 1
    }
    return p
}

fn main() {
    print(power(2, 10))
}
```

```error
error: cannot assign to `n`, which is not `var`
 --> main.koo:5:9
  |
1 | fn power(base: int, n: int) -> int {
  |                     ------ the parameter `n` is declared here
...
5 |         n -= 1
  |         ^
  |
  = note: a parameter is lent to the function: it reads the caller's value and cannot change it (Section 5.3)
  = help: to change the caller's value, declare the parameter as `n: var int` and pass `var n`
  = help: to change a copy, make one: `var copy := n`

1 error
```

The compiler offers two ways out, and they mean different things. If the function wants a counter of its own, it makes one:

```koo
fn power(base: int, n: int) -> int {
    var p := 1
    var left := n
    while left > 0 {
        p *= base
        left -= 1
    }
    return p
}

fn main() {
    print(power(2, 10))
}
```

```text
1024
```

When it is the caller's variable that is to be changed, the parameter is declared `var`, and the argument is passed with `var` too:

```koo
// Add one to each element of a list.
fn increment(numbers: var [int]) {
    for i in 0..numbers.len {
        numbers[i] += 1
    }
}

fn main() {
    var numbers := [1, 2, 3]
    before := numbers
    increment(var numbers)
    print(numbers)
    print(before)
}
```

```text
[2, 3, 4]
[1, 2, 3]
```

The `var` at the call is not decoration. It shows the reader of `main`, who may never look at `increment`, that `numbers` may be different afterwards. Without it the program is refused.

Notice `before`. The statement `before := numbers` gave the name `before` to the value that `numbers` had at that moment, and what was later done to `numbers` did not touch it. This is true of every type in Koo: assignment gives a value, never a second name for the same storage. You might suppose that copying a list at every assignment is expensive. It is not, because the compiler does not copy until it must: `before` and `numbers` share their storage up to the moment that `increment` changes one of them. Chapter 5 tells the full story.

## 1.9 Strings

The most common type of list in C is the array of characters. In Koo a string is a type of its own, and it is a value like any other: it can be assigned, compared, passed and returned freely. To illustrate, let us write a program that reads a set of text lines and prints the longest. The outline is simple enough:

    while there is another line
        if it is longer than the longest so far
            save it
    print the longest line

In C this program needs a function to read a line into an array, a function to copy one array to another, and a decision about how long a line may be. Here the outline is nearly the program:

```koo
import std/io

// Print the longest line of the input.
fn main() {
    var longest := ""
    while line := io.read_line() {
        if line.count() > longest.count() {
            longest = line
        }
    }
    if longest.count() > 0 {
        print(longest)
    }
}
```

```input
now is the time
for all good people
to come to the aid
```

```text
for all good people
```

A line may be of any length. `longest = line` saves the line; it does not matter that `line` is a new string each time round the loop, because `longest` has the value, not a reference to something that is about to change.

Strings are put together with `+`, or more often by writing one inside another:

```koo
fn main() {
    first := "Grace"
    last := "Hopper"
    name := first + " " + last
    print(name)
    print("{last}, {first}")
    print(name.to_upper())
    print(name.contains("ace"))
    print(name.split(" "))
}
```

```text
Grace Hopper
Hopper, Grace
GRACE HOPPER
true
["Grace", "Hopper"]
```

**Exercise 1-14.** Write a program to print all input lines that are longer than 80 characters.

**Exercise 1-15.** Write a program to remove trailing blanks and tabs from each line of input, and to delete entirely blank lines. (The method `trim` removes white space from both ends of a string, which is more than is asked.)

**Exercise 1-16.** Write a function `reverse(s)` that returns the string `s` with its characters in the opposite order. Use it to write a program that reverses its input a line at a time.

## 1.10 Scope

The variables in `main`, such as `lines` and `longest`, are private or local to `main`. Because they are declared within `main`, no other function can have direct access to them. The same is true of the variables in other functions. Each local variable comes into existence when its declaration is executed, and disappears when the block it was declared in is left. The body of a function is a block, and so is the body of a loop or of an `if`.

A name may be declared only once in a block:

```koo
fn main() {
    total := 1
    total := 2
    print(total)
}
```

```error
error: `total` is already declared in this scope
 --> main.koo:3:5
  |
2 |     total := 1
  |     ----- `total` is first declared here
3 |     total := 2
  |     ^^^^^ declared a second time
  |
  = help: use another name, or declare the first as `var total` and assign to it with `=`

1 error
```

A block inside may, however, declare a name that a block outside has also declared. Inside, the name means the inner declaration, and the outer one is untouched:

```koo
fn main() {
    n := 1
    if n > 0 {
        n := 2
        print(n)
    }
    print(n)
}
```

```text
2
1
```

In C it is possible to define variables that are *external* to all functions, which any function may read and change by name. Koo has none. The names declared outside functions are constants (Section 1.4), which every function may read and none can change. A function gets what it needs through its parameters, and what it produces goes back through its result, or through a parameter that it and its caller have both marked `var`. When a program outgrows this, when some state must be kept for a long time and used from many places, as the stock of a shop is used by everyone who buys from it, the state is given to an *actor*, which is the subject of Chapter 9.

The reason is the one behind every other choice in this chapter. A variable that any function may change is a variable whose value you cannot know without reading every function. In Koo, what a function can change is written at the place where it is called.

At this point we have covered what might be called the conventional core of Koo. With this handful of building blocks, it is possible to write useful programs of considerable size, and it would probably be a good idea to pause long enough to do so. These exercises suggest programs of somewhat greater complexity than the ones earlier in this chapter.

**Exercise 1-17.** Write a program `detab` that replaces tabs in the input with the proper number of blanks to space to the next tab stop. Assume a fixed set of tab stops, say every *n* columns. Should *n* be a variable or a constant?

**Exercise 1-18.** Write a program `entab` that replaces strings of blanks by the minimum number of tabs and blanks to achieve the same spacing. Use the same tab stops as for `detab`.

**Exercise 1-19.** Write a program to "fold" long input lines into two or more shorter lines after the last non-blank character that occurs before the *n*-th column of input. Make sure your program does something intelligent with very long lines, and if there are no blanks or tabs before the specified column.
