# Chapter 7: Errors

Things go wrong. A file is not there, a number that someone typed is not a number, an order asks for more than is in stock. A program worth using spends a large share of its code on such cases, and much of the difference between languages lies in how they have you write that share.

C has no mechanism for errors at all, only conventions: a function returns −1, or a null pointer, or sets a variable somewhere, and the caller is trusted to look. The trouble with trust is that nothing happens when it is misplaced, until much later. Languages with exceptions repair that, at a price: any call may leave the function it is in, by a path that is written nowhere, and you cannot tell from reading a function which of its lines may do so.

Koo takes a third course. A function that can fail says so in its type, and says how. The caller of such a function must do something about the failure: handle it, or pass it on, with a mark that the reader can see. Nothing happens behind the page.

## 7.1 Failing

An *error type* is declared with the word `error`. It may have fields, like a structure, to carry the particulars:

    error Empty
    error TooLong { limit: int, found: int }

A function that may fail lists the errors it may fail with after an exclamation mark, following its return type. In its body, the statement `fail` ends the function with an error, as `return` ends it with a value.

```koo
error Empty
error TooLong { limit: int, found: int }

// A name that is acceptable, in lower case.
fn check(name: string) -> string ! Empty | TooLong {
    if name.len == 0 {
        fail Empty
    }
    if name.len > 8 {
        fail TooLong { limit: 8, found: name.len }
    }
    return name.to_lower()
}

fn main() {
    for name in ["Ada", "", "Bartholomew"] {
        match check(name) {
            good => print("ok: {good}")
            ! Empty => print("a name is required")
            ! TooLong { limit, found } => print("{found} letters is over the limit of {limit}")
        }
    }
}
```

```text
ok: ada
a name is required
11 letters is over the limit of 8
```

The type `string ! Empty | TooLong` is read "a string, or failing that, an `Empty` or a `TooLong`". A value of such a type is called a *result*. The errors after the `!` are the function's *error set*. A function that returns nothing but may fail is written with the `!` directly after its parameters: `fn save(path: string) ! IoError`.

`main` uses a `match` to take the result apart. The first arm is for the success, and its pattern is matched against the string. The arms that begin with `!` are for the errors: after the `!` comes a pattern for an error, which may name its fields exactly as a pattern for a structure does. The `match` must cover the success and every error of the set. If `check` is one day given a third way to fail, this `match` will stop compiling until it has an arm for it.

## 7.2 Handling an Error

What a result cannot be is ignored. It is not a `string`, so it cannot be used as one, and the compiler will not accept a call that simply drops it:

```koo
error Empty

fn check(name: string) -> string ! Empty {
    if name.len == 0 {
        fail Empty
    }
    return name
}

fn main() {
    check("ada")
    print(check("").len)
}
```

```error
error: unhandled result: Empty
  --> main.koo:11:5
   |
11 |     check("ada")
   |     ^^^^^^^^^^^^ this is a `string ! Empty`, which may be an error
   |
   = note: a result is a value or an error, and the error may not be ignored (Section 6.4)
   = help: to handle it here, give a value in its place with `else`, or inspect it with `match`
help: or pass the error on to the caller: write `?`, and let `main` fail with `Empty`
   |
10 | fn main() ! Empty {
   |          ++++++++
11 |     check("ada")?
   |                 +

error: the value may be an error, so `.len` cannot be used on it
  --> main.koo:12:11
   |
12 |     print(check("").len)
   |           ^^^^^^^^^ this is a `string ! Empty`
   |
   = help: handle the error first: `x?.len` passes it on to the caller, and `(x else fallback).len` gives a value in its place (Section 6.4)

2 errors
```

Each message names the two things that can be done. This section is about the first, handling the error here; the next is about the second.

The fullest way to handle a result is the `match` of the last section. In an error arm, the pattern may be an error type alone, as `! Empty`; a type with its fields, as `! TooLong { limit, found }`; a name and a type, as `! e: TooLong`, which binds the whole error value to the name; or a name alone, as `! e`, which matches any error that the arms above it did not.

More often all that is wanted is a value to carry on with. That is the operator `else`, which we have been using since Chapter 1:

    name := check(input) else "anonymous"

When the alternative needs to know what went wrong, a name and `=>` after the `else` bind the error:

    name := check(input) else e => "rejected: {e}"

And as with an optional, the alternative may be a block that leaves, by `return`, `break`, `continue` or `fail`.

```koo
import std/io

error Empty
error TooLong { limit: int, found: int }

fn check(name: string) -> string ! Empty | TooLong {
    if name.len == 0 {
        fail Empty
    }
    if name.len > 8 {
        fail TooLong { limit: 8, found: name.len }
    }
    return name.to_lower()
}

fn main() {
    var accepted: [string] = []
    while line := io.read_line() {
        print(check(line) else "anonymous")
        print(check(line) else e => "rejected: {e}")

        name := check(line) else {
            continue
        }
        accepted.push(name)
    }
    print(accepted)
}
```

