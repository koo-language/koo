# Chapter 4: Functions and Program Structure

Functions break large computing tasks into smaller ones, and enable people to build on what others have done instead of starting over from scratch. Appropriate functions hide details of operation from parts of the program that do not need to know about them, thus clarifying the whole, and easing the pain of making changes.

Koo has been designed to make functions efficient and easy to use; Koo programs generally consist of many small functions rather than a few big ones. A program may reside in one or more source files, and the compiler needs no help to find its way among them: there is nothing to declare before it is used, and no order in which things must be written.

## 4.1 Basics of Functions

To begin, let us design and write a program to print each line of its input that contains a particular "pattern" or string of characters. (This is a special case of the UNIX program `grep`.) For example, searching for the pattern of letters `ould` in the set of lines

    Ah Love! could you and I with Fate conspire
    To grasp this sorry Scheme of Things entire,
    Would not we shatter it to bits -- and then
    Re-mould it nearer to the Heart's Desire!

will produce the output

    Ah Love! could you and I with Fate conspire
    Would not we shatter it to bits -- and then
    Re-mould it nearer to the Heart's Desire!

The job falls neatly into three pieces:

    while there's another line
        if the line contains the pattern
            print it

Although it is certainly possible to put the code for all of this in `main`, a better way is to use the structure to advantage by making each part a separate function. Three small pieces are easier to deal with than one big one, because irrelevant details can be buried in the functions, and the chance of unwanted interactions is minimized. And the pieces may even be useful in other programs.

"While there's another line" is `io.read_line`, a function that we already have, and "print it" is `print`. This means we need only write a function to decide whether the line contains an occurrence of the pattern.

We can solve that problem by writing a function `str_index(s, t)` that returns the position in the string `s` where the string `t` begins, or `none` if `s` does not contain `t`. Positions are counted in characters, from zero. (The library has a method `find` that does much the same, and `contains` when the position is not wanted; we write our own to see how it is done.)

Given this much design, filling in the details of the program is straightforward. Here is the whole thing, so you can see how the pieces fit together. For now, the pattern to be searched for is a constant, which is not the most general of mechanisms; Chapter 8 shows how to take it from the command line.

```koo
import std/io

pattern := "ould"   // the pattern to search for

// Find all lines matching the pattern.
fn main() {
    var found := 0
    while line := io.read_line() {
        if str_index(line, pattern).is_some() {
            print(line)
            found += 1
        }
    }
    print("{found} lines")
}

// The position of t in s, or none if it is not there.
fn str_index(s: string, t: string) -> int? {
    text := s.chars()
    wanted := t.chars()
    for i in 0..text.len {
        var k := 0
        while k < wanted.len and i + k < text.len and text[i + k] == wanted[k] {
            k += 1
        }
        if k > 0 and k == wanted.len {
            return i
        }
    }
    return none
}
```

```input
Ah Love! could you and I with Fate conspire
To grasp this sorry Scheme of Things entire,
Would not we shatter it to bits -- and then
Re-mould it nearer to the Heart's Desire!
```

```text
Ah Love! could you and I with Fate conspire
Would not we shatter it to bits -- and then
Re-mould it nearer to the Heart's Desire!
3 lines
```

`is_some()` is a method of every optional: it is `true` when there is a value. Its opposite is `is_none()`.

Each function definition has the form

    fn name(parameter declarations) -> return-type {
        declarations and statements
    }

Various parts may be absent. A function that takes nothing has an empty list of parameters, and one that returns nothing has no `->` and no return type. A minimal function is

    fn dummy() {
    }

which does nothing and returns nothing. A do-nothing function like this is sometimes useful as a place holder during program development.

A program is just a set of definitions of types, constants and functions. Communication between the functions is by arguments and values returned by the functions. The functions can occur in any order in the source file, and the source program can be split into multiple files, so long as no function is split.

The `return` statement is the mechanism for returning a value from the called function to its caller. Any expression of the function's return type can follow `return`. A function that is declared to return a value must do so on every path: the compiler rejects a function that can reach its closing brace without one.

```koo
fn sign(n: int) -> int {
    if n > 0 {
        return 1
    }
    if n < 0 {
        return -1
    }
}

fn main() {
    print(sign(-5))
}
```

