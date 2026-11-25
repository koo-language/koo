# Appendix C: The Compiler

This appendix describes the program `koo`: how to build programs with it, how to test them, what its messages are like, and which parts of the language it does not build yet.

## C.1 Commands

```sh
koo run program.koo         # build the program and run it
koo run program.koo -- a b  # the same, giving the program the arguments a and b
koo build program.koo       # build an executable named program
koo check program.koo       # report the errors of a program, and build nothing
koo test program.koo        # run the tests in a file
```

Where a command takes a file it also takes a directory, which is a *package*: all the files in it whose names end in `.koo` are one program (Section 4.4). `koo test` given a directory also takes in the files whose names end in `_test.koo`, which the other commands leave out, so that tests can be kept beside a program without being built into it.

`run` and `build` have these options:

| | |
| --- | --- |
| `--release` | optimize the program. Without it the program is built quickly and runs slowly; with it, the reverse. Use it for anything whose speed matters |
| `-o path` | with `build`, the name of the executable |
| `--static` | on Linux, make an executable that depends on nothing installed on the machine it runs on |
| `--target name` | make an executable for Linux on whatever machine the compiler is run on: `linux-arm64` or `linux-x86-64` |
| `--sanitize` | build with AddressSanitizer, and `--sanitize=thread` with ThreadSanitizer: tools that check the program, as it runs, for misuse of memory and for races between threads. A correct compiler makes both impossible in a Koo program, and these options exist to check the compiler |

The compiler translates a program into the intermediate language of LLVM, and uses `clang` to turn that into an executable, so `clang`, version 15 or later, must be installed. If it is installed under another name, the environment variable `KOO_CLANG` gives the name.

Without `--target` a program is built for the machine at hand. With it, `koo build` makes a program for Linux on either of the architectures Koo runs on, from any machine the compiler runs on, a Mac included: a server is written on one machine and built there for the container it will run in.

```sh
koo build --release --target linux-x86-64 server.koo
```

The two targets are `linux-arm64` and `linux-x86-64`, and the names Docker gives them, `linux/arm64` and `linux/amd64`, are read as the same. If `--target` is not given and the environment variable `KOO_TARGET` is set, it names the target. Four things are to be known about a program built this way.

- Everything is linked into it, as with `--static`, so it runs on any Linux of its architecture, an empty container included. The C library in it is musl, which the compiler carries with it, and not the one of the machine it is built on. A released compiler carries it; one that you build from the source does if Rust has the two targets when it is built (`rustup target add aarch64-unknown-linux-musl x86_64-unknown-linux-musl`), and otherwise says, when asked for a target, that it was built without.
- Like any program built with `--static`, it cannot load a library, and so has no TLS: it cannot ask another server at an `https` address, and its connection to a database is not encrypted (Section 11.7 says what each `sslmode` does then).
- `clang` makes the code, as always, but it links a program only for the machine it is on. Linking for another takes LLVM's linker, `ld.lld`: `brew install lld` on macOS, `apt install lld` on Debian and Ubuntu. The one a Rust toolchain carries is found as well, and `KOO_LLD` names one that is somewhere else.
- `koo run` and `koo test` take the option only when it names the kind of machine they are run on, since no other can run the program. `--sanitize` does not go with it.

Some environment variables affect a program as it runs. `KOO_LOG_LEVEL`, `KOO_LOG` and `KOO_LOG_TIME` say what the program logs and in what form (Section 8.10). `KOO_THREADS` is the number of threads that run what actors are told; without it there is one for each processor the program may use, which in a container is what the container is allowed (Section 10.7). `KOO_SIDE_BY_SIDE=off` makes a worker of a server answer one request at a time, as it did before a handler that waits could be left to wait (Section 10.7); it is there in case of trouble. And if `KOO_LEAKS` is set, a program that ends normally with memory still allocated says so and ends with status 102. This too is a check on the compiler, since a Koo program has no way to leak memory by its own doing.

## C.2 Tests

A *test* is a function with no parameters whose name begins with `test_`. `koo test` runs every test of a file or package, each in a process of its own, so that a test that panics does not take the others with it, and reports which passed and which failed. The tests are run in the alphabetical order of their names. A test fails if it panics, and the package `std/test` has three functions that panic on purpose: `test.assert(condition)`, `test.assert_eq(left, right)` and `test.fail(message)`.

```koo
import std/test

fn slug(title: string) -> string {
    return title.trim().to_lower().replace(" ", "-")
}

fn test_slug_replaces_spaces() {
    test.assert_eq(slug("Hello World"), "hello-world")
}

fn test_slug_trims() {
    test.assert_eq(slug("  Koo  "), "koo")
    test.assert(slug("").is_empty())
}

fn test_slug_of_punctuation() {
    test.assert_eq(slug("Hello, World!"), "hello-world")
}

fn main() {
    print(slug("A Tour of Koo"))
}
```

`koo test` reports:

```test
FAILED test_slug_of_punctuation
       panic: assertion failed at main.koo:17
          left: "hello,-world!"
         right: "hello-world"
ok     test_slug_replaces_spaces
ok     test_slug_trims

1 of 3 tests failed
```

A failed assertion shows the line it is on and, for `assert_eq`, the two values, each in the form in which it would be written in a program. `koo test` ends with status 0 if every test passed, and 1 otherwise. `main` is not run.