```input
Grace

Bartholomew
```

```text
grace
grace
anonymous
rejected: Empty
anonymous
rejected: TooLong { limit: 8, found: 11 }
["grace"]
```

An error can be printed like any other value, as the second `print` shows.

The conversions of Chapter 2 are functions of this kind, and now their types can be told in full: `int.parse(s)` is an `int ! ParseError`, `byte.from(n)` a `byte ! Overflow`, and `x.round()` an `int ! Overflow`.

## 7.3 Passing an Error On

Very often the function in which an error turns up is not the place to deal with it. A function that reads a configuration file cannot know what its caller wants done when the file is missing; it can only stop and say so. For this there is the postfix operator `?`.

After an expression whose value is a result, `?` means: if this is an error, fail with it, here and now; otherwise, the value. So `int.parse(text)?` is an `int`, and the lines after it run only if the text was a number.

```koo
error Missing { key: string }

// The value of a setting, as text.
fn setting(settings: [string: string], key: string) -> string ! Missing {
    return settings[key] else fail Missing { key }
}

// The value of a setting that is a number.
fn number(settings: [string: string], key: string) -> int ! Missing | ParseError {
    text := setting(settings, key)?
    return int.parse(text)?
}

// The address to listen at.
fn address(settings: [string: string]) -> string ! Missing | ParseError {
    host := setting(settings, "host")?
    port := number(settings, "port")?
    return "{host}:{port}"
}

fn main() {
    good := ["host": "localhost", "port": "8080"]
    bad := ["host": "localhost", "port": "eighty"]
    none_at_all: [string: string] = [:]

    for settings in [good, bad, none_at_all] {
        match address(settings) {
            found => print(found)
            ! Missing { key } => print("no {key} is set")
            ! e: ParseError => print("{e.input} is not a number")
        }
    }
}
```

```text
localhost:8080
eighty is not a number
no host is set
```

In `setting`, the alternative of the `else` is itself a `fail`: the value of the map entry if there is one, and otherwise the function fails with `Missing`. This is the usual way to turn "it was not there" into an error that says what was not.

A function that uses `?` on an error must have that error in its own error set, since it may now fail with it. The compiler checks this, and when it finds an error that is passed on but not declared, it offers to add it:

```koo
error Missing { key: string }

fn setting(settings: [string: string], key: string) -> string ! Missing {
    return settings[key] else fail Missing { key }
}

fn port(settings: [string: string]) -> int ! Missing {
    return int.parse(setting(settings, "port")?)?
}

fn main() {
    print(port(["port": "80"]) else 0)
}
```

```error
error: `?` propagates `ParseError`, which is not in this function's error set
 --> main.koo:8:12
  |
8 |     return int.parse(setting(settings, "port")?)?
  |            ^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^ the function may fail only with `Missing`
  |
  = help: to handle it here, give a value in its place with `else`, or inspect it with `match`
help: or pass it on: add `ParseError` to the errors of `port`
  |
7 | fn port(settings: [string: string]) -> int ! Missing | ParseError {
  |                                                     +++++++++++++

1 error
```

So the first line of a function is the whole truth about how it can fail. The set is not a comment that may be out of date. The compiler does not let a function fail with an error it has not declared, nor a caller overlook one that is.

Notice what the `?` does for the reader. In `address`, the two lines that may leave the function early are the two that have a `?` at the end. A line without one runs to completion, always. With exceptions, every line is under suspicion.

Sometimes a function has something of its own to do before the error goes on its way: to give back what it had taken, or to write down that it failed. Then it handles the error as in the last section, and fails with it again. An error that was caught by `! e` or by `else e =>` may be given to `fail` just as it is, whichever of several types it has, provided that the function's error set has them all.

```koo
error Missing { key: string }

fn number(settings: [string: string], key: string) -> int ! Missing | ParseError {
    text := settings[key] else fail Missing { key }
    return int.parse(text)?
}

fn port(settings: [string: string], problems: var [string]) -> int ! Missing | ParseError {
    match number(settings, "port") {
        port => return port
        ! e => {
            problems.push("no port: {e}")
            fail e
        }
    }
}

fn main() {
    var problems: [string] = []
    good := ["port": "8080"]
    bad := ["port": "eighty"]
    empty: [string: string] = [:]
    for settings in [good, bad, empty] {
        print(port(settings, var problems) else 0)
    }
    for problem in problems {
        print(problem)
    }
}
```

```text
8080
0
0
no port: ParseError { input: "eighty" }
no port: Missing { key: "port" }
```