```error
error: `sign` can reach its end without returning a value
 --> main.koo:8:1
  |
1 | fn sign(n: int) -> int {
  |                    --- it returns `int`
...
8 | }
  | ^ the function ends here
  |
  = help: end every path with `return` or `fail`

1 error
```

There is no overloading in Koo. One name names one function, whatever the types of its parameters, so that a reader who sees a call knows which function is called without working out the types of the arguments.

**Exercise 4-1.** Write the function `str_rindex(s, t)`, which returns the position of the *rightmost* occurrence of `t` in `s`, or `none` if there is none.

## 4.2 Parameters and Arguments

A parameter is declared by its name, a colon and its type. It may also have a *default value*, written after an `=`; an argument for a parameter with a default may be left out of a call. And any argument may be passed *by name*, as `name: value`, which says at the call what the value is for and frees the caller from remembering the order:

```koo
fn describe(width: float, height: float = 1.0, unit: string = "m") -> string {
    return "{width} by {height} {unit}"
}

fn main() {
    print(describe(2.0))
    print(describe(2.0, 3.0))
    print(describe(2.0, unit: "ft"))
    print(describe(height: 4.0, width: 2.5))
}
```

```text
2.0 by 1.0 m
2.0 by 3.0 m
2.0 by 1.0 ft
2.5 by 4.0 m
```

A default value is a constant. Naming an argument is particularly worth while when a function has several parameters of the same type, or when the argument is a bare `true` or `0` that would otherwise say nothing to the reader.

We saw in Chapter 1 that a function is given the values of its arguments and cannot change them. To be exact, a parameter is one of three kinds.

A plain parameter, `numbers: [int]`, is *lent* to the function. The function may read it, and nothing more. This is the kind to use unless there is a reason not to. No matter how large the value is, nothing is copied to pass it.

A `var` parameter, `numbers: var [int]`, is lent to be changed. The argument must be a variable declared with `var`, or a part of one, and is written with `var` at the call. What the function does to the parameter, it does to the caller's variable.

A `take` parameter, `numbers: take [int]`, is *given* to the function to keep: to store in a structure, to return, to hold on to after the call is over. It is no more changeable than a plain parameter, and the caller's variable is not affected. A function may keep a plain parameter as well, so `take` is seldom necessary: it tells the caller, and the compiler, that the value is wanted for keeps, which lets a caller who has finished with the value hand it over without ceremony. For one type of value it is required, as Section 4.6 explains: a function that keeps a *function* it was passed must take it.

```koo
fn total(numbers: [int]) -> int {
    var sum := 0
    for n in numbers {
        sum += n
    }
    return sum
}

fn double_all(numbers: var [int]) {
    for i in 0..numbers.len {
        numbers[i] *= 2
    }
}

fn with_zero(numbers: take [int]) -> [int] {
    var mine := numbers
    mine.push(0)
    return mine
}

fn main() {
    var numbers := [1, 2, 3]
    print(total(numbers))
    double_all(var numbers)
    print(numbers)
    print(with_zero(numbers))
    print(numbers)
}
```

```text
6
[2, 4, 6]
[2, 4, 6, 0]
[2, 4, 6]
```

A function returns one value, but the value may be a *tuple*: several values in parentheses, separated by commas. The caller can take a tuple apart in the declaration that receives it:

```koo
// The quotient and the remainder of a division.
fn divide(a: int, b: int) -> (int, int) {
    return (a / b, a % b)
}

fn main() {
    (quotient, remainder) := divide(17, 5)
    print("{quotient} remainder {remainder}")
}
```

```text
3 remainder 2
```

## 4.3 A Calculator Without External Variables

In *The C Programming Language*, the example for this chapter is a calculator, and it is the occasion for introducing external variables: its stack is declared outside any function, so that the functions `push` and `pop` can share it without its being passed to them. Koo has no external variables, and it is instructive to see what becomes of the example.

The calculator uses reverse Polish notation, in which each operator follows its operands; an infix expression like

    (1 - 2) * (4 + 5)

is entered as

    1 2 - 4 5 + *

Parentheses are not needed; the notation is unambiguous as long as we know how many operands each operator expects. The implementation is simple. Each operand is pushed onto a stack; when an operator arrives, the proper number of operands (two for binary operators) is popped, the operator is applied to them, and the result is pushed back onto the stack. In the example above, 1 and 2 are pushed, then replaced by their difference, −1. Next, 4 and 5 are pushed and then replaced by their sum, 9. The product of −1 and 9, which is −9, replaces them on the stack. The value on the top of the stack is popped and printed when the end of the input line is encountered.