## C.3 Messages

A program that the compiler cannot accept is answered with one message for each mistake. A message has the same parts each time:

```koo
fn add_all(names: var [string], more: [string]) {
    for name in more {
        names.push(name)
    }
}

fn main() {
    var names := ["ada"]
    add_all(names, ["bob"])
    print(names.frist() else "nobody")
}
```

```error
error: `add_all` changes this argument, so it is passed with `var`
 --> main.koo:9:13
  |
1 | fn add_all(names: var [string], more: [string]) {
  |            ------------------- the parameter `names` is `var`
...
9 |     add_all(names, ["bob"])
  |             ^^^^^
  |
  = note: `var` at the call shows the reader that the argument may change (Section 5.3)
help: write `var` before the argument
  |
9 |     add_all(var names, ["bob"])
  |             ++++

error: `[string]` has no method `frist`
  --> main.koo:10:17
   |
10 |     print(names.frist() else "nobody")
   |                 ^^^^^
   |
help: did you mean `first`?
   |
10 -     print(names.frist() else "nobody")
10 +     print(names.first() else "nobody")

2 errors
```

- The first line says what is wrong, in the terms of the program.
- The second gives the file, the line and the column.
- Then come the lines concerned. The place of the mistake is marked with `^`, and other places that explain it, such as the declaration that made a parameter `var`, are marked with `-` and a few words.
- A line that begins with `= note:` gives the rule that was broken; the section it cites is a section of the specification. One that begins with `= help:` suggests what to do.
- Where there is exactly one thing that must have been meant, a `help:` at the left margin shows the corrected line, with `+` under what was added, or the old line and the new marked `-` and `+`.

A name that is nearly the name of something that exists gets a "did you mean". Syntax that belongs to another language, such as `let`, `function`, `&&`, `x++`, a semicolon or `null`, gets a message that says what Koo has in its place.

The compiler reports the mistakes it can find in one run, not only the first, and tries not to report a mistake that is only the consequence of another.

## C.4 What Is Not Built

The [specification](../../SPEC.md) describes more than the compiler builds. For a construct that it knows of and does not build, the compiler says "is not supported by this compiler yet". These are the ones that a reader of this book is most likely to reach for.

*Types.* There is no `char`: a character is a string. There are no integer or floating-point types of other sizes, and no decimal numbers. A `Time` cannot yet be shown in the time of a place, only in UTC, and there are no dates apart from times. A structure cannot contain itself through an optional (Section 6.5). An interface cannot be the type of a value (Section 6.9). `distinct` types and `resource` types are not built.

*Type parameters.* The body of a function with type parameters is checked for each type it is used with, and not once against its constraints (Section 6.8). An actor cannot have type parameters.

*Expressions.* A list is sliced with the method `slice`, not with `v[a..b]`. A range is not a value: it can be the sequence of a `for` and a pattern, and nothing else. There is no `is` test; use `match`. A tuple cannot be assigned to as a whole, so two variables are exchanged through a third.

*Statements.* Loops cannot be labeled, so `break` and `continue` apply to the innermost loop. There is no `defer`.

*Constants.* A constant at the top level is a number, a string, or `true` or `false`; it cannot be a list, a structure or a computed value. A default, of a field or of a parameter, is one of those, a duration, `none`, an empty list or map, or a variant of an enumeration that has no fields, as `priority: Priority = Priority.Normal`.

*Concurrency.* Of the concurrency that the specification describes, actors are built, as Chapter 9 describes them, with the limits listed in Section 9.11. Tasks, `spawn`, `scope`, channels and `select` are not.

*Packages.* A program is one package. One package of your own cannot yet import another, and there is no package manager. The standard packages are `std/io`, `std/os`, `std/time`, `std/http`, `std/sql`, `std/sql/postgres`, `std/hash`, `std/encoding`, `std/crypto`, `std/random`, `std/log` and `std/test`.

*The library.* There are no sockets for a program's own use, only the clients for HTTP and PostgreSQL; no digest but SHA-256; no encryption except of those clients' connections; and no way to read a file except whole. Section 11.8 lists what `std/sql` leaves out.

*Targets.* The compiler runs on macOS on arm64, and on Linux on x86-64 and arm64, and builds for the machine it runs on; with `--target` it builds for either Linux from any of the three (Section C.1). It does not build for macOS from Linux, for Windows, or for WebAssembly.

## C.5 Reliability

The compiler is new, and it would be wrong to end a book that claims exactness without saying how much the claim covers.

What is checked, every time the compiler's tests are run: every program in this book is built and run, and what it does is compared with what the book says, as the Preface described. Some two hundred other programs are checked as well: those that should run are built with and without optimization, and fail the tests if they print the wrong thing or end with memory still allocated, and those that should be refused are compared with the messages they should get. The test programs with actors are also run under ThreadSanitizer, which reports nothing. The programs of Chapter 11, and others that use a database, are run when the tests are given a PostgreSQL server to use, each against a database with nothing in it; without one they are only checked by the compiler.

What is not: a compiler is a large program, this one has had few users, and it has not been tested at the scale that finds the rare bug. If a program of yours behaves in a way that this book says it should not, the book is right about the language and the compiler is wrong, and its authors would like to hear of it.
