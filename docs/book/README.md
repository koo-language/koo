# The Koo Programming Language

## Preface

Koo is a compiled language for writing the programs that run behind a network: services, tools, and the plumbing between them. It is small. A value is a value and is never secretly shared; an error is part of a function's type and cannot be dropped by accident; state that several parts of a program use belongs to an actor, which runs beside the others without locks.

This book is meant to help the reader learn to program in Koo. It is not an introduction to programming: it assumes that you know what a variable, a loop and a function are. It is organized as *The C Programming Language* by Kernighan and Ritchie is, because we know no better way to teach a language. Chapter 1 is a tutorial on the central part of Koo, so that you can write useful programs as soon as possible. Chapters 2 through 8 cover the same ground again, completely and in order. Chapters 9 and 10 are about what Koo was made for: actors, and a server. The appendices are a reference manual, a summary of the standard library, and a guide to the compiler.

The book describes the language that the compiler builds today, and nothing else. Koo is young, and its [specification](../../SPEC.md) describes a good deal that is not built yet; none of that is taught here, and Appendix C lists it so that you know where the edges are.

We have tried to make the book exact, and there is a mechanism behind the attempt. Every program in these pages is a whole program. The test suite of the compiler takes each one out of the book, builds it, runs it, and compares what it does with what the book says it does, to the letter: what it prints, what a panic writes, what the compiler says of a program that is wrong on purpose. The output shown under a program was put there by the program. If the compiler changes and the book does not, the tests fail.

There are exercises throughout. They are part of the text: the quickest way to learn a language is to write programs in it.

## Contents

1. [A Tutorial Introduction](01-tutorial.md)
2. [Types, Operators and Expressions](02-types.md)
3. [Control Flow](03-control.md)
4. [Functions and Program Structure](04-functions.md)
5. [Lists, Strings and Maps](05-collections.md)
6. [Structures](06-structures.md)
7. [Errors](07-errors.md)
8. [Input and Output](08-io.md)
9. [Actors](09-actors.md)
10. [A Server](10-server.md)

- [Appendix A: Reference Manual](a-reference.md)
- [Appendix B: The Standard Library](b-library.md)
- [Appendix C: The Compiler](c-compiler.md)

## Introduction

Koo is a general-purpose language with a particular job in mind. Most of the programs it is for spend their time taking requests, transforming data, keeping some state and answering. Such programs are usually written in a language that is easy and slow, or in one that is fast and demanding. Koo tries to be easy and fast, by choosing a small set of ideas and making each of them do a lot of work.

*Values.* Every value in Koo behaves like a number does. If you assign a list to another name, or pass it to a function, the two are independent: changing one never changes the other. There are no references to take and no pointers to follow, so there is no question of who else can see a change. A function can change its argument only when both the function and the call say `var`. The compiler arranges that this costs little: nothing is copied until one of two holders changes what they share, and memory is freed the moment its last holder is gone. There is no garbage collector, and there is nothing to free by hand.

*Types.* The basic types are integers, floating-point numbers, booleans, bytes and strings. From them you build lists, maps, tuples, structures and enumerations. A value that may be absent has an optional type, and the compiler does not let you use it until you have said what happens when it is absent. There is no null. Types are checked when the program is compiled, but they seldom have to be written, because the compiler works them out.

*Errors.* A function that can fail says so in its type, and says how. The caller handles the failure or passes it on with a single character. A failure that is neither handled nor passed on is a compile-time error, so the paths through a program on which something goes wrong are as visible as the path on which nothing does.

*Checks.* Arithmetic that overflows, an index outside a list and a division by zero stop the program with a message and a line number. They never produce a wrong answer quietly.

*Actors.* State that is shared lives in an actor, which answers one message at a time. Actors run on all the processors of the machine, and what they hand each other is shared without being copied, because once handed over it can no longer be changed in place. There are no locks to take and no races to find.

Koo is compiled. The compiler translates a program to machine code, by way of LLVM, and the result is a single executable. On the eleven small benchmarks in the repository, a Koo program takes between 0.7 and 1.8 times as long as the same program written in Rust; the [README](../../README.md#benchmarks) has the table.

The language is not finished. This book tells you plainly where something is missing, and what the compiler says when you reach for it.

You will need the compiler to follow along. The [README](../../README.md) at the top of the repository says how to build it; from then on,

```sh
koo run hello.koo
```

builds the program in `hello.koo` and runs it.