The structure of the program is thus a loop that performs the proper operation on each operator and operand as it appears:

    while there is another line
        for each word of the line
            if it is an operator
                pop operands, do the operation, push the result
            else if it is a number
                push it
            else
                error
        pop and print the top of the stack

The stack is a list of `float`. A list already has the two operations of a stack: `push` adds an element at the end, and `pop` removes the last element and returns it, as an optional, because the list may be empty. So there is nothing to write for the stack itself, and the only question is where it lives. It lives in `main`, as a local variable, and `main` lends it, with `var`, to the function that needs to change it.

```koo
import std/io

// Reverse Polish calculator.
fn main() {
    var stack: [float] = []
    while line := io.read_line() {
        for word in line.split(" ") {
            match word {
                "" => {}
                "+" | "-" | "*" | "/" => operate(var stack, word)
                _ => {
                    number := float.parse(word) else {
                        print("error: unknown command {word}")
                        continue
                    }
                    stack.push(number)
                }
            }
        }
        if top := stack.pop() {
            print("\t{top}")
        }
    }
}

// Replace the two numbers on top of the stack by the result of an operation on them.
fn operate(stack: var [float], operator: string) {
    if stack.len < 2 {
        print("error: stack empty")
        return
    }
    right := stack.pop() else 0.0
    left := stack.pop() else 0.0
    if operator == "/" and right == 0.0 {
        print("error: zero divisor")
        return
    }
    stack.push(match operator {
        "+" => left + right
        "-" => left - right
        "*" => left * right
        _ => left / right
    })
}
```

```input
1 2 - 4 5 + *
2 3 4 + *
1 0 /
7 +
3 four +
```

```text
	-9.0
	14.0
error: zero divisor
error: stack empty
	7.0
error: unknown command four
error: stack empty
	3.0
```

`line.split(" ")` is the list of the pieces of the line between blanks. Two blanks in a row have an empty piece between them, which the first arm of the `match` passes over: `{}` is the empty block.

Because `+` and `*` are commutative operators, the order in which the popped operands are combined is irrelevant, but for `-` and `/` the left and right operands must be distinguished, and the right one is the first to come off the stack.

The declaration of `number` shows a form of `else` that we have not used before. The alternative after `else` may be a block that does not produce a value at all, provided that it *leaves*: by `continue`, as here, or by `break` or `return`. So the line reads "`number` is the number that `word` is; if it is not one, complain and go on to the next word." The statements after it are reached only when there is a number.

Now consider what was gained by having no external variables. In the C program, to know what `push` and `pop` do one must know that there is a variable `sp` and an array `val` somewhere in the file, and every function in the file may alter them. Here the first line of `operate` says everything: it changes a stack that it is lent, and nothing else. A second calculator, with a second stack, would need no change to `operate` at all.

**Exercise 4-2.** Given the basic framework, it is straightforward to extend the calculator. Add the modulus (`%`) operator and provisions for negative numbers.

**Exercise 4-3.** Add commands to print the top element of the stack without popping, to duplicate it, and to swap the top two elements. Add a command to clear the stack.

**Exercise 4-4.** Add commands for handling variables. (It is easy to provide twenty-six variables with single-letter names: where should they be kept?) Add a variable for the most recently printed value.

## 4.4 Programs in Several Files

A program need not all be in one file. A *package* is a directory, and every file in it whose name ends in `.koo` is part of one program. A calculator larger than ours might be laid out so:

    calc/
        main.koo        fn main() ...
        stack.koo       fn operate(stack: var [float], operator: string) ...
        words.koo       fn next_word(line: string, at: var int) -> string? ...

The files of a package see each other's declarations as if they were one file. There are no header files, nothing to declare twice, and nothing to keep consistent between a declaration and a definition, since a thing is written once. Any function may call any other, wherever each is.

The exception is `import`. An import belongs to the file in which it is written, so each file imports the standard packages that it uses, and no others: an import that a file does not use is an error.

The commands that take a file take a directory as well:

```sh
koo run calc        # build the package and run its main
koo build calc      # leave an executable named calc
koo test calc       # run its tests (Appendix C)
```

## 4.5 Scope Rules

The scope of a name is the part of the program within which the name can be used.