## 7.4 Error Sets

When several functions fail in the same ways, it is tedious to repeat the list. An error declaration can name a set:

    error ConfigError = Missing | ParseError

and `ConfigError` may then be written wherever the list could be.

There is also a set that contains every error, called `Failure`. A function declared `! Failure` may fail with anything. It is the right type for a function that calls many others and does not care to enumerate their failures, at the price that its callers cannot tell them apart by a `match` that is known to be complete: a `match` on a `Failure` needs a final arm `! e`.

```koo
error Missing { key: string }
error Negative { value: int }
error ConfigError = Missing | ParseError

fn number(settings: [string: string], key: string) -> int ! ConfigError {
    text := settings[key] else fail Missing { key }
    return int.parse(text)?
}

fn workers(settings: [string: string]) -> int ! Failure {
    count := number(settings, "workers")?
    if count < 0 {
        fail Negative { value: count }
    }
    return count
}

fn main() {
    for text in ["4", "-1", "many"] {
        match workers(["workers": text]) {
            count => print("{count} workers")
            ! e: Negative => print("{e.value} is negative")
            ! e => print("failed: {e}")
        }
    }
}
```

```text
4 workers
-1 is negative
failed: ParseError { input: "many" }
```

**The errors of a function that is passed.** A function that is given another function to call can fail, as a rule, in whatever ways that function can. Its error set cannot be written as a list, because it is a different list at every call. So it is written as a type parameter (Section 6.8): a name declared in the angle brackets and used after a `!` stands for the errors of the function that is passed.

```koo
error Refused { attempt: int }
error GaveUp { after: int }

// Calls `body` until it succeeds, `times` times at the most.
fn retry<T, E>(times: int, body: fn(int) -> T ! E) -> T ! E | GaveUp {
    for attempt in 1..=times {
        match body(attempt) {
            value => return value
            ! e => print("attempt {attempt} failed: {e}")
        }
    }
    fail GaveUp { after: times }
}

fn connect(attempt: int) -> string ! Refused {
    if attempt < 3 {
        fail Refused { attempt }
    }
    return "connected at attempt {attempt}"
}

fn main() {
    for times in [5, 2] {
        match retry(times, connect) {
            said => print(said)
            ! GaveUp { after } => print("gave up after {after}")
            ! e: Refused => print("refused")
        }
    }
    print(retry(3, attempt -> attempt * 10) else 0)
}
```

```text
attempt 1 failed: Refused { attempt: 1 }
attempt 2 failed: Refused { attempt: 2 }
connected at attempt 3
attempt 1 failed: Refused { attempt: 1 }
attempt 2 failed: Refused { attempt: 2 }
gave up after 2
10
```

In `retry(times, connect)` the parameter `E` is `Refused`, so the call is a `string ! Refused | GaveUp`, and the `match` on it must have an arm for each of the two, as it has. In the last call the function passed is a lambda that cannot fail. There `E` stands for no error at all, the call is an `int ! GaveUp`, and the arm of `retry` that deals with an error of `body` is not part of the function that is made for that call.

A function that only passes the errors on, doing something before or after, is written the same way, with `?` or with the `fail e` of Section 7.3:

    fn timed<T, E>(what: string, body: fn() -> T ! E) -> T ! E {
        started := time.uptime()
        value := body()?
        print("{what} took {time.uptime() - started}")
        return value
    }

Where the declaration of a function parameter lists its errors instead, as in `body: fn() -> int ! Missing | ParseError`, the list says how the function may fail and not how it must: a lambda that fails with one of the two, or with neither, may be passed.

## 7.5 Errors Are Values

An error is a value like any other. It can be made without being failed with, kept in a variable, put in a list, passed to a function. It can have methods, and if one of them is `to_string`, that is how the error is printed.

A result is a value too, and can be kept before it is examined, though it must be examined in the end: by `match`, by `else`, or by `?`.

```koo
error OutOfStock { item: string, wanted: int, have: int }

fn (self: OutOfStock) to_string() -> string {
    return "only {self.have} of {self.item} left, and {self.wanted} wanted"
}

fn withdraw(stock: var [string: int], item: string, wanted: int) -> int ! OutOfStock {
    have := stock[item] else 0
    if have < wanted {
        fail OutOfStock { item, wanted, have }
    }
    stock[item] = have - wanted
    return have - wanted
}

fn main() {
    var stock := ["pens": 5, "ink": 1]
    results := [
        withdraw(var stock, "pens", 2),
        withdraw(var stock, "ink", 3),
        withdraw(var stock, "pens", 4),
    ]
    for result in results {
        match result {
            left => print("done, {left} left")
            ! e => print(e)
        }
    }
    print(stock)
}
```