The scope of a function, a type or a constant declared at the top level of a file is the whole package: every file, from beginning to end. The order of declarations does not matter.

The scope of a parameter is the function it is declared in. The scope of a local name begins at its declaration and lasts to the end of the block in which the declaration appears. As Section 1.10 showed, a block may declare a name that is also declared in a block around it, and the name then means the inner declaration until the inner block ends; but a block may not declare the same name twice.

Functions cannot be declared inside functions. What can be written inside a function is a *lambda*, the subject of the next section, which is a function without a name; given a name by `:=`, it does the job of a local function.

There are no `static` variables in Koo, that is, no variables that belong to one function and keep their values from one call to the next. A function's result depends on its arguments and on nothing else, unless the function reads something outside the program, such as its input. This is a strong statement, and a useful one: a function that is given the same arguments does the same thing, and can be tested by calling it.

## 4.6 Functions as Values

In Koo a function is itself a value. It can be assigned, passed to a function, returned from one, and stored in a list. The type of a function is written like its first line without the names: `fn(int, int) -> int` is the type of a function that takes two integers and returns one, and `fn(string)` that of a function that takes a string and returns nothing.

The name of a function, without parentheses, is the function as a value. More often the value is written on the spot, as a lambda:

    x -> expression
    (a, b) -> expression
    () -> expression

The parameters of a lambda are written before the arrow, with parentheses if there is not exactly one, and its result is the expression after it. The types of the parameters are found from the place where the lambda is used; where there is nothing to find them from, as when a lambda is simply given a name, they are written out, `(n: int) -> n * n`. In place of the expression there may be a block of statements, which uses `return` as any function does.

```koo
fn apply(f: fn(int, int) -> int, a: int, b: int) -> int {
    return f(a, b)
}

fn add(a: int, b: int) -> int {
    return a + b
}

fn main() {
    print(apply(add, 3, 4))
    print(apply((a, b) -> a * b, 3, 4))
    print(apply((a, b) -> {
        if a > b {
            return a
        }
        return b
    }, 3, 4))

    square := (n: int) -> n * n
    print(square(12))
}
```

```text
7
12
4
144
```

Functions that take functions are the stock in trade of the library. A list has methods that apply a function to each of its elements: `map` makes a list of the results, `filter` keeps the elements for which the function returns `true`, `each` calls the function for its effect. Chapter 5 has many examples; a few will do here.

```koo
fn main() {
    numbers := [3, 8, 1, 9, 4]
    print(numbers.map(n -> n * n))
    print(numbers.filter(n -> n % 2 == 1))
    print(numbers.any(n -> n > 8))

    var sum := 0
    numbers.each(n -> { sum += n })
    print(sum)
}
```

```text
[9, 64, 1, 81, 16]
[3, 1, 9]
true
25
```

A lambda may use the variables of the function it is written in, as the last one uses `sum`. What that means depends on what becomes of the lambda, and the rule has two cases.

A lambda that is passed directly to a function, as an argument, works on the variables around it. It runs while the function it is passed to runs, and is gone before that function returns, so it is as much a part of the surrounding code as the body of a loop is. The lambda given to `each` above adds to the `sum` of `main`.

A lambda that is *kept*, that is, given a name, returned from a function, or stored, holds its own copies of the variables it uses, as they were when the lambda was made. It is a value, and like every value it is independent of where it came from. It cannot assign to a variable that it holds a copy of, since the assignment would be to the copy and would be lost.

```koo
// A function that multiplies by a factor.
fn scale_by(factor: int) -> fn(int) -> int {
    return n -> n * factor
}

fn main() {
    triple := scale_by(3)
    print(triple(14))

    var rate := 2
    double := (n: int) -> n * rate
    rate = 100
    print(double(21))
}
```

```text
42
42
```

`scale_by(3)` returns a function that remembers the 3. And `double` goes on doubling after `rate` has been changed, because it has the 2 that `rate` was when it was made.

The distinction matters once more when a function is passed to a function. A lambda written as an argument works, as we said, directly on its caller's variables, and those are gone when its caller returns. So a function that is merely lent a function may call it, or pass it on to be called, and nothing else. A function that wants to keep a function it is passed, by returning it, storing it, or using it in a lambda that is itself kept, must declare the parameter `take`. Then the lambda that is passed is one of the kept kind, with its own copies.