```text
done, 3 left
only 1 of ink left, and 3 wanted
only 3 of pens left, and 4 wanted
["pens": 3, "ink": 1]
```

## 7.6 When Main Fails

`main` may have an error set like any other function. If it fails, the program writes the error to the standard error output and ends with an exit status of 1, the conventional signal to whatever ran it that something went wrong.

```koo
error Missing { key: string }

fn (self: Missing) to_string() -> string {
    return "the setting `{self.key}` is missing"
}

fn setting(settings: [string: string], key: string) -> string ! Missing {
    return settings[key] else fail Missing { key }
}

fn main() ! Missing {
    settings := ["host": "localhost"]
    print(setting(settings, "host")?)
    print(setting(settings, "port")?)
    print("not reached")
}
```

```text
localhost
```

```stderr
error: the setting `port` is missing
```

For a small program this is often all the error handling there need be: every function passes its errors up with `?`, and `main` lets them out.

## 7.7 Panics

There is a second way for a Koo program to stop, which we have met already: the *panic*. An integer overflows, a subscript is out of range, a number is divided by zero, and the program writes a message and ends with status 101.

The two are for different things. An error is something that can happen to a correct program: the world outside did not cooperate. It is part of a function's type, and the caller must provide for it. A panic means that the program itself is wrong: it has reached a state that its author believed impossible. There is nothing sensible for a caller to do about that, so a panic is in no function's type and cannot be handled. It is to be fixed.

A program may panic deliberately, by calling `panic` with a message, at a point that it should never reach:

```koo
// The number of days in a month of an ordinary year.
fn days_in(month: int) -> int {
    return match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31
        4 | 6 | 9 | 11 => 30
        2 => 28
        _ => panic("there is no month {month}")
    }
}

fn main() {
    print(days_in(2))
    print(days_in(13))
}
```

```text
28
```

```panic
panic: there is no month 13
  at main.koo:7
```

The choice between failing and panicking is a matter of whose mistake it is. If `days_in` is given months that a user typed, an impossible month is the user's mistake, to be expected and reported, and the function should fail with an error. If its callers have already checked, month 13 is a bug in the program, and a panic that names the line is the quickest road to finding it.

## 7.8 An Example

To pull the chapter together, here is a program that reads a configuration in the form that many programs use, lines of `key = value`, into a structure. It reports the first thing that is wrong, with the number of the line it is on.

```koo
import std/io

type Config = { host: string = "localhost", port: int = 8080, workers: int = 1 }

error Malformed { line: int, text: string }
error UnknownKey { line: int, key: string }
error BadNumber { line: int, key: string, value: string }
error ConfigError = Malformed | UnknownKey | BadNumber

// Read a configuration from lines of `key = value`. Empty lines and lines that
// begin with # are passed over.
fn parse(lines: [string]) -> Config ! ConfigError {
    var config := Config {}
    for (i, raw) in lines.enumerate() {
        text := raw.trim()
        if text == "" or text.starts_with("#") {
            continue
        }
        (left, right) := text.cut("=") else fail Malformed { line: i + 1, text }
        key := left.trim()
        value := right.trim()
        match key {
            "host" => config.host = value
            "port" => config.port = number(i + 1, key, value)?
            "workers" => config.workers = number(i + 1, key, value)?
            _ => fail UnknownKey { line: i + 1, key }
        }
    }
    return config
}

fn number(line: int, key: string, value: string) -> int ! BadNumber {
    return int.parse(value) else fail BadNumber { line, key, value }
}

fn main() {
    match parse(io.read_lines()) {
        config => print(config)
        ! Malformed { line, text } => print("line {line}: expected `key = value`, found `{text}`")
        ! UnknownKey { line, key } => print("line {line}: there is no setting `{key}`")
        ! BadNumber { line, key, value } => print("line {line}: `{key}` must be a number, not `{value}`")
    }
}
```

```input
# The orders service.
host = orders.example.com

port = 9000
workers = four
```

```text
line 5: `workers` must be a number, not `four`
```

`Config {}` is a configuration with every field at its default. `number` shows the third use of `else`: it turns one error into another that knows more, here the line and the key, which `int.parse` could not have known.

Read `parse` once more, as a reader who has never seen it. Without knowing anything about the functions it calls, you can say exactly where it may stop early: at the `fail`, at the two lines with a `?`, and at the `else fail`. And from its first line, you can say exactly what a caller must be prepared for.

**Exercise 7-1.** Change the program so that it reports every mistake in its input, and not only the first.

**Exercise 7-2.** Add a setting `debug` whose value must be `true` or `false`, and an error for a value that is neither.

**Exercise 7-3.** Rewrite the calculator of Section 4.3 so that `operate` fails with an error in place of printing a message, and `main` prints it.