```koo
// The function that applies f twice.
fn twice(f: take fn(int) -> int) -> fn(int) -> int {
    return n -> f(f(n))
}

fn main() {
    add_six := twice(n -> n + 3)
    print(add_six(1))
}
```

```text
7
```

Without the `take`, the compiler rejects `twice`, and explains.

**Exercise 4-5.** Write a function `compose(f, g)` that takes two functions from `int` to `int` and returns the function that applies `g` to the result of `f`.

## 4.7 Recursion

Koo functions may be used recursively; that is, a function may call itself either directly or indirectly. Consider printing a number as a character string. As we mentioned before, the digits are generated in the wrong order: low-order digits are available before high-order digits, but they have to be printed the other way around.

There are two solutions to this problem. One is to store the digits as they are generated, and then print them in the reverse order, as we did with `itoa` in Section 3.6. The alternative is a recursive solution, in which `print_decimal` first calls itself to cope with any leading digits, then prints the trailing digit.

```koo
import std/io

// Print n in decimal.
fn print_decimal(n: int) {
    if n < 0 {
        io.write("-")
        print_decimal(-n)
        return
    }
    if n / 10 != 0 {
        print_decimal(n / 10)
    }
    io.write("{n % 10}")
}

fn main() {
    print_decimal(123)
    io.write("\n")
    print_decimal(-4096)
    io.write("\n")
}
```

```text
123
-4096
```

When a function calls itself recursively, each invocation gets a fresh set of all its local variables, independent of the previous set. Thus in `print_decimal(123)` the first `print_decimal` receives the argument `n = 123`. It passes 12 to a second `print_decimal`, which in turn passes 1 to a third. The third-level `print_decimal` prints 1, then returns to the second level. That `print_decimal` prints 2, then returns to the first level. That one prints 3 and terminates.

Another good example of recursion is quicksort, a sorting algorithm developed by C. A. R. Hoare in 1962. Given a list, one element is chosen and the others are partitioned into two subsets: those less than the partition element and those greater than or equal to it. The same process is then applied recursively to the two subsets. When a subset has fewer than two elements, it does not need any sorting; this stops the recursion.

Our version of quicksort is not the fastest possible, but it is one of the simplest. We use the middle element of each part for partitioning.

```koo
// Sort v[left]...v[right] into increasing order.
fn quick_sort(v: var [int], left: int, right: int) {
    if left >= right {      // do nothing if the part has fewer than two elements
        return
    }
    v.swap(left, (left + right) / 2)    // move the partition element to v[left]
    var last := left
    for i in left + 1..=right {         // partition
        if v[i] < v[left] {
            last += 1
            v.swap(last, i)
        }
    }
    v.swap(left, last)                  // restore the partition element
    quick_sort(var v, left, last - 1)
    quick_sort(var v, last + 1, right)
}

fn main() {
    var v := [31, 41, 59, 26, 53, 58, 97, 93, 23, 84]
    quick_sort(var v, 0, v.len - 1)
    print(v)
}
```

```text
[23, 26, 31, 41, 53, 58, 59, 84, 93, 97]
```

The function sorts in place: all its invocations work on the one list of `main`, which each lends to the next with `var`.

Recursion may provide no saving in storage, since somewhere a stack of the values being processed must be maintained. Nor will it be faster. But recursive code is more compact, and often much easier to write and understand than the non-recursive equivalent. Recursion is especially convenient for recursively defined data structures like trees; we will see a nice example in Section 6.5.

A recursion that never stops, or that goes many hundreds of thousands of calls deep, runs out of the memory that is set aside for calls. At present such a program is ended by the operating system, without the message and line number of a panic.

**Exercise 4-6.** Adapt the ideas of `print_decimal` to write a recursive version of `itoa`; that is, convert an integer into a string by calling a recursive function.

**Exercise 4-7.** Write a recursive version of the function `reverse(s)`, which reverses the string `s`.

## 4.8 What Is Not There

C programs begin by passing through a *preprocessor*, which includes files, expands macros and leaves out parts of the program on request. Koo has no preprocessor, and nothing like one.

There is nothing to include, because a package sees all its files and a file imports packages by name. In place of a macro that names a constant there is a constant. In place of a macro that computes, there is a function; in an optimized build the compiler substitutes short functions into their callers by itself, so nothing is gained by doing so by hand. And where C programmers write a macro because a function could not be made to work for several types, Koo has functions that are parameterized by type, which Chapter 6 describes.
