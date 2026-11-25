# Koo Language Specification

**Version 0.5 — draft.** This document is the normative description of the Koo language, its standard library and its toolchain. Specification versions are numbered independently of the implementation milestones of Section 13.7, which this document always calls "milestone 0.x". It changes through the proposal process in Section 2. Numbered sections correspond one-to-one with the conformance test suite. Section 15 records changes between versions.

Throughout, "is a compile error" means the compiler rejects the program; "panics" means Section 5.9 applies; "suspends" and "blocks" mean Section 7.1 applies; "target" marks a measurable goal that the benchmark suite (Section 13.6) tracks and that is not a language guarantee.

## 1. Introduction

Koo is a compiled programming language for backend services. It targets the performance of Rust with the ease of use of a scripting language: no garbage collector, no borrow checker, no `async`/`await`, and one way to do each common thing. Programs compile to native binaries for arm64 and x86-64, and to WebAssembly.

### 1.1 Design goals

1. **Rust-class performance.** No garbage collector, monomorphized generics, value types laid out inline, optimized code via LLVM. Measured, not asserted: Section 13.6 defines the benchmark suite and its targets against named Rust and Go baselines, and Section 5.11 lists every place where Koo's design costs more than Rust's and what bounds the cost.
2. **Readable by someone who has never seen Koo.** Every feature must be explainable in one sentence to a working programmer. If it cannot be, it is not in the language.
3. **Safe by default.** No null, no data races, no unchecked errors, no undefined behavior outside `unsafe` blocks. Memory safety is guaranteed by the compiler without programmer-written lifetimes.
4. **Concurrency without ceremony.** Values are never shared mutably; state is owned by an actor or published as a snapshot. Blocking is cheap. Every function can block; none is colored.
5. **JSON is native.** Any type is decoded from and encoded to JSON, validated, and described as a schema with no annotation, no schema file and no dependency (Section 10.4).
6. **One way.** One string type, one error mechanism, one formatter with no options, one package manager, one test runner, one HTTP server in the standard library. Where two features would do the same job, one is cut. A language with one way to write each thing is also the one a code-generating model writes correctly most often, and the one a reviewer checks fastest.
7. **Fast compiles.** Section 12.2 states the targets. Features that threaten them are rejected.
8. **Runs anywhere.** A Koo binary is statically linked on Linux by default and has no runtime dependency. The same service source builds for x86-64, arm64 and WebAssembly, and core mode (Section 9) runs with no operating system at all.

### 1.2 Target use cases

- HTTP, gRPC and raw TCP/UDP services, from a single endpoint to a large API
- Background workers, queue consumers, schedulers and cron-style jobs
- Kubernetes operators, controllers, CLIs and infrastructure tooling
- The services around machine-learning systems: inference servers, feature and embedding pipelines, agent and retrieval backends
- WebAssembly components for edge compute, and WebAssembly modules for plugin systems and sandboxed execution
- Libraries with a C ABI embedded in programs written in other languages (core mode, Section 9)

### 1.3 Non-goals

- Game engines and real-time graphics
- Desktop or mobile GUI applications
- Scripting or REPL-first interactive use
- Numerical kernels, GPU programming and model training
- Kernel modules, drivers and firmware at 1.0. Core mode keeps them possible; they are not a target the 1.0 design is judged against.
- Being a superset of or source-compatible with any existing language
- Supporting every programming paradigm. Koo is imperative with value semantics and actors. It has no inheritance, no macros, no operator overloading and no exceptions.

### 1.4 Reference example

A complete Koo service. Every construct in it is specified later in this document. It has no dependencies outside the standard library, and the same source builds as a native server and as a WebAssembly component (Section 9.5).

```koo
import std/os
import std/http
import std/sql
import std/sql/postgres

pub type Id = distinct int
pub type Order = { id: Id, customer: Id, total: Decimal, placed: Time }
pub type NewOrder = { customer: Id, total: Decimal }
error NotFound { id: Id }

fn get_order(db: sql.Pool, id: Id) -> Order ! NotFound | sql.Error {
    row := db.query_one("select * from orders where id = {id}")? else fail NotFound { id }
    return Order.from_row(row)?
}

fn create_order(db: sql.Pool, input: NewOrder) -> Order ! sql.Error {
    row := db.query_row(
        "insert into orders (customer, total) values ({input.customer}, {input.total}) returning *"
    )?
    return Order.from_row(row)?
}

fn get_many(db: sql.Pool, ids: [Id]) -> [Order] ! NotFound | sql.Error {
    return ids.map_parallel(id -> get_order(db, id))?
}

fn main() ! sql.Error | http.Error {
    url := os.env("DATABASE_URL") else "postgres://localhost/orders"
    db := postgres.connect(url, pool: 20)?
    var app := http.App.new()
    app.get("/orders/:id", req -> {
        id := Id.parse(req.path("id"))?
        return http.json(get_order(db, id)?)
    })
    app.post("/orders", req -> {
        input: NewOrder = req.json()?
        return http.json(create_order(db, input)?, status: 201)
    })
    app.on_error((e: NotFound) -> http.status(404, "order {e.id} not found"))
    app.serve(port: 8080)?
}
```

What the example does not contain is the point. The `{id}` in the query is a bound parameter, not text spliced into SQL (Section 10.3). `Order` and `NewOrder` are read from rows and JSON and written as JSON with no annotations (Section 10.4). A malformed id or body is answered with a 400 that names the offending field, and a bug in one handler fails one request (Section 5.9). Nothing is marked `async`, and no value is shared between requests.

## 2. Project

Koo is open source under permissive licenses and developed in the open at `koo-language/koo`. Contributions are welcome; language changes go through a public proposal process.

### 2.1 License

The compiler, standard library, runtime and tooling are dual-licensed under the **Apache License 2.0** and the **MIT License**, at the user's choice (the arrangement Rust uses). Apache 2.0 provides an explicit patent grant; MIT is compatible with GPLv2 projects, including the Linux kernel.

The project commits to never relicensing under a source-available, non-commercial or otherwise restrictive license. This commitment is repeated in `LICENSE` and in the README.

### 2.2 Contributing

Contributors certify their right to submit each change by signing off their commits under the **Developer Certificate of Origin (DCO)**, the mechanism used by the Linux kernel and the CNCF. No Contributor License Agreement is required and no copyright is assigned. Contributions are licensed under the same dual license as the project.

Pull requests are welcome for bug fixes, standard library additions, documentation, platform support and tooling. Anything that changes the language's syntax or semantics goes through a proposal (Section 2.3).

### 2.3 Governance

Koo follows the project-lead model used by Python, Zig and Ruby. `GOVERNANCE.md` is the full description.

- **Project lead:** Doron Grinstein. Final authority on language design, the roadmap and releases.
- **Core team:** maintainers with merge rights on specified areas. Membership is listed in `GOVERNANCE.md`.
- **Language proposals:** any syntax or semantics change is proposed as a numbered document in `koo-language/proposals`. The core team discusses; the project lead accepts, rejects or defers. Accepted proposals are implemented behind a feature flag before stabilizing.
- **Stability:** from version 1.0, code that compiles under 1.x compiles under every later 1.x release. Breaking changes require a new major version and ship with an automated migration tool.

### 2.4 Trademark

The name **Koo** and the Koo logo are trademarks of Doron Grinstein. The trademark policy in `TRADEMARK.md` follows the practice of Rust, Go and Python: anyone may say that software is written in Koo or works with Koo, use the name in package names, tutorials, books and talks, and distribute unmodified builds of the compiler under the Koo name. The policy reserves the use of the name for modified forks, product and company names, and official conformance or certification marks, so that "Koo" always means this language.

### 2.5 Names and conventions

| Item | Name |
| --- | --- |
| Language | Koo |
| Source file extension | `.koo` |
| Compiler and CLI | `koo` (`koo build`, `koo run`, `koo test`, `koo fmt`) |
| GitHub organization | `koo-language` |
| Main repository | `koo-language/koo` |
| Proposals repository | `koo-language/proposals` |
| First-party packages | `koo-language/<name>` on the registry |
| Website | koo.io |
| Package registry | pkg.koo.io |

## 3. Lexical structure and grammar

Koo source is UTF-8 text in `.koo` files. The grammar is parsed by a hand-written recursive-descent parser without type information and formatted by a single canonical formatter. The full grammar in EBNF lives in `koo-language/koo/spec/grammar.ebnf` and is normative; this section states the rules a reader needs.

### 3.1 Identifiers and naming

Identifiers are ASCII letters, digits and underscores, not starting with a digit. Case carries meaning and the compiler enforces it:

| Kind | Convention | Example |
| --- | --- | --- |
| User-defined types, enum variants, actors, interfaces, error types | UpperCamelCase | `Order`, `Card`, `NotFound`, `Database` |
| Built-in primitive types | lowercase | `int`, `float`, `bool`, `string`, `char`, `byte`, `i32` |
| Functions, variables, fields, packages, constants, labels | snake_case | `get_order`, `total_due`, `max_retries` |
| Names imported from C (Section 11) | as spelled in C | `c.compressBound`, `c.Z_OK` |

A capitalized identifier followed by `{` is a struct literal or a destructuring pattern, depending on position (Section 3.9). A capitalized identifier followed by `.` is a type-level access (`Order.from_row`, `Payment.Cash`, `Channel<int>.new`). Acronyms are written as words: `HttpServer`, `user_id`, `to_json`.

The primitive type names of Section 4.2 are predeclared. They cannot be declared again or used as the name of a binding, parameter, constant, label or import. Like the reserved words they may name a member (`row.int("amount")`, Section 3.2). A name imported from C is always written after its import's prefix (`c.Z_OK`), and after that prefix case carries no meaning (Section 11.1).

### 3.2 Keywords

The complete reserved word list, 32 words:

`actor` `and` `as` `break` `continue` `defer` `distinct` `else` `enum` `error` `fail` `fn` `for` `if` `import` `in` `interface` `is` `match` `not` `or` `pub` `resource` `return` `scope` `select` `spawn` `take` `type` `unsafe` `var` `while`

A word that follows `.` is always a member name, so the reserved words are free there: `log.error(...)`, `rows.all()`, `event.type`. Such a member is declared the same way, with the word as a function or method name after `fn` or as a field name before `:` (`fn error(...)` in `std/log`, `{ type: string }`), and is reachable only after a `.` or in a struct literal. The primitive type names (Section 3.1) may name members on the same terms. `true`, `false` and `none` are literals. `self` is an ordinary identifier bound inside methods, and `Self` is a type name bound inside interfaces (Section 4.9). Five words are **contextual**, reserved only in the position named: `c` (after `import` and before a string), `link` (in an `import c` declaration), `after` (at the start of a `select` arm), and `all` and `race` (as the bare callee of a call, Section 7.2).

### 3.3 Literals

| Literal | Form | Type |
| --- | --- | --- |
| Integer | `42`, `1_000_000`, `0xFF`, `0b1010`; an underscore may separate two digits | `int`, or a sized type when the context requires one |
| Float | `3.14`, `1e9`, `2.5e-3` | `float` |
| Decimal | `19.99d`, `0.1d`, `110d` | `Decimal` |
| Duration | `30s`, `5min`, `2h`, `250ms`, `10us`, `1h30min` | `Duration` |
| Character | `'a'`, `'\n'`, `'\u{1F600}'` | `char` |
| String | `"hello"`, with the escapes `\n` `\r` `\t` `\0` `\\` `\"` `\'` `\{` `\u{1F600}`; on one line | `string` |
| Interpolated string | `"total: {order.total}"`; a literal brace is `\{` | `string` |
| Text block | `"""` on its own line, then lines of text, then `"""`; interpolated; the closing delimiter's indentation is removed from every line | `string` |
| Raw string | `` `no \escapes or {interpolation} here` `` (backticks, may span lines) | `string` |
| Boolean | `true`, `false` | `bool` |
| Absent | `none` | `T?` |
| List | `[1, 2, 3]`, `[]` | `[int]`, or `Set<int>` when that is the expected type |
| Map | `["a": 1, "b": 2]`, `[:]` | `[string: int]` |
| Tuple | `(1, "a")`, `()` | `(int, string)`, `()` |
| Struct | `Order { id: id, total: 10d }` or shorthand `Order { id, total }` | `Order` |

The `d` suffix is the only way to write a decimal literal; `0.1` without it is a float. It follows decimal digits only, so `0x1d` is the hexadecimal integer 29. The duration units are `ns`, `us`, `ms`, `s`, `min` and `h`; there is no day unit (Section 10.6), so `d` is never ambiguous. A duration literal is one or more pairs of a whole number and a unit with no spaces, largest unit first, each unit at most once; a fraction is written in the smaller unit (`1500ms`, `1h30min`). Every string literal is interpolated unless raw; `{` inside a string begins an expression and is escaped as `\{`; `}` outside an interpolation is an ordinary character. An interpolation is written on one line and holds any expression, string literals included; it may end with a format after a colon: `{ratio:.2}`, `{id:08}`, `{name:>10}`. A text block is the form for multi-line SQL, and like any interpolated literal it may be a template (below). Maps use square brackets so that braces always mean a block, a struct literal or a pattern. A field-less struct, error or enum variant is written by its name alone: `Timeout`, `Payment.Cash`.

**Template types.** When the expected type of an interpolated string literal is a *template type*, the literal is not built into a string: the compiler passes the literal's fixed segments and its interpolated values to the type separately. A template type declares a type-level function `from_template(parts: [string], values: ...P)` returning the type: it receives the literal's fixed segments, one more than it has interpolations, and then the interpolated values, each of which must satisfy the parameter type `P`. An interpolation format (`{x:.2}`) is a compile error in a template, because the value is not being turned into text. Only `std` may declare a template type. There are two: `sql.Query`, whose values are `sql.Param`s and become bound parameters (Section 10.3), and `log.Message`, whose values become structured fields. A `string` value is not accepted where a template type is expected.

### 3.4 Operators and precedence

From highest to lowest precedence. Operators on one line are left-associative, with two exceptions: levels 5 and 6 do not associate (`a < b < c` is a compile error), and level 10 associates to the right, so `a else b else c` tries each in turn.

| Level | Operators | Meaning |
| --- | --- | --- |
| 1 | `x.f` `x.f(args)` `f(args)` `x[i]` `x[a..b]` `x?` | Postfix: field, call, index, slice, propagate |
| 2 | `-x` | Negation |
| 3 | `* / %` | Multiplicative |
| 4 | `+ -` | Additive |
| 5 | `a..b` `a..=b` | Range (exclusive, inclusive end) |
| 6 | `== != < <= > >=` `x is T` | Comparison; type test (Section 6.6) |
| 7 | `not x` | Logical not |
| 8 | `and` | Logical and |
| 9 | `or` | Logical or |
| 10 | `x else y` | Fallback (Section 6.4) |

`:=` and `=`, `+=`, `-=`, `*=`, `/=` are statements, not operators, and have no precedence. `->` introduces a return type or a lambda body; `=>` introduces a match or select arm, or the alternative of a binding `else`; `|` separates error types in an error set and alternatives in a pattern; `!` separates a success type from its error set and begins an error arm in a `match` on a result; `+` between constraints joins them (Section 4.10). These are punctuation, not operators. `&&`, `||`, `++`, `--`, `<<`, `>>`, `&`, `^`, `~` do not exist; bit operations are methods (`a.bit_and(b)`, `a.shl(3)`).

No operator can be given a new meaning. Arithmetic is defined on the built-in numeric types, `Decimal`, `Duration` and `Time` only, and `+` also joins strings. `==` and `!=` are structural equality, defined by the compiler for every type whose parts have it. Every type has it except function types, interface values and types that contain one; handles (Section 5.8) are equal when they refer to the same object. The prelude interface `Equatable` names the types that have `==`, for use as a constraint (Section 4.9). `<`, `<=`, `>` and `>=` are defined for the built-in ordered types of Section 4.2, and for every user-defined type that satisfies `Ordered`, where they call its `compare` (Section 4.9). An operator means in generic code what it means outside it: it is resolved for the type argument.

### 3.5 Declarations

`name := expr` declares an immutable binding with inferred type. `var name := expr` declares a mutable one. `name: Type = expr` and `var name: Type = expr` give an explicit type. Redeclaring a name in the same scope is a compile error; an inner scope may shadow an outer name.

Top-level declarations are constants: types, functions, actors, interfaces, errors, and bindings whose initializer is a compile-time constant expression (a literal, or an operator or constructor applied to constants). A top-level binding names one constant and is never `var`. Anything that runs at startup happens in `main`. There is no initialization order because there is no runtime initialization outside `main`.

### 3.6 Comments and documentation

`//` begins a line comment. `///` begins a documentation comment attached to the following declaration; `koo doc` renders it. There are no block comments.

### 3.7 Statements and lines

Statements end at newline; there are no semicolons. Braces are of two kinds. **Block braces** hold statements or declarations, one per line: the bodies of functions, lambdas, control flow, `match`, `select`, `scope`, `spawn`, `defer` and `unsafe`, and of `actor`, `interface` and `enum` declarations (in an `enum` body a comma may stand for the newline). **Literal braces** hold a comma-separated list: a struct literal, a struct type or a pattern. Which kind a brace is follows from what precedes it (Section 3.9), never from what is inside it.

A newline ends a statement when the innermost open bracket is a block brace, or when no bracket is open. Inside `(`, `[` and literal braces a newline is white space, and the list they hold may end with a comma; a block nested inside them, such as a lambda body passed as an argument, has significant newlines again. The angle brackets of a type-argument list are not brackets in this sense. In addition, a statement continues onto the next line when that line begins with `.`, `else`, `|`, or a binary operator from levels 3 to 9 of Section 3.4 other than `-`. A line that begins with `-` starts a new statement, because `-` also begins an expression; a long subtraction is wrapped inside parentheses. Nothing about how a line ends causes continuation. A long error set is therefore wrapped with each `|` leading its line, and the `{` that opens a block is on the line of the statement it belongs to. `koo fmt` enforces one statement per line.

An expression used as a statement must be a call, or end in `?` or in an `else` alternative (`done.recv() else return`). Its value is discarded, unless it is a result, which may not be (Section 6.3).

### 3.8 Attributes

An attribute is `@name` or `@name(args)` placed before a declaration or after a field's type. An argument is a literal or a name; a name may be dotted (`yaml.codec`) or take arguments of its own (`restart(max: 5)`), and an argument may be given by name (`tag: "kind"`). The set of attributes is closed and defined by the compiler and the standard library; packages cannot define new attributes except deriver names inside `@derive`.

| Attribute | On | Meaning |
| --- | --- | --- |
| `@c` | function | C calling convention and unmangled name (Section 11.3) |
| `@export("name")` | function | WebAssembly component export (Section 9.5) |
| `@derive(pkg.deriver)` | type | Run a deriver (Section 10.5) |
| `@json(...)` | field, type | JSON mapping (Section 10.4) |
| `@sql("name")`, `@csv("name")` | field | Column or header name (Section 10.5) |
| `@closed` | enum | Variants are final (Section 4.7) |
| `@mailbox(n)` | actor | Mailbox capacity (Section 7.3) |
| `@supervise(stop)`, `@supervise(restart(max: n, within: d))` | actor | Supervision policy (Section 7.4) |
| `@serial` | test function | Run alone (Section 12.4) |

### 3.9 Disambiguation rules

- **Type arguments are never written at call sites.** `largest(xs)` infers `T`; to pin a type, annotate the result (`m: int? = largest(xs)`) or an argument. Type arguments appear only after a type name. Because case is enforced (Section 3.1), an UpperCamelCase identifier followed by `<` always begins a type-argument list, in declarations, type expressions and expressions alike (`Cache<string, Order>`, `Channel<int>.new(capacity: 8)`); to compare two enum variants with `<`, parenthesize the left one. A name after a C import's prefix never takes type arguments, so `c.Z_OK < n` is a comparison (Section 11.1). For type-level calls the list and map types have the names `List<T>` and `Map<K, V>` (`List<byte>.with_size(n)`, `Map<string, int>.new()`); everywhere else they are written `[T]` and `[K: V]`, and `koo fmt` rewrites the long form.
- **Lambdas.** `x -> body` for one untyped parameter; `(a, b) -> body` for several or for a typed parameter `(e: NotFound) -> body`; `() -> body` for none. The body is a brace block or one simple statement: an expression, an assignment (`x -> total += x`, whose value is `()`), a `return` or a `fail`. A parenthesized list followed by `->` is a lambda; otherwise it is a tuple or a parenthesized expression, and `()` is the empty tuple. `return`, `fail` and `?` in a lambda body leave the lambda, not the enclosing function.
- **Braces.** After a capitalized name in an expression, `{` opens a struct literal, except in the positions the next rule names. In a `type`, `resource`, `error` or enum-variant declaration, and inside a type, it opens a struct type. In a `match` arm before `=>`, it opens a pattern. Every other `{` opens a block.
- **Struct literals in conditions.** The condition of `if` and `while`, the scrutinee of `match` and the subject of `for` may not contain a struct literal unless parenthesized: `if a == (Order { id }) { ... }`. Without this rule `Order {` would be ambiguous with the block.
- **Bare variants.** Where a value of an enum type is expected, a bare capitalized name is looked up among that enum's variants first, then among types.
- **Patterns.** `Name { ... }` is a destructuring pattern only in a `match` arm, where it ends at `=>`. The `is` operator takes a bare type name and no braces (Section 6.6), so `if p is Cash {` is unambiguous.
- **Binding `else`.** After `else`, an identifier followed by `=>` binds the error (Section 6.4); anything else is the alternative expression. A `match` guard and a `select` operand end at the arm's own `=>`, so in them `else` never binds unless it is parenthesized: in `Card if cached(k) else fallback => f()` the alternative is `fallback`.
- **Statements that begin with `if` or `match`.** They are that statement, and nothing follows the closing brace. To use the value, bind it or parenthesize it: `(match k { ... })?`.
- **Labels.** `label: for ...` and `label: while ...` name a loop; `break label` and `continue label` target it. A label is distinguishable from a typed declaration because a keyword follows the colon.

## 4. Type system

Koo is statically typed with local inference. Types appear on function signatures and type definitions and nowhere else unless the programmer chooses. There is no `null`, no implicit conversion between numeric types, and no subtyping. Apart from interface satisfaction (Section 4.9), a value changes type without being asked in exactly four ways, none of which loses anything about the value. Literals are typed by what is expected of them (Sections 3.3 and 10.4); that is typing, not conversion.

**Widening.** A `T` is accepted where a `T?` is expected; a `T` is accepted where a `T ! E` is expected; a `T ! E1` is accepted where a `T ! E2` is expected when every error of `E1` is in `E2`; and a function value is accepted where a `resource fn` of the same signature is expected (Section 4.11). Widening applies wherever a value meets an expected type: an argument, a `return`, an assignment or typed binding, and an element of a literal. It also applies to what a function value or method returns, so a function that fails in fewer ways is accepted where one that fails in more is expected; the compiler adapts the representation. It does not reach inside other types: a `[T]` is not a `[T?]`. Inference never chooses a widening when an exact match exists: passing an `int?` to a parameter `x: T?` binds `T` to `int`.

### 4.1 Prelude

The following are in scope in every file without an import: the primitive types of Section 4.2; `Decimal`, `Duration`, `Time`, `List`, `Map`, `Set`, `Json`; the interfaces `Equatable`, `Ordered`, `Hashable`, `Failure`, `Any`, `Iterator`, `Reader`, `Writer`; the error types `Overflow`, `Inexact`, `ParseError`, `Invalid`, `IoError`; the functions `panic`, `print`. All are defined in `std/core`, except `Json`, which is defined in `std/json`. In runtime mode the prelude also includes, from `std/task`: `Channel`, `Shared`, `Timeout`, `Panic`, `with_timeout` and `isolate`. The list methods `map_parallel` and `map_parallel_settled` (Section 7.2) are built into `[T]` and implemented by the runtime, not by `std/core`; a core-mode package may not call them. Everything else is imported.

### 4.2 Primitive types

| Type | Meaning | Notes |
| --- | --- | --- |
| `int` | 64-bit signed integer | The default integer on every target, including 32-bit ones. `i64` is an alias. |
| `float` | 64-bit IEEE 754 binary floating point | For measured quantities. `f64` is an alias. |
| `Decimal` | 128-bit IEEE 754-2008 decimal floating point (decimal128), 34 significant digits | For money and anything a human counts. Section 4.3. |
| `bool` | `true` or `false` | Not convertible to or from integers. |
| `string` | Immutable UTF-8 text | The only string type. Section 4.4. |
| `char` | One Unicode scalar value, 32 bits | What `s.chars()` yields. `c.to_string()` and `int.from(c)` convert. |
| `byte` | 8-bit unsigned | `[byte]` is the type for binary data. `u8` is an alias. |
| `i8 i16 i32 u16 u32 u64 f32` | Sized numerics | For wire formats, FFI and `core` mode. |

**Conversions** are explicit and written as type-level calls. Between integer types, a conversion whose source might not fit the destination is fallible: `i32.from(x)` is `i32 ! Overflow` for `x: int`, and `int.from(n)` is `int ! Overflow` for `n: u64`. One that always fits is plain: `int.from(y)` is `int` for `y: i32`. `float.from(x)` is `float` for every integer type and rounds to the nearest representable value, as every platform does; `float.exact(x)` is `float ! Overflow` for code that must not round. From `float` to `int` the caller chooses the rounding: `f.round()`, `f.floor()`, `f.ceil()` and `f.trunc()` are each `int ! Overflow`, failing on NaN, infinity and out-of-range values. `from` and `exact` on the built-in numeric types, `char` and distinct types (Section 4.12) are provided by the compiler, one per source type. They are the only names in the language with more than one signature (Section 6.1).

**Overflow.** Integer arithmetic is checked in every build mode; overflow panics (Section 5.9). Code that expects overflow says how to treat it: `a.checked_add(b)` is `int ! Overflow`, for arithmetic on untrusted input; `a.wrapping_add(b)` and `a.saturating_mul(b)` do what their names say. The compiler removes checks it can prove unnecessary; Section 13.6 sets the cost target.

**Division.** Integer division truncates toward zero, and `%` takes the sign of the dividend. Dividing an integer by zero panics; `a.checked_div(b)` is fallible instead. `Decimal` has no `/` operator (Section 4.3). Dividing a `float` by zero follows IEEE 754.

**No implicit mixing.** `3 / 2` is `1`; `3.0 / 2.0` is `1.5`; `3 / 2.0` is a compile error. `Decimal` mixes with neither: `price * 1.1` is a compile error, `price * 1.1d` is correct.

**Ordering and hashing.** The integer types, `string`, `char`, `bool`, `Decimal`, `Duration` and `Time` satisfy `Ordered` and `Hashable`. Strings compare and order by their bytes, which is scalar-value order; no normalization is applied. `Decimal` hashes its normalized value, so `1.0d` and `1.00d`, which are equal, hash alike.

`float` satisfies `Ordered` too, but differs from the others in two ways. Its operators, `<` and its relatives included, are IEEE 754's, in generic code as elsewhere: `NaN == NaN` is false, `NaN < x` is false and `-0.0 == 0.0` is true. Its `compare` is the IEEE `totalOrder` predicate, and every library algorithm that needs a total order (`sort`, `largest`, `min`, `max`, binary search) calls `compare` and never `<`, so sorting is well defined with `NaN` last. And because no hash can agree with IEEE equality, `float` is not `Hashable`: a float, or a struct containing one, cannot be a map key or a set element.

### 4.3 Decimal

`Decimal` follows IEEE 754-2008 decimal128. Addition, subtraction and multiplication are exact whenever the exact result has at most 34 significant digits, which covers every monetary computation in practice. A result that needs more is rounded half-even to 34 digits, as the standard specifies; `a.exact_add(b)`, `a.exact_sub(b)` and `a.exact_mul(b)` return `Decimal ! Inexact` for code that must know. A result outside the exponent range panics (Section 5.9).

Division is where money goes missing, so it never happens implicitly: `total / 3d` is a compile error; `total.divide(3d, scale: 2, round: HalfEven)` is correct, and panics on a zero divisor as integer division does (`checked_divide` is fallible instead). Comparison is numeric (`1.0d == 1.00d`). Formatting preserves scale (`1.00d` prints as `1.00`).

`std/decimal` provides `Money`, a struct `{ amount: Decimal, currency: Currency }`, with methods `add`, `sub` and `scale` returning `Money ! CurrencyMismatch`. `Money` has no arithmetic operators, because an operator cannot fail.

### 4.4 Strings

`string` is an immutable sequence of Unicode scalar values encoded as UTF-8. `s[i]` and `s[a..b]` are compile errors: neither byte nor character indexing is what most callers mean. Instead:

- `s.len` is the length in bytes, O(1). `s.count()` is the number of scalar values, O(n).
- `s.chars()` iterates scalar values as `char`; `s.bytes()` iterates bytes; `s.lines()` iterates lines.
- Most code needs no positions at all: `s.split(",")`, `s.cut(":")` (the text before and after the first separator, as `(string, string)?`), `s.starts_with(x)`, `s.strip_prefix(x)`, `s.trim()`, `s.contains(x)`, `s.replace(a, b)`.
- A position, where one is needed, is a byte offset: `s.find(x)` returns `int?`, and `s.slice(from, to)` takes offsets and is O(1). An offset that is not on a scalar-value boundary is a programming error and panics, like an out-of-range list index. Offsets returned by `string` methods are always on a boundary.
- `+` concatenates; `var s` with `s += x` appends in place with amortized growth.

A substring shares the original's buffer (Section 5.1). Because strings are immutable, the buffer is never copied, and it stays allocated while any substring of it lives. `s.detach()` returns a copy with its own buffer, for a short string that would otherwise keep a large one alive.

### 4.5 Collections

| Type | Written | Semantics |
| --- | --- | --- |
| List | `[T]` | Growable, contiguous, value semantics |
| Map | `[K: V]` | Hash map, insertion-ordered iteration, `K: Hashable` |
| Set | `Set<T>` | Hash set, insertion-ordered iteration, `T: Hashable` |
| Tuple | `(A, B)` | Fixed-size, positional. Its elements are reached by destructuring, `(a, b) := pair` or `(a, _) := pair`, and in no other way. `()` is the empty tuple, the value of a function that returns nothing. |
| Fixed array | `[T; N]` | Compile-time size, no heap; for `core` mode and FFI |
| Range | `a..b`, `a..=b` | Lazy integer range |

**Size.** `len` is the number of elements of a list, map, set or fixed array, and the number of bytes of a string. It is a property, written without parentheses, and O(1).

**Indexing.** `xs[i]` on a list or fixed array panics when `i` is out of range; `xs.get(i)` returns `T?`. `m[k]` on a map returns `V?`; `m[k] = v` inserts or replaces and requires a `var` binding; `m.update(k, 0, n -> n + 1)` changes an entry in place, starting from the given value when the key is absent. `xs[a..b]` is a slice: a list sharing the original's buffer until either is mutated. A constant index into a fixed array is checked at compile time.

**Values.** Collections are values (Section 5). Collection methods (`map`, `filter`, `sort`, `sum`, `group_by`, and so on) are eager: each returns a new collection, and each stage runs to completion before the next begins. When the functions in a chain such as `xs.filter(f).map(g)` do nothing that could be noticed from outside (they do not suspend, do no input or output, change no variable but their own, and call only functions of which the same is known), the compiler fuses the chain into one pass with at most one allocation. The only differences a program could then observe concern a panic: which one is reported when two stages would both panic, and, for a chain that ends in `any`, `all` or `find`, which stop at the first answer, whether a panic in a later element happens at all. Both are unspecified. For a sequence too large to hold, use an iterator (Section 6.7).

### 4.6 Structs

```koo
pub type Order = {
    id: Id,
    customer: Id,
    total: Decimal,
    placed: Time,
    note: string = "",             // default value
    _cache: [string: int] = [:],   // underscore: visible only in this package
}
```

A struct type has no mutability of its own; whether its fields can be assigned depends on the binding (Section 5.2). Construction uses the type name and braces, with field shorthand; every field without a default must be given. A field not visible at the construction site must have a default, or the type cannot be constructed there.

A field's type may be written inline as another struct type, so a type can mirror a nested document without a name for every level: `pub type Reply = { user: { id: Id, name: string }, items: [{ sku: string, qty: int }] }`. A field's type may contain at most one inline struct type. An inline struct type is named by its field in UpperCamelCase under the enclosing type (`Reply.User`, `Reply.Items`), and is constructed like any struct: `Reply { user: Reply.User { id, name }, items: [] }`.

Every struct has `==` when all its fields have `==`, satisfies `Hashable` when all its fields do, and has a generated `to_string()` method for debugging, which a hand-written `to_string()` replaces. JSON and other formats are in Sections 10.4 and 10.5.

**Methods** are declared in the same package as the type, and are visible outside it when `pub`. The receiver is always named `self`, and its kind follows Section 5.3:

```koo
fn (self: Order) describe() -> string { ... }               // borrow
fn (self: var Order) apply_discount(pct: Decimal) { ... }   // mutate: needs a var binding or a temporary
fn (self: take Order) into_invoice() -> Invoice { ... }     // take: consumes self
```

**Constructors** are ordinary type-level functions (`fn Order.new(...) -> Order`, Section 6.1) by convention: `T.new(...)` for values, `T.open(...)` for resources, `T.start(...)` for actors. The compiler does not treat these names specially, except that it generates `start` for actors (Section 7.3).

**Visibility.** A type, function, actor, interface, error or constant is visible outside its package when declared `pub`. Fields of a `pub` type are visible wherever the type is, except fields whose names begin with an underscore, which are visible only inside the defining package. Enum variants are exactly as visible as their enum.

### 4.7 Enums

Enums are sum types. Each variant may carry fields. Variants are separated by newlines or commas.

```koo
pub enum Payment {
    Card { last4: string, brand: string }
    Transfer { iban: string }
    Cash
}

p := Payment.Card { last4: "4242", brand: "visa" }
q: Payment = Cash                    // the expected type is known, so the bare name suffices
```

A variant's full name is `Enum.Variant`, so two enums in one package may both have a `Pending`. The bare name may be used wherever the expected type is already that enum: in a `match` on a value of it, in an argument or return position of that type, on the right of a typed binding, and after `is`. Elsewhere the full name is required.

Every enum has `==` and is `Hashable` when its fields are. An enum whose variants are all field-less is also `Ordered`, by declaration order.

**Open and closed.** Inside its own package, a `match` on an enum must name every variant. Outside it, a `pub` enum is *open*: a `match` must also end with a catch-all arm (`_ => ...`), so that adding a variant is a minor version change that breaks no caller. `@closed` on a `pub` enum promises that its variants are final: importers match it exhaustively without a catch-all, and adding a variant becomes a breaking change (Section 8.5).

An enum or struct may refer to itself through a variant, a `T?` or a collection (`enum Tree { Leaf { value: int }, Node { left: Tree, right: Tree } }`, `type Item = { value: int, next: Item? }`); the compiler places the recursive occurrence on the heap. A type that contains itself with no way to end is a compile error. There is no `Box`. Data with cycles or parent links (graphs, doubly linked structures) is expressed with indexes into a collection that owns the nodes.

### 4.8 Option

`T?` is the absence type; `none` is its empty value. It is handled with the same tools as errors (Section 6.4): `?` to propagate, `match` to inspect, `else` to supply an alternative.

```koo
fn find_user(email: string) -> User?

user := find_user(e) else anonymous_user
name := find_user(e)?.name          // in a function returning T?, propagates none
```

The compiler rejects `user.name` when `user: User?` has not been unwrapped. A `T` is accepted wherever a `T?` is expected (widening, Section 4). In a `match`, the arms are `none` and a pattern for the `T`. `if user := find_user(e) { ... }` runs its block only when a value is present (Section 6.7).

Absence nests, because generic code requires it: `xs.get(i)` on an `[int?]` is an `int??`, in which "no such element" and "the element is `none`" are different values. `none` denotes the outermost absence, and each `?`, `else` or `match` removes one level. Writing a nested optional by hand is almost always a modelling mistake: `koo lint` flags `T??` in a declaration and suggests an enum that names the cases.

### 4.9 Interfaces

An interface names a set of signatures: methods, and type-level functions written `fn Self.name(...)`. A type satisfies an interface by having them (structural, as in Go): the same parameter types, and a return type that is the interface's or widens to it (Section 4), which includes an error set that is smaller. Parameter kinds must agree on `var`. Borrow and `take` satisfy one another, because `take` is an optimization (Section 5.3), except where the parameter is a resource or a function: for those `take` changes what the callee may do, so the kinds must be the same. A declaration may state what its type satisfies, to get a compile-time check and a clearer error message: `pub type Postgres: Database = { ... }`, with `+` between several interfaces. `resource`, `enum` and `actor` declarations may do the same.

```koo
pub interface Database {
    fn query_one(q: sql.Query) -> sql.Row? ! sql.Error
    fn query(q: sql.Query) -> sql.Rows ! sql.Error
}

pub resource interface Writer {                       // files and sockets may satisfy it (below)
    fn (self: var) write(data: [byte]) ! IoError      // a method that changes the receiver says so
}
```

An interface method with no receiver written borrows `self`; `(self: var)` and `(self: take)` declare the other kinds (Section 5.3). Inside an interface, `Self` names the type that satisfies it:

```koo
pub interface Ordered {
    fn compare(other: Self) -> int       // negative, zero or positive
}
```

A user-defined type becomes sortable, and gains `<`, `<=`, `>` and `>=`, by having a `compare` method.

**Derived interfaces.** `Equatable` and `Hashable` are satisfied automatically by any type whose parts satisfy them and cannot be implemented by hand, so that hashing always agrees with `==`. The interfaces behind the derived codecs, such as `json.Encode` and `json.Decode` (Section 10.4), are satisfied automatically too, though a type may write their functions by hand. A generic type satisfies each of these for the type arguments that do: a `Cache<K, V>` is `Hashable` when `K` and `V` are.

**Static and dynamic use.** An interface used as the type of a parameter in a function or method declaration is shorthand for a type parameter: `fn save(w: var Writer)` means `fn save<W: Writer>(w: var W)`, and the call is resolved at compile time like any generic call. Where there is no body to instantiate (a method signature inside an interface, a function type), where one message type is required (an actor method, Section 7.3), or where the parameter is variadic (Section 6.1), the parameter is an interface value instead, and a borrowed or `var` argument is lent to it in that form for the call, without being moved or boxed. An interface used as any other type (a field, a collection element such as `[Database]`, a local, a return type) is an *interface value*: the value together with a vtable. Interface values and stored function values (Section 4.11) are the only two places a call is dispatched at run time. An interface value keeps its dynamic type, which `is` and `match` can test (Section 6.6). An interface that mentions `Self` outside a receiver, or declares a type-level function, states something about a type that a vtable cannot carry: it may be used as a constraint and as a parameter type, not as an interface value.

**Resource interfaces.** `resource interface` declares an interface that resources (Section 4.13) may satisfy. Any type may satisfy it. In exchange, a parameter, type parameter or interface value of that interface follows the rules for resources: it is lent and moved, never copied. `Reader`, `Writer` and `Iterator` are resource interfaces, because files, sockets and the streams read from them are what they exist for. A resource cannot satisfy an ordinary interface.

`Failure` is the interface every error type satisfies, and only error types satisfy it (Section 6.2). `Any` is the interface every copyable type satisfies; it exists for variadic formatting and serialization and is a lint warning outside the standard library.

### 4.10 Generics

Functions and types take type parameters in angle brackets, constrained by interfaces. An **error-set parameter** is a type parameter that appears after `!`; it ranges over error sets.

```koo
fn largest<T: Ordered>(items: [T]) -> T?
pub type Cache<K: Hashable, V> = { entries: [K: V], max: int }
fn with_timeout<T: resource, E>(d: Duration, f: fn() -> T ! E) -> T ! E | Timeout
```

An error-set parameter may be empty; a result type with an empty set is its success type, so `with_timeout` applied to a function that cannot fail returns `T ! Timeout`.

Type arguments are inferred at call sites and cannot be written there (Section 3.9). A parameter may have several constraints: `T: Ordered + Hashable`. Interfaces and actors may take type parameters as functions and types do. A method or type-level function of a generic type names the type's parameters in its receiver: `fn (self: Cache<K, V>) get(key: K) -> V?`, `fn Cache<K, V>.new(max: int) -> Cache<K, V>`.

A type parameter stands for a copyable type: generic code may copy a `T` freely, and a resource (Section 4.13) cannot be its argument. A parameter declared with the constraint `resource` (`T: resource`) accepts every type, resources included; in exchange the generic code treats a `T` as a resource, moving and lending it and never copying it. A parameter constrained by a resource interface (Section 4.9) is of this kind without saying so. The built-in collections, `T?`, tuples, results, `Channel`, `with_timeout` and `isolate` are declared this way; nothing about them is special.

A type parameter may be instantiated with any type, including an optional or a result. Generic code treats the `T` as data. Section 4.8 says what a nested optional means, and Section 6.3 what a nested result means.

There are no higher-kinded types, no associated types, no specialization, no generic methods on interfaces and no variadic generics. The two built-in forms that would need variadic generics, `all` and `race`, are compiler intrinsics (Section 7.2).

### 4.11 Function types

`fn(A, B) -> R ! E` is the type of a function taking `A` and `B` and returning the result type `R ! E`. `fn(A) -> R` has an empty error set. `fn()` takes nothing and returns `()`. The `!` after a function type's return type belongs to the function type; a result whose success is a function is written `(fn() -> R) ! E`, and parentheses around any one type only group. Named functions and lambdas are values of these types. Parameter kinds (Section 5.3) are part of the type: `fn(var [int])` and `fn([int])` are different types.

A lambda or named function written directly as an argument is compiled into the callee's instantiation and inlined like any other code, so `xs.map(x -> x + 1)` costs what the equivalent loop costs. A function value that is stored, in a variable, a field or a collection, is called indirectly.

Function values are copyable. The one exception is `resource fn(A) -> R`, the type of a function value that owns a resource: it is move-only, and calling it consumes it, so it runs at most once (Section 5.7). Every function value widens to a `resource fn` of the same signature (Section 4).

### 4.12 Aliases and distinct types

`type UserId = Id` creates an alias; the two names are interchangeable. `type UserId = distinct Id` creates a distinct type with the same representation, methods and interface implementations as `Id`, which cannot be passed where an `Id` or another distinct type is expected. In inherited signatures, every occurrence of the base type is replaced by the distinct type: `Id.parse(s)` returns `Id ! ParseError`, and `id + 1` requires `Id.from(1)`. Conversion is explicit: `UserId.from(id)`, `int.from(user_id)`. A distinct type may be given further methods in the package that declares it. Distinct types have zero runtime cost and are the recommended way to make identifiers and units type-safe.

### 4.13 Resources

`resource` declares a move-only type: a file, a socket, a database connection, a lock guard.

```koo
pub resource File = { _fd: i32 }
fn (self: take File) close() { ... }
```

A resource value has exactly one owner. Assigning or passing it to a `take` parameter moves it; using the original afterwards is a compile error. Copying a resource is a compile error. Passing it to a borrow or `var` parameter lends it for the call. When the owner's scope ends, the compiler calls `close()` if the type defines it (Section 6.8); calling `close()` explicitly consumes the resource (its receiver is `take`), so it cannot be closed twice. A resource that is moved on some paths and not on others is closed at scope end on exactly the paths that still own it. Resources are the only values for which a move is visible in the language.

A struct, enum variant, tuple, `T?`, result, `[T]` or map value that contains a resource is itself move-only. A `Channel<T>` of resources stays a copyable handle (Section 5.8): `send` moves a resource in and `recv` moves it out. The built-in collections support move-only elements directly: indexing lends the element, `pop` and `remove` move it out, `for x in xs` lends each element in turn, and `for x in take xs` consumes the collection, moving each element into `x`. A resource held in a struct field is lent in place; it leaves the field only when the whole struct is taken apart by a destructuring pattern. A resource may be a type argument only where the parameter is declared `resource` (Section 4.10), and it satisfies only resource interfaces (Section 4.9). A value of a resource interface type and a `resource fn` (Section 4.11) are resources for every rule in this section. An error type may not contain a resource.

## 5. Values, mutability and memory model

Koo has no garbage collector and no programmer-visible pointers or lifetimes. Variables hold values. The compiler decides how values are stored, copied and freed, and guarantees that no program reads freed memory or races on shared memory. A programmer must learn one word, `var`. A second word, `take`, is required to keep a resource or a function passed as a parameter, and is otherwise an optimization.

### 5.1 Value semantics

Assignment, argument passing and return have copy semantics: after `b := a`, changing `b` never changes `a`. For types without heap storage (numbers, booleans, structs of them) this is a literal copy. For types with heap storage (strings, lists, maps, structs containing them) the compiler implements the copy lazily: `a` and `b` share one buffer until one is mutated, at which point the mutating side copies first. The programmer reasons about values; the compiler delivers pointer performance in the common case.

Because a lazy copy becomes a real one at a mutation, "no hidden allocation" is not a property Koo claims. Instead the compiler reports every potential copy: `koo build --show-copies` lists them and the language server shows them inline.

### 5.2 Immutability by default

Bindings are immutable unless declared `var`. A struct's fields may be assigned only through a `var` binding of the struct; the type declaration says nothing about mutability. Collections follow the same rule. Most values are therefore never copied: nothing mutates them.

### 5.3 Parameter kinds

A parameter is one of three kinds:

| Written | Meaning | Caller sees |
| --- | --- | --- |
| `fn f(x: T)` | **Borrow.** The function uses the caller's value in place. | No copy and no reference-count change at the call. |
| `fn f(x: var T)` | **Mutate.** The function may change `x`; the caller's value changes. | The call is written `f(var x)`, so a change is visible where it happens. `x` is a `var` binding, or a field or list element of one. No copy. |
| `fn f(x: take T)` | **Take.** The function will keep `x`: store it, send it or return it. | The argument is moved if this is its last use, otherwise copied (lazily). |

A function may do anything with a borrowed parameter that it could do with a local, with the two exceptions at the end of this section. Uses that only read it cost nothing. A use that needs to keep the value (returning it, storing it in a field, sending it, capturing it in a closure, passing it to a `take` parameter) copies it. For a value with no heap storage, such as an `Id`, that copy is free; otherwise it is the lazy copy of Section 5.1, one reference-count increment, and `--show-copies` lists it.

`take` is how a signature says "I will keep this", so that a caller who is finished with the value can hand it over instead of sharing it. Nothing breaks if it is omitted; `koo lint` suggests it for a parameter that every path copies. Because the kind is in the signature, the compiler decides between move, copy and borrow at each call from the callee's declaration alone, without whole-program analysis.

What a declaration does not say is whether a borrowed argument will be kept. The compiler therefore records one inferred fact for each borrow parameter, *may keep*, in the package's interface file, beside the suspension fact of Section 13.5. Like that one it is never written by the programmer, is not part of the function's type, and affects only how well a caller is optimized (Section 5.6), never whether a program compiles.

Two annotations are never optional. `var` is required to change the caller's value. `take` is required to keep a value that the function is not free to copy, of which there are two kinds. One is a resource (Section 4.13): a resource passed to a borrow parameter is lent for the call, and a function that stores or closes one must take it. The other is a function passed as a parameter, because it may be a lambda working directly on its caller's variables (Section 5.7). A borrowed function parameter may be used in exactly two ways: it may be called, and it may be passed to another borrow parameter whose declared type is a function type. It may not be bound to a variable, stored, returned, sent, passed as a `T` or to a `take` parameter, or captured by a closure.

The same three kinds apply to method receivers (Section 4.6) and are part of a function's type (Section 4.11). A receiver is not marked at the call: in `xs.push(x)` the value being changed is already the first thing on the line. A `var` method may be called on a `var` binding, on a field or element of one, or on a temporary, as in `http.App.new().get(...)`, which changes a value nobody else can see.

### 5.4 Exclusivity

A borrow holds no reference count, so the storage it refers to must not be changed or freed while the borrow is live. Value semantics already says what every program means; exclusivity is the rule by which the compiler keeps that meaning without counting.

**What a call means.** Arguments are evaluated left to right, and each denotes the value its expression had when it was evaluated. A `for` loop iterates over the value its collection had when the loop began.

**What the compiler does.** It treats a borrowed argument, and the subject of a `for` loop, as a *read* of some storage that lasts until the call returns or the loop ends. An argument passed as `var`, a `var` receiver, an assignment and a move are *writes*, and so is a `for` loop over an iterator, which the loop advances (Section 6.7). A lambda passed to a borrow parameter works on the enclosing variables directly (Section 5.7): for the length of that call it is a read of every variable it reads and a write of every variable it assigns. Two accesses overlap when they name the same variable or one names part of the other (a field, an element, a slice).

- **A read that overlaps a write is copied first.** The copy is the lazy one of Section 5.1: one reference-count increment, nothing more unless the write really happens. It appears in `--show-copies`.
- **Two writes that overlap are a compile error.** There is no value-semantics reading of changing one thing through two names at once.
- **A read that cannot be copied is a compile error too.** A resource, or a value that contains one, has no copy to fall back on, so for it any overlap with a write is rejected.

```koo
order.set_note(order.note)        // fine: the argument is the old note, kept alive by a copy
swap(var p.x, var p.y)            // fine: distinct fields do not overlap
swap(var xs[i], var xs[j])        // error: the elements may be the same one; use xs.swap(i, j)
xs.each(x -> total += x)          // fine: the lambda writes total, and nothing else in the call touches it

for x in xs {
    if x.stale { xs.remove(x) }   // fine: the loop walks xs as it was; lint notes the snapshot
}
```

Closures that are kept, and tasks, need no rule: they capture by value (Section 5.7), so they never hold a name for the caller's storage.

There are no references in the language, so two names can reach the same storage only within one statement or one loop, and never across a function boundary except through that call's own arguments. The check is therefore local to a function body and uses nothing but signatures. It establishes what Rust's borrow checker and Swift's exclusivity enforcement establish, with no lifetime annotations and no runtime checks, and the only programs it rejects are those with two overlapping writes, or with an overlap on a value that cannot be copied.

### 5.5 Moves

When a value is passed to a `take` parameter, assigned to another binding, or returned, and the compiler can see it is not used again, it is moved rather than copied. This is inferred for ordinary values and mandatory for resources (Section 4.13). `xs = sort(xs)`, where `sort` takes `xs: take [T]`, performs no copy.

### 5.6 Allocation, deallocation and sharing

Values are stack-allocated when their size is known at compile time and they do not escape the function; otherwise heap-allocated. Heap memory is freed when the last owner's scope ends, as in Rust and C++ RAII. Values contain values, never references to one another, so no cycle of values can exist and reference counting never leaks.

A heap buffer is in one of two states:

- **Owned.** Reachable from bindings in a single task. If more than one binding shares it (Section 5.1), a non-atomic reference count tracks them. The compiler elides the count when it can prove a single owner, which requires that every function the value is lent to is known not to keep it (Section 5.3).
- **Frozen.** Reachable from more than one task. A frozen buffer is immutable and uses an atomic reference count. A mutation through any binding copies first, and the copy is owned. The one exception is a frozen buffer whose count is one: its holder is the only task that can reach it, so it is changed in place.

**Freezing is deep.** A value is frozen when it becomes reachable from a second task: it is sent (Section 5.8), captured by a task, or placed in a `Shared`, while anything in the sending task still shares one of its buffers. Freezing marks that buffer and every buffer reachable from it, because the other task will count references to the inner buffers too. The walk visits each buffer once and stops at one that is already frozen, so a large table shared with a thousand tasks is walked once, not a thousand times.

The compiler omits the walk for types with no heap storage and for values it proves uniquely owned throughout, which is the usual case for a message built and sent. `koo build --show-copies` reports every send that walks.

Top-level constants (Section 3.5) are statically allocated and never counted.

### 5.7 Closures

A lambda is one of two things, and the parameter it is passed to says which.

- **A lambda passed to a borrow parameter is a block.** It cannot outlive the call, so it uses the enclosing function's variables directly, as the body of a `for` loop does: it may read them, assign the `var` ones and lend resources. `xs.each(x -> total += x)` and `db.transaction(tx -> { ... })` do what they appear to do. Exclusivity (Section 5.4) treats the lambda's reads and writes as accesses that last for the call.
- **Every other lambda is a closure.** A lambda passed to a `take` parameter, stored in a variable, field or collection, or returned, captures the variables it uses by value at the moment it is created. Reading a captured variable is always allowed. Assigning to a captured `var` is a compile error, because the closure holds a copy and the assignment would change nothing the enclosing function can see; the error message says so and suggests returning the new value or using an actor.

The distinction is read off the callee's signature and nothing else: the lambda is a block when the parameter's declared type is a function type and its kind is not `take`. A variadic, optional or generic parameter does not count, and a lambda passed to one is a closure. This is why a borrowed function parameter cannot be kept (Section 5.3).

A closure that captures a resource takes ownership of it. Such a closure is a `resource fn` (Section 4.11): it is move-only and calling it consumes it, so its body may consume what it captured, for example by returning it. It can be passed only where a `resource fn` is expected, which `isolate` does (Section 5.9).

`spawn` blocks and the argument expressions of `all` and `race` are closures of the `resource fn` kind: they capture by value, run once, and may consume what they capture. The bodies of `if`, `for`, `while`, `match`, `scope` and `defer` are plain blocks and use the enclosing variables directly.

### 5.8 Handles and sharing across tasks

Values are never mutated by two tasks. Tasks communicate by message (to an actor or through a channel) or by snapshot (`Shared`), and the parameter that carries the value is `take`. When the value is uniquely owned and at its last use, it moves and nothing is shared. Otherwise it is frozen (Section 5.6): both tasks may read it, and the first to change it copies. Data races are therefore impossible by construction and the compiler needs no `Send`/`Sync` analysis.

This is also how large immutable data (a configuration, a lookup table) is shared by many tasks without copying: send it to each, or put it in a `Shared`; it freezes once.

**Handles.** A handle is a small copyable value that refers to an object owned by the runtime: an actor, a `Channel`, a `Shared`, a `sql.Pool`, an HTTP client. Copying a handle copies the reference, not the object, and every handle may be used from any task; the object behind it does its own synchronization. Handles are the only values in safe Koo with reference semantics, and the only way to form a cycle; a cycle of handles keeps runtime objects alive until the program stops them and is never unsafe. A program defines a new kind of handle by declaring an actor. A library may also build one in `unsafe`, as `std` does for the connection pool, and is then responsible for its thread safety.

### 5.9 Panics and isolation

A panic reports a fault in the program: integer overflow, division by zero, an out-of-range index, an explicit `panic("msg")`, an ask cycle, a task exceeding its depth limit. It is not for conditions a correct program expects; library code must not panic on bad input, it returns an error.

A panic unwinds the current task, running cleanup (Section 6.8). In the task where it happens it is not a value, cannot be matched and does not appear in error sets. Where it goes next depends on what started the task:

- **`spawn`, `map_parallel`, `all`, `race`:** the enclosing scope or call cancels its other tasks and the panic continues in the task that owns it.
- **An actor:** the actor's supervisor handles it (Section 7.4); the asker whose message caused it, if any, panics.
- **`isolate`:** the panic stops there and becomes an error.
- **The main task:** the process prints the panic and exits with status 101.

```koo
fn isolate<T: resource, E>(f: take resource fn() -> T ! E) -> T ! E | Panic
// Panic { message: string, stack: string }
```

`isolate` is sound in Koo for a reason that does not hold in most languages. The task it starts captured its inputs by value and can affect the rest of the program only through whole messages and whole snapshots, so a task that dies half-way through leaves nothing in memory half-changed behind it, and its resources have been closed. There is no lock to poison and no shared structure to corrupt. Effects outside the process are the program's concern as always: a transaction is rolled back by its cleanup (Section 10.3), a half-written file is not.

A panic raised by cleanup code while its task is already unwinding does not start a second unwind: it is recorded in the first panic's report and the remaining cleanup runs.

Isolation belongs at the boundary of a unit of work and nowhere else: `std/http` and `std/grpc` isolate each request (Section 10.3), and a queue consumer should isolate each job. One request that divides by zero costs one 500 response.

Bounds and overflow checks are elided when the compiler proves them unnecessary.

### 5.10 What the programmer never writes

References, lifetimes, borrows, `Box`, `Rc`, `Arc`, `Mutex`, `Cell`, `Pin`, `Send`, `Sync`, raw pointers outside `unsafe`. A Koo program with no `unsafe` block cannot exhibit a use-after-free, double free, buffer overflow, null dereference or data race.

### 5.11 Cost model

Koo matches Rust where the two do the same thing: monomorphized generics, structs laid out inline with no headers, static dispatch, stack allocation, no garbage collector, and LLVM. It differs in the following places, and a specification that claims Rust-class performance owes the reader the list.

| Cost | Paid when | Kept small by |
| --- | --- | --- |
| Reference-count increment and decrement | A heap value is copied and both copies stay live, or a read overlaps a write (Section 5.4) | Borrow parameters, moves at last use, elision; non-atomic unless frozen |
| Uniqueness check | A heap value is changed in place | One load and branch; hoisted out of loops |
| Freeze walk | A value that is still shared is sent to another task | Done once per value; omitted for uniquely owned messages |
| Overflow and bounds checks | Integer arithmetic; indexing | Range analysis; `for` loops over collections have none; `wrapping_` methods |
| Cancellation check | Every loop iteration | One load and a predictable branch |
| Frame allocation | A suspending function is called recursively or indirectly | Direct call chains share one allocation (Section 13.5) |
| Shared buffers instead of borrowed views | A substring or slice is stored in a data structure | A count instead of a lifetime; `detach()` when the parent is large |

The design bets that for services, whose time goes to I/O, parsing and allocation, these add up to less than ten percent. Section 13.6 defines the measurements that test the bet, and `koo build --show-copies` shows every copy, count and freeze walk the compiler could not remove.

## 6. Functions, errors and control flow

A function declares what it returns and what can go wrong. The caller must handle or pass along everything that can go wrong. There are no exceptions, no error codes and no null.

### 6.1 Functions

```koo
fn get_order(db: sql.Pool, id: Id) -> Order ! NotFound | sql.Error { ... }
fn log_all(level: int, messages: ...string) { ... }
fn Order.new(customer: Id) -> Order { ... }          // type-level function, called as Order.new(c)
fn (self: Order) describe() -> string { ... }        // method (Section 4.6)
fn main() ! sql.Error { ... }
```

Parameters are typed. The return type follows `->`; a function with no `->` returns `()`, the empty tuple (Section 4.5). An error set follows `!`, with or without a return type.

`return expr` ends the function with a success value. **`fail expr`** ends it with an error, which must be in the function's error set. The two are different statements so that a reader, and the compiler, never has to infer which one a value means.

A **variadic** parameter `name: ...T` must be last and collects zero or more arguments into a `[T]`. When `T` is an interface the elements are interface values (Section 4.9), so the arguments may be of different types. Parameters may have defaults (`port: int = 5432`), which are constants; any argument may be passed by name (`connect(host, port: 5432)`), and after the first named argument all must be named. There is no overloading: one name, one function. The built-in conversions of Section 4.2 are the single exception. A type-level function is declared in the package that declares the type.

`main` is `fn main()` or `fn main() ! E`. An error returned from `main` is printed and the process exits with status 1.

### 6.2 Error types

```koo
error NotFound { id: Id }
error Conflict
error StorageError { cause: Failure }
error OrderError = NotFound | sql.Error      // a named error set
```

An error type is a struct declared with `error`; it may not contain a resource. Every error type, and no other type, satisfies the `Failure` interface, which has `to_string()` and `cause() -> Failure?`; the compiler provides both, printing the type's name and fields and returning the field named `cause` if there is one. `error Name = A | B` names an error set; the name may be used wherever a set is written and is interchangeable with its expansion. Error values carry no stack trace by default; `koo build --traces` attaches one.

### 6.3 Results and error sets

`T ! E1 | E2` is a **result type**: a value that is either a `T` or one of the listed errors. A call to `get_order` produces a value of type `Order ! NotFound | sql.Error`.

A result is an ordinary value. It can be bound, stored in a field or collection (`[Order ! NotFound]`), sent through a channel and passed to a parameter. What it cannot be is ignored:

- A result may not be discarded: an expression statement whose value is a result, and a binding of result type that is never consumed or passed on, are compile errors.
- A result has no fields, methods or operators of its success type. The only way to reach the `T` is `?`, `else` or `match` (Section 6.4). `wrap` (Section 6.5) changes a result's errors and leaves it a result.
- `return e` takes a success value, or a result with the function's success type whose errors are all in the function's set, which it passes through unchanged. `return get_order(db, id)` and `return get_order(db, id)?` therefore mean the same.

The check is on expressions and local bindings whose type is a result in the code as written, before any type parameter is replaced. A result placed in a data structure is data, and what becomes of it is the program's business; so is a result that generic code holds as a `T`.

A function's success type may contain results (`[Order ! NotFound]`), but a declaration may not write a result of a result. Generic instantiation can still produce one, as it produces nested optionals (Section 4.8): a function declared to return `T ! Closed`, used with `T` as `Order ! NotFound`, returns `(Order ! NotFound) ! Closed`, and each `?`, `else` or `match` removes the outer layer. Where a signature has an error-set parameter, as in `fn() -> T ! E`, inference always puts a function's errors in `E` and never in `T`.

**Writing the set.** The author writes the set. The compiler checks that it is complete (every error the body can produce is listed) and warns when a listed error cannot occur. An unexported function may write `-> T !` to have its set inferred; exported functions must spell it out. Lambdas always infer.

**`Failure` as a set.** A function may declare `! Failure`, meaning any error; every error propagates into it with `?`. It is intended for application code, such as request handlers and `main`, where errors are logged or turned into responses rather than matched. `koo publish` warns on a `pub` function that declares it, because callers can no longer match exhaustively.

**Evolving a set.** An error set is part of a function's signature. Adding an error type to a `pub` function's set is a breaking change, because callers that list their own sets must now list it too; removing one is a minor change. A package that expects its failures to grow therefore exposes one error type carrying an open enum (Section 4.7) that says what went wrong, as `std` does: `sql.Error { kind: sql.ErrorKind, message: string, ... }`. New kinds are then minor changes, and callers match on `e.kind` with a catch-all.

### 6.4 Consuming a result

There are three ways, and `?` and `else` also apply to `T?`.

**Propagate with `?`.** If the value is an error, fail the current function with it; the function's error set must include it. If the value is `none` and the current function returns `T?`, return `none`. If the value is `none` and the current function does not return `T?`, it is a compile error: use `else`. `?` removes one layer: applied to a `Row? ! sql.Error` it propagates the error and yields a `Row?`. Inside a lambda, `?` leaves the lambda, not the function around it.

```koo
order := get_order(db, id)?
```

**Inspect with `match`.** Arms for the success value are ordinary patterns. An arm for an error begins with `!`, as the error set does in the type, so a success and an error can never be confused, whatever their types. The `!` applies to the whole arm, so `! NotFound | Conflict` matches either error. Every outcome must be covered; `! e` covers every remaining error. An arm for an error that is not in the set is a warning, not an error, so that a library may stop returning an error without breaking its callers (Section 8.5).

```koo
match get_order(db, id) {
    order            => respond(order)
    ! NotFound       => respond_404()
    ! e: sql.Error   => {
        log.error("db failure", e)
        respond_503()
    }
}
```

**Fall back with `else`.** If the value is an error or `none`, evaluate the alternative instead. The alternative is an expression of the success type, or a block or statement that ends in `return`, `fail`, `break`, `continue` or `panic`. `else name => alternative` binds the error first; its type is the error type when the set has one member and `Failure` otherwise.

```koo
count := int.parse(s) else 0
row := db.query_one(q)? else fail NotFound { id }       // ? handles sql.Error; else handles none
user := load_user(id) else e => {
    log.warn("user lookup failed", e)
    fail Unavailable { cause: e }
}
```

`else` is the same keyword as in `if`/`else` and means the same thing. A plain `else` hides which error occurred, which is right for `int.parse(s) else 0` and wrong for a database call; `koo lint` flags a plain `else` on a result whose set has more than one member.

A result that is neither propagated, matched, given an alternative nor passed on is a compile error: `unhandled result from get_order: NotFound, sql.Error`.

### 6.5 Converting errors

To report a lower-level error as a higher-level one, `wrap` it before propagating. `wrap` takes a function from `Failure` to the new error type and applies it to any error in the result:

```koo
rows := db.query(q).wrap(e -> StorageError { cause: e })?
```

`wrap` is defined on every result and returns a result with the same success type and the new error as its set. Error chains are preserved through `cause()` and printed by the default logger.

### 6.6 Pattern matching and type tests

`match` works on any value and may destructure, bind and guard:

```koo
match payment {
    Card { brand: "amex", last4 }               => amex_fee(last4)
    Card { .. }                                  => card_fee()
    Transfer { iban } if iban.starts_with("DE")  => sepa()
    Transfer | Cash                              => no_fee()
}
```

- `Name { field, field: pattern, .. }` destructures a struct, error or enum variant. Without `..` the pattern must name every field, and a pattern on a struct or variant declared in another package must end with `..`, so that a field added there breaks no caller. A bare `Name` matches any value of that type or variant whatever its fields.
- `name` matches anything and binds it; `name: Type` matches a value of that type and binds it. Because case is enforced (Section 3.1), a lowercase name in a pattern is always a binding and a capitalized one is always a type or variant.
- `none` matches absence. `_` matches anything. `a | b` matches either; both sides must bind the same names.
- `! pattern` matches an error in a result (Section 6.4).
- Literals, ranges (`1..=9`) and tuples (`(0, y)`) are patterns. `if cond` after a pattern is a guard.
- Variant names are written bare in patterns because the scrutinee's type is known (Section 4.7).

A name bound by a pattern holds a copy of the part it matched (a lazy one, Section 5.1), so the matched value may be changed inside the arm. Matching a value that contains a resource consumes it.

`match` must be exhaustive, with the catch-all that Section 4.7 requires for another package's open enum. A `match` on a result must cover the success value and every error in the set. **Values.** A block of statements never has a value in Koo. `match` and `if`/`else` may still be used for their value. Every arm or branch is then a single expression (for `if`, the one expression inside its braces: `larger := if a > b { a } else { b }`), and all of them have the same type. An arm or branch that ends by leaving, with `return`, `fail`, `break`, `continue` or `panic`, has any type and may be a block. A computation that needs statements to produce a value is a function. Used as a statement, `match` takes in each arm a block or one simple statement (an expression, an assignment, `return`, `fail`, `break` or `continue`), `if` takes blocks, and any value is discarded.

`x is T` is a `bool` that tests whether `x`'s variant or dynamic type is `T`; it binds nothing and takes no braces. To bind, use `match`.

### 6.7 Conditionals and loops

`if`/`else if`/`else`, no parentheses. `while cond`. `break` and `continue`, optionally labeled (Section 3.9). No `do-while`, no `goto`, no three-clause `for`.

**Binding conditions.** `if name := expr { ... }` evaluates `expr`, which must be a `T?`, and runs the block with `name` bound to the value when there is one; an `else` branch runs otherwise. `while name := expr { ... }` repeats for as long as there is one. As in any `:=`, a tuple pattern or `_` may stand in place of the name.

```koo
if user := find_user(email) { greet(user) }

while row := rows.next()? {        // a stream whose every step can fail: the ? handles each step
    total += row.int("amount")?
}
```

`for x in e` iterates over:

| `e` | `x` |
| --- | --- |
| `[T]`, `[T; N]`, `Set<T>` | each element, in order |
| `[K: V]` | each `(k, v)`, in insertion order: `for (k, v) in m` |
| `a..b`, `a..=b` | each integer |
| `Channel<T>` | each received value, until the channel is closed and drained |
| any value with a `next` method | each value until `none` |

The last row is the iteration protocol. It is the prelude interface

```koo
pub resource interface Iterator<T> {
    fn (self: var) next() -> T?
}
```

and any type with that method can be looped over. The loop advances the iterator, so `e` is a `var` binding or a temporary. The loop is a write of that binding for as long as it runs (Section 5.4), and when it is left early the iterator stays where the loop stopped. Lazy sequences (`s.chars()`, `s.lines()`, `xs.chunks(n)`) are iterators. A stream whose `next` can fail, such as `sql.Rows` or a network reader, is not an `Iterator`; it is consumed with `while row := rows.next()?`, as above, so that each step's failure is handled by the ordinary means and `for` needs no special case.

A loop iterates over the value its collection had when the loop began (Section 5.4). A body that changes the collection does not disturb the iteration: the loop continues over the original elements, at the cost of the copy that the first change makes. `koo lint` points this out, because a loop that neither needs nor wants the snapshot is usually clearer as two steps. A collection of resources cannot be copied, so a loop over one lends its elements and its body may not change the collection.

### 6.8 Cleanup

There are two cleanup mechanisms, one for types and one for blocks.

**Resources** (Section 4.13) are closed when their owning scope ends, by any path. If a resource's `close` has an error set, the program must call it (`file.close()?`) before the scope ends normally, that is by reaching its end, `return`, `break` or `continue`, because a failed close is information: letting such a resource fall out of scope on one of those paths is a compile error. When the scope is left by `fail` or `?`, by a panic or by cancellation, a failure is already on its way out and there is one channel for it: the compiler closes the resource, and a failure of that close is logged. The requirement is on a binding whose own type is the resource. A resource held inside another value, whether a collection, a struct, an interface value or a generic `T`, is closed when that value is dropped, and a failure of that close is logged as well.

**`defer { ... }`** runs its block when the enclosing block exits, by any path: falling off the end, `return`, `fail`, `break`, `continue`, a panic or cancellation. Deferred blocks and resource closes run together in reverse order of creation. `defer` is for cleanup that does not merit a type: decrementing a gauge, removing a temporary file.

```koo
in_flight.send.add(1)
defer { in_flight.send.add(-1) }
```

A deferred block is a block, not a closure: it may read and assign the enclosing variables. It may not leave itself with `return`, `fail`, `break`, `continue` or `?`; errors inside it are handled inside it.

Both mechanisms run inside the cleanup window of Section 7.7, so cleanup may perform I/O even when the task is being cancelled. There is no `try`/`finally` and no `with`.

## 7. Actors and concurrency

Koo's concurrency has three layers, each built on the one below. Most code uses the first; shared state uses the second; pipelines and protocols use the third. All run on lightweight tasks scheduled by the runtime; none uses `async`, `await`, locks or atomics.

| Layer | Construct | Use when |
| --- | --- | --- |
| 1. Parallel data | `map_parallel`, `all`, `race`, `with_timeout`, `isolate` | Fan out independent work and collect results |
| 2. Shared state | `actor` (ask, send); `Shared` | State that many tasks change; state that many tasks read |
| 3. Tasks and channels | `scope`, `spawn`, `Channel`, `select` | Pipelines, protocols, long-lived workers |

### 7.1 Tasks and suspension

A task is a lightweight thread of execution. Starting one costs one small allocation, and a service may run millions. The runtime multiplexes tasks over worker threads, one per core by default. A call that would wait (a socket read, a database query, a sleep, a channel receive, an ask) **suspends** the task and lets its worker run another; the code looks and behaves like ordinary blocking code, and "blocks" elsewhere in this document means exactly this.

There is no `async` keyword and no `await`. A function that performs I/O has the same signature as one that does not. The compiler works out which functions can suspend and compiles them accordingly (Section 13.5); the programmer never sees the distinction, and it is identical on native targets and WebAssembly.

**Fairness.** A task that computes without suspending keeps its worker thread. The runtime keeps every other task running by adding a worker when one has been occupied for more than 10 ms, so a long computation costs a thread, not the service's responsiveness. The number of workers is capped (`runtime.max_workers`, four per core by default); beyond the cap, runnable tasks wait. On WebAssembly there is one thread: a long computation there should call `task.yield()` periodically.

**Depth.** A task's call depth is bounded in two ways. The frames of functions that suspend count against a memory limit, 1 GB by default (`runtime.max_task_memory`). Functions that do not suspend run on the worker thread's stack, whose remaining space is checked (Section 13.5). Exceeding either panics (Section 5.9). WebAssembly differs, as Section 9.5 states.

### 7.2 Layer 1: parallel collections and intrinsics

```koo
orders := ids.map_parallel(id -> get_order(db, id))?         // [Order] ! NotFound | sql.Error
(user, orders, prefs) := all(get_user(id), get_orders(id), get_prefs(id))?
first := race(fetch_primary(), fetch_replica())?
order := with_timeout(5s, () -> get_order(db, id))?          // Order ! NotFound | sql.Error | Timeout
```

- `xs.map_parallel(f)` runs `f` on every element in its own task and returns the results in element order. Concurrency is capped at `limit:`, 64 by default: `xs.map_parallel(f, limit: 8)`. Its error set is `f`'s; the first failure cancels the remaining tasks and is returned.
- `xs.map_parallel_settled(f)` does the same but never cancels: it returns every outcome as a `[U ! E]` (Section 6.3), for batch endpoints that report partial success.
- `all(e1, ..., en)` runs each argument expression in its own task and returns a tuple of their results. Its error set is the union of theirs; the first failure cancels the rest and is returned.
- `race(e1, ..., en)` runs each argument expression in its own task and returns the first **success**; the rest are cancelled. If every argument fails, `race` fails with the error of the last to fail. All arguments must have the same success type; the error set is the union. This is the shape of a hedged request: a replica that refuses the connection at once does not beat a primary that answers.
- `with_timeout(d, f)` runs `f` in the current task with the deadline narrowed to `d` (Section 7.7). `f` is a block lambda (Section 5.7), so it works on the caller's variables and on resources the caller holds. If the deadline passes first, the code inside `f` unwinds to the `with_timeout` call, running its cleanup, and the call fails with `Timeout`; what `f` had already done stays done, as when a function fails part-way. This is the only way to put a deadline on code; the `after` arm of `select` (Section 7.6) is a wait, not a deadline.
- `isolate(f)` runs `f` in a task and turns a panic in it into the error `Panic` (Section 5.9). `f` is a closure: a resource it captures moves into the task, and comes back only if `f` returns it.

No task started by any of these outlives the call. Whatever runs in another task is a closure: the lambdas of `map_parallel` and `isolate` and the arguments of `all` and `race` capture by value (Section 5.7). `with_timeout` starts no task.

`all` and `race` are **intrinsics**: their argument expressions are not evaluated before the call, each becomes a task, and `all` returns a tuple whose shape depends on the argument count. They accept two to eight arguments. They are the only call-like forms with these properties and exist because variadic generics are excluded (Section 4.10).

### 7.3 Layer 2: actors

An actor owns state and processes one message at a time. It is the way to hold state that many tasks change.

```koo
@mailbox(4096)
pub actor RateLimiter {
    var hits: [string: int] = [:]
    limit: int

    fn allow(key: string) -> bool {
        n := (hits[key] else 0) + 1
        hits[key] = n
        return n <= limit
    }

    fn reset() {
        hits = [:]
    }
}

fn main() {
    limiter := RateLimiter.start(limit: 100)
    if limiter.allow("10.0.0.1") { ... }     // ask: waits for the reply
    limiter.send.reset()                      // send: fire and forget
}
```

- **Fields** follow Section 5.2: `var` fields may be assigned by the actor's methods; others may not. `T.start(...)` takes every field without a default as a named argument and returns a handle. Actors may be generic: `actor Cache<K: Hashable, V> { ... }`.
- **Handles** (Section 5.8) are copyable values that refer to the running actor. Copying a handle does not copy the actor. A handle satisfies any interface whose methods the actor has (the implicit `take` of its parameters satisfies a borrow, except for a resource or function parameter, which the interface must also declare `take`; Section 4.9), so an actor can be replaced by a fake in tests.
- **Methods.** Every parameter of an actor method is implicitly `take` (its value moves into the mailbox); writing `var` or `take` on one is a compile error. A parameter of interface type is an interface value here, not a type parameter, because a mailbox holds messages of one type. Fields are used by bare name, and a parameter or local may not reuse a field's name. Inside a method, `self` is the actor's own handle. Methods are visible outside the package when `pub`, as a struct's are.
- **Ask.** `handle.method(args)` sends a message and suspends the caller until the reply. Its type is the method's return type, error set included.
- **Send.** `handle.send.method(args)` sends a message and continues. It is permitted only for methods that return `()` and have no error set, so nothing is discarded. `handle.try_send.method(args)` returns `false` instead of suspending when the mailbox is full.
- **Mailboxes** are bounded, 1,024 messages by default, set by `@mailbox(n)`. A send or ask to a full mailbox suspends the sender. Backpressure is automatic. The one sender that cannot wait is the actor itself, inside one of its own methods: only it could make room, so a `send` to `self` that finds the mailbox full panics.
- **One message at a time.** Methods need no locks and cannot race. Different actors run in parallel. An actor on a request's hot path is a queue every request waits in: for state that is mostly read, use `Shared` (Section 7.5).
- **Calls within the actor.** A method calls another method of the same actor by bare name, as an ordinary call within the same message. `send`, `try_send` and `stop` are reserved and cannot be method names.
- **Lifetime.** An actor runs until `handle.stop()` is called or `main` returns. Stopping finishes the message in progress, cancels the actor's tasks and waits for them, discards queued messages (their askers panic with `ActorStopped`), and then runs cleanup for the fields. When `main` returns, the runtime stops every remaining actor in reverse order of start. An actor is not stopped because its handles were dropped; an unreferenced actor with an empty mailbox costs memory only.
- **Cycles.** Every ask carries its *ask chain* in the task context: the actors whose methods are waiting on it, directly or through tasks they started. An ask to an actor already in the chain would wait forever; the runtime panics the asker with a message naming the chain. This catches cycles of any length, including an actor asking itself. Use `send` on one side. A task started with `spawn` in an actor method (below) begins with an empty chain, because no method is waiting for it.
- **Cost.** Target: an idle actor occupies under 512 bytes plus its fields.

**Background work.** A method that waits on I/O stops the actor from processing other messages until it returns; `koo lint` warns about it. Instead, `spawn { ... }` inside an actor method starts a task owned by the actor: the method returns at once, the task captures by value like any `spawn` block, and it reports back by sending a message to `self`.

```koo
fn refresh(url: string) {
    spawn { self.send.refreshed(url, http.get(url)) }    // the result, error or not, is the message
}
fn refreshed(url: string, result: http.Response ! http.Error) { ... }
```

A task started this way has nowhere to propagate an error, so its block must have an empty error set. A panic in it is treated as a panic of the actor (Section 7.4).

### 7.4 Supervision

If an actor method panics, the panic is contained to the actor. The asker whose message was being processed, if any, panics in turn (Section 5.9); askers of other queued messages are unaffected. The actor's tasks are cancelled, and its supervision policy, set by `@supervise`, applies:

- **`restart(max: n, within: d)`**: a new instance is created from the same `start` arguments, of which the runtime keeps a copy (a lazy one, Section 5.1), with default field values otherwise. The failing message is dropped and logged; messages queued behind it are processed by the new instance. When more than `n` restarts occur within `d`, the policy becomes `stop`. The default policy is `restart(max: 5, within: 1min)`.
- **`stop`**: the actor stays stopped. Later asks panic with `ActorStopped`; later sends are dropped and logged.

A restart discards the actor's state. State that must survive a fault belongs in a database, or is rebuilt by the actor from one. An actor whose `start` arguments include a resource cannot be recreated from them: it must be declared `@supervise(stop)`, or take a function that opens the resource instead of the resource itself. Anything else is a compile error.

### 7.5 Shared snapshots

An actor answers one message at a time, which is wrong for state that every request reads and something occasionally replaces: configuration, a routing table, feature flags, a cache snapshot. `Shared<T>` is a handle (Section 5.8) to a cell holding one frozen value.

```koo
flags := Shared<Flags>.new(load_flags()?)

if flags.get().new_checkout { ... }       // any task, any core, no waiting

flags.set(load_flags()?)                   // replace the snapshot
flags.update(f -> f.with_rollout(50))      // derive a new snapshot from the current one
```

- `get() -> T` returns the current value. It never suspends and never copies data: the reader receives a reference to the frozen value (Section 5.6) and keeps a consistent snapshot however many times the cell is replaced afterwards.
- `set(v: take T)` replaces the value. `update(f: fn(T) -> T)` applies `f` to the current value and installs the result; if another writer replaced the value meanwhile, `f` runs again on the new one, so `f` must be free of side effects.
- There is no in-place mutation. A reader that assigns to its copy changes only its copy (Section 5.1).

Use `Shared` when reads dominate and a write replaces the value. Use an actor when each update must see the previous one exactly once (a counter, a ledger, a rate limiter) or when the state's owner performs I/O.

### 7.6 Layer 3: scopes, tasks, channels and select

For work that is not "fan out and collect" or "ask an owner": streaming pipelines, protocol handlers, background loops.

```koo
fn process(input: Channel<Event>) -> Summary {
    stats := Channel<Stat>.new(capacity: 1000)
    var summary := Summary.new()

    scope {
        spawn {
            scope {
                for _ in 0..8 {
                    spawn { for event in input { stats.send(handle(event)) } }
                }
            }                 // the eight workers have finished
            stats.close()
        }
        for s in stats { summary.add(s) }   // the scope's own block runs in the current task
    }
    return summary
}
```

- **`scope { ... }`** is a block that runs in the current task and may use and assign the enclosing function's variables like any other block. **`spawn { ... }`** inside it starts a task running the inner block. The `scope` does not complete until every task it started has finished, so tasks cannot leak. `scope` is not a value and cannot be stored or returned. `spawn` is permitted in exactly two places: inside a `scope`, and inside an actor method (Section 7.3).
- **A `spawn` block is a closure** (Section 5.7): it captures the variables it uses by value when the task starts, taking ownership of any resource among them, and cannot assign the enclosing function's variables. Results leave a task through a channel, an actor or `Shared`. `return`, `break` and `continue` that would leave a `spawn` block are compile errors.
- **Failure.** `?` and `fail` inside a `spawn` block end that task and fail the scope: the scope cancels its other tasks, and its own block if that is still running, waits for them, and the error propagates from the `scope` block, whose enclosing function must list it. A panic in a task cancels the others and re-raises from the `scope` block (Section 5.9). `return` or `fail` in the scope's own block cancels its tasks, waits for them, then leaves.
- **`Channel<T>`** is a handle to a typed, bounded queue; `capacity:` is required. `send(x: take T)` suspends when the queue is full. `recv()` returns `T?`, `none` once the channel is closed and drained; `for x in ch` receives until then. `try_send` and `try_recv` never suspend. A channel has any number of senders and receivers. `close()` may be called once; a later `send` panics. The runtime does not track senders, so closing is the program's responsibility, as in Go. `Channel<()>` is the type of a pure signal.
- **`select`** waits on several channel operations or a timeout and runs exactly one arm; if several are ready, one is chosen at random:

```koo
select {
    msg := inbox.recv()     => handle(msg)
    outbox.send(pending)    => pending = next()
    _ := shutdown.recv()    => break
    after 30s               => heartbeat()
}
```

The operands of every arm are evaluated before the wait, and a value offered to `send` is moved only if that arm runs. An arm's body is a block or one simple statement, as in `match` (Section 6.6).

### 7.7 Cancellation and context

Cancellation is not an error value. When a task is cancelled (its scope failed, a `race` sibling succeeded, its deadline passed, or the request it serves was abandoned by the client), it unwinds, running cleanup, as a panic does. Ordinary code cannot observe, catch or suppress cancellation; it does not appear in error sets and `else` cannot swallow it. Two constructs bound it. A `with_timeout` whose own deadline passed stops the unwinding at its call and fails with `Timeout` (Section 7.2). A `scope` one of whose tasks failed stops the unwinding of its own block and fails with that task's error (Section 7.6). Cancellation that comes from further out passes through both. Cancellation takes effect at the task's next suspension point, loop iteration or recursive call, whichever comes first; the straight-line code between two such points always runs to completion.

**Cleanup window.** Cleanup code, meaning a resource's `close()` and the body of a `defer` (Section 6.8), runs with cancellation held off, so it may perform I/O: roll back a transaction, send a closing frame, flush a buffer. All the cleanup of one unwinding shares one deadline, 5 s unless `runtime.cleanup_grace` in `koo.toml` says otherwise. When it expires, the remaining cleanup still runs but any further suspension in it fails immediately, and the runtime logs what was cut short.

**Actors are not interrupted.** A method that an actor has begun always runs to completion or panics; it is never cancelled part-way, because that would leave the actor's state half-changed. If the asker is cancelled or its deadline passes, the asker stops waiting and the reply is discarded. A message still in the mailbox when its asker is cancelled is dropped unprocessed. An actor method carries the asker's trace record but not its deadline. A `with_timeout` inside a method limits what is inside it and nothing else.

**Context.** Every task carries a context: a deadline, a cancellation signal, a trace record and an ask chain (Section 7.3). A spawned task inherits its parent's context; `with_timeout` narrows the deadline. A task spawned by an actor method (Section 7.3) inherits the trace record only: it has no deadline and an empty ask chain. `std/http` sets each request's context (Section 10.3); `std/sql`, the HTTP client and `std/grpc` read it to bound every operation and to propagate trace identifiers. User code reads it through `task.context()`, which offers `deadline() -> Time?`, `remaining() -> Duration?` and `trace_id() -> string`. There is no other task-local storage.

### 7.8 What is deliberately absent

No mutexes, read-write locks, condition variables, atomics or thread-local storage in safe code. No `async`/`await`, futures, promises or executors. No raw OS threads. Each is either unnecessary given values, actors and `Shared`, or available inside `unsafe` (Section 9.4) for driver and library authors.

## 8. Modules and packages

A directory is a package; the files in it share one namespace. Packages depend on other packages by name and version, recorded in a lockfile with content hashes. There is one registry and one tool.

### 8.1 Packages and files

Every `.koo` file in a directory belongs to that directory's package. Declarations are visible to every other file in the package without imports. A declaration is visible outside the package only when `pub` (Section 4.6). Subdirectories are separate packages.

```
my-service/
  koo.toml          // name, version, dependencies
  koo.lock          // exact resolved versions and hashes
  main.koo          // package root
  orders/           // package my-service/orders
    orders.koo
    orders_test.koo
```

### 8.2 Imports

```koo
import std/http
import std/sql/postgres
import pkg.koo.io/acme/stripe v2
import ./orders
```

An import binds the package's last path segment: `http.App`, `postgres.connect`, `orders.get_order`. `import x as y` renames, and is required when the last segment is not an identifier. Imports are per file and come before its declarations. An unused import is a compile error. Packages may not import one another in a cycle.

The standard library is `std/...`. Local packages are relative paths. Registry packages are `pkg.koo.io/owner/name` followed by their major version; the major must match a dependency in the manifest, and two majors of one package may coexist in a build.

### 8.3 Manifest and resolution

```toml
[package]
name = "acme/orders-service"
version = "1.4.0"
koo = ">=1.2"

[dependencies]
"acme/stripe" = "2.3"        # at least 2.3.0, below 3.0.0
"jsmith/retry" = "0.9.1"
```

A dependency names the **minimum** version it needs. Resolution uses minimum version selection, the Go algorithm: the chosen version of each package is the lowest, within its major, that satisfies every minimum in the dependency graph. The result is pinned in `koo.lock` with a SHA-256 of each package's source; builds read only the lockfile. Adding a dependency never silently upgrades another; `koo update` raises minimums deliberately.

### 8.4 Registry and supply-chain safety

pkg.koo.io is the single registry.

- **Namespaced names.** Every package is `owner/name`. There is no global namespace to squat.
- **Immutable versions.** A published version can be yanked (hidden from new resolutions) but never changed or deleted. A hash mismatch against `koo.lock` fails the build.
- **No install scripts.** Packages are source; the registry executes nothing and the compiler executes nothing at resolution. The only build-time code generation is derivation (Section 10.5), which runs in a sandbox.
- **Signed publishes.** Publishing requires a key registered to the owner; the registry records signature and provenance in the Sigstore style.
- **Capability declarations.** A package's manifest declares `network`, `filesystem`, `process` and `unsafe` capabilities. The compiler verifies them from what the package imports (`std/net`, `std/io`, `std/os`) and whether it contains `unsafe` blocks, transitively. `koo deps` shows the tree, so a JSON parser that asks for network access is visible before it is used.

### 8.5 Versioning

Semantic versioning is enforced mechanically. `koo publish` compares the exported API against the previous version and refuses a breaking change under a minor or patch bump. Breaking changes include removing or changing a `pub` declaration, renaming a parameter (callers may pass it by name), adding an error type to a `pub` function's set (Section 6.3), adding a variant to a `@closed` enum, adding a required field to a `pub` struct, and any change that takes away something the compiler had derived for a `pub` type: `Equatable`, `Hashable`, a codec interface, or the ability to be copied. Adding a variant to an open enum (Section 4.7), a field with a default that takes nothing away, or a new declaration is a minor change, and so is removing an error type from a set (Section 6.4).

### 8.6 Vendoring and offline builds

`koo vendor` copies all dependencies into `vendor/`, after which builds need no network. Organizations may run a private mirror; the lockfile hashes prevent a mirror from substituting code.

## 9. Runtime and core split

Koo is two layers: a **core** language that needs no runtime, and a **runtime** that adds tasks, actors and the service standard library. Services use both. WebAssembly plugins, libraries embedded in other languages and freestanding code use core alone. This is the division of Rust's `core`/`std`, designed in from the start.

### 9.1 Core mode

A package declares `core = true` in `koo.toml`. In core mode:

- **Available:** everything in Sections 3 to 6 except `isolate`; `unsafe`; FFI (Section 11); and the `std` packages marked *core* in Section 10.1.
- **Unavailable:** everything in Section 7 (tasks, actors, channels, `Shared`, `scope`, `map_parallel`, `all`, `race`, `with_timeout`, `isolate`) and the `std` packages marked *runtime*. No function in a core package suspends of its own accord: it has no suspending primitive to call.
- **Memory.** Core mode needs an allocator. On a hosted target the platform provides one; `wasm32-none` includes one (Section 9.5). On a freestanding target the package supplies one by implementing the `Allocator` interface in `unsafe` code. Heap values then behave exactly as in Section 5; in a core-mode build reference counts are never atomic, because there are no tasks.
- **Panics**, in a core-mode build, call a handler the program supplies; there is no unwinding and no resource cleanup after a panic.
- **Output:** an object file, static library or WebAssembly module with a C ABI and no libc dependency.

A core package may be imported by a runtime-mode package; the reverse is a compile error.

Core is a restriction on what a package may use, not a way of compiling it. A core package built into a runtime-mode program is compiled as part of that program: its reference counts honor freezing, its panics unwind and run cleanup, its loops check for cancellation, and a function of its that calls a lambda, a function value or an interface value supplied by the program suspends if that callee does. What this section says about counts, panics and suspension describes a *core-mode build*, one whose output is the library or module itself.

### 9.2 Runtime mode

The default. The runtime provides the task scheduler, actor mailboxes, timers, the I/O event loop, the allocator, panic reporting and the support code beneath the *runtime* `std` packages. It is linked statically into the binary (Section 13.4); there is no separate runtime to install or ship. Target: a minimal HTTP service binary under 3 MB, statically linked, without TLS; under 5 MB with it.

I/O uses epoll on Linux by default; `io_uring` is opt-in (`runtime.io = "uring"` in `koo.toml`) because container seccomp profiles commonly block it. macOS uses kqueue; Windows uses IOCP; WebAssembly uses WASI 0.3 (Section 9.5).

### 9.3 unsafe

`unsafe { ... }` blocks permit raw pointers, pointer arithmetic, uninitialized memory, calls into C, inline assembly, and the primitives of Section 9.4. Everything the compiler guarantees in Sections 5 and 7 is the programmer's responsibility inside the block.

- `koo deps` reports which packages contain `unsafe`, and a package may forbid it in its dependencies.
- An `unsafe` block must carry a comment, on the line of the opening brace or the line after, stating the invariant it relies on; `koo lint` enforces this.
- Safe code calling a function that contains `unsafe` is still safe. The function's author is responsible for the interface.

### 9.4 Threads and atomics

In `unsafe`, `std/core/sync` exposes OS threads, atomics with explicit orderings, spin locks and a futex. They exist so that driver authors and the standard library can build safe abstractions; `koo lint` flags their use outside `std`.

### 9.5 WebAssembly

WebAssembly is a first-class target with two forms. Nothing in Sections 3 to 8 changes meaning on either; this section lists what the platform changes.

**`wasm32-wasip3`: runtime mode, a WebAssembly component for WASI 0.3.**

- Tasks, scopes, channels, actors, `Shared`, cancellation and deadlines all work. The scheduler has one worker; there are no threads and no signals. Reference counts are never atomic, and freezing (Section 5.6) costs nothing.
- A task that waits on I/O suspends into the component model's asynchronous functions and streams; the host's event loop resumes it. This is the same suspension lowering as on native targets (Section 13.5), not an emulation.
- `std/http` serves through `wasi:http` (Section 10.3) and makes client requests through it; TLS for those is the host's. `std/io`, `std/os` environment access, clocks and random numbers use the matching WASI interfaces. `std/net` sockets and `std/sql/postgres` work where the host grants `wasi:sockets`.
- Not available, and a compile error when used: `std/os` processes and signals, TLS over raw sockets, `io_uring`. `task.blocking(f)` runs `f` inline. `std/crypto` uses portable constant-time implementations in place of aws-lc's assembly.
- `import c` works for C libraries that have been compiled to WebAssembly and are linked statically.
- **WIT.** `koo gen world.wit` generates a Koo package from a WIT world: imported interfaces become functions and types, and each export becomes an interface the program must satisfy. A function that implements an export is marked `@export("namespace:package/interface#name")`. `std` is itself written against these generated bindings.
- **Size.** Release builds for Wasm optimize for size, and only the runtime components the program reaches are linked. Section 13.6 sets the target.

**`wasm32-none`: core mode, a core WebAssembly module.**

- For plugins and sandboxed extensions hosted by another program. The module imports nothing unless the program declares imports with `import c`; `@c` functions are its exports; its memory is exported; a built-in allocator is included and exported as `koo_alloc` and `koo_free` so that the host can pass buffers.
- Everything available in core mode (Section 9.1) is available, including `std/json`, which is the usual way a plugin exchanges structured data with its host.
- A panic calls the package's panic handler and then traps.

On both targets `int` is 64 bits, a `[byte]` is limited to 4 GB, and exhausting the engine's call stack traps, which ends the instance.

## 10. Standard library

`std` ships one API for each protocol and standard that will still be in use in twenty years. Where a vetted implementation exists and correctness is a security matter (TLS, cryptographic primitives, the allocator), `std` wraps that implementation rather than rewriting it; the API is Koo's and the dependency is linked statically, so nothing changes for the user. This section specifies what the language and the reference example depend on; the complete API of every `std` package is the library reference in `koo-language/koo/spec/std`, which is normative and versioned with this document. Vendor-shaped APIs that change on their own schedule are first-party packages outside `std` (Section 10.2), so the 1.x stability promise never freezes a Kafka client. A new Koo service starts with zero third-party dependencies.

### 10.1 Standard library packages

| Package | Contents | Mode |
| --- | --- | --- |
| `std/core` | The prelude (Section 4.1): primitive types, `Decimal`, `Duration`, `Time`, collections, `Set`, formatting, the interfaces `Failure`, `Any`, `Equatable`, `Ordered`, `Hashable`, `Iterator`, `Reader`, `Writer`, the prelude error types, `panic`, `print`; and, outside the prelude, the `Allocator` interface | core |
| `std/core/sync` | OS threads, atomics, futex (Section 9.4) | core |
| `std/ffi` | The types that describe C to Koo: platform integers, `Ptr`, `Str`, `CString`, `Fn` (Section 11.2) | core |
| `std/decimal` | Rounding modes, `Money`, `Currency` | core |
| `std/hash` | SHA-2, SHA-3, BLAKE3, and non-cryptographic hashes, written in Koo | core |
| `std/json` | Typed encode and decode, JSON Schema, streaming parser, `Json` dynamic value (Section 10.4) | core |
| `std/encoding` | base64, hex, URL encoding, CSV, Protobuf wire format | core |
| `std/crypto` | HMAC, AES-GCM, ChaCha20-Poly1305, Ed25519, X25519, secure random, Argon2. A Koo API over the statically linked aws-lc library, whose primitives are constant-time and assembly-optimized; Koo does not reimplement them. | runtime |
| `std/time` | `Time.now()`, monotonic clock, `Date`, `Zone` with the IANA database, formatting and parsing | runtime |
| `std/task` | `Channel`, `Shared`, `with_timeout`, `isolate`, `select` support, `task.context()`, `task.yield()`, `task.blocking()` | runtime |
| `std/actor` | Actor runtime, supervision, registry, introspection | runtime |
| `std/io` | Files, directories, buffered readers and writers, streams | runtime |
| `std/net` | TCP, UDP, Unix sockets, DNS, TLS 1.2 and 1.3 through the statically linked rustls library with aws-lc (`tls = "system"` in `koo.toml` links the platform's OpenSSL or BoringSSL instead, for environments that mandate one) | runtime |
| `std/http` | HTTP/1.1 and HTTP/2 server and client, routing, middleware, WebSockets, graceful shutdown | runtime |
| `std/grpc` | gRPC server and client, Protobuf code generation via `koo gen`, streaming | runtime |
| `std/sql` | `Pool` handle, `Conn` and `Rows` resources, `Query` templates, transactions, row-to-struct mapping, migrations | runtime |
| `std/sql/postgres` | Native driver (wire protocol in Koo) | runtime |
| `std/log` | Structured logging, levels, JSON and text output; fields from the task context | runtime |
| `std/metrics` | Counters, gauges, histograms; Prometheus and OpenTelemetry export | runtime |
| `std/trace` | Distributed tracing via the task context; OpenTelemetry export; automatic spans in `std/http`, `std/grpc`, `std/sql` | runtime |
| `std/config` | Environment variables, files and flags, typed into a struct with validation | runtime |
| `std/test` | Assertions, table tests, property tests, benchmarks. The runner, which also does the timing, is `koo test` (Section 12.4); the fakes for `std/http` and `std/sql` live in those packages. | core |
| `std/os` | `os.env(name) -> string?`, processes, signals, exit codes | runtime |
| `std/cli` | Argument parsing, subcommands, help generation | runtime |

### 10.2 First-party packages

Maintained in the `koo-language` organization, published to the registry, versioned independently of the compiler: `koo-language/mysql`, `koo-language/sqlite` (binds the C library via FFI; SQLite has no wire protocol), `koo-language/redis`, `koo-language/kafka`, `koo-language/nats`, `koo-language/k8s` (typed client generated from the OpenAPI spec, informer cache, controller framework), `koo-language/oauth` (OAuth2, OIDC, JWT), `koo-language/yaml`, `koo-language/avro`, `koo-language/s3`.

### 10.3 HTTP and SQL

The reference example in Section 1.4 relies on these facts.

**`std/http`**

- `app.get(path, handler)` and its siblings for other methods register a handler of type `fn(http.Request) -> http.Response ! E` for any error set `E`. Registration changes the app, so these are `var` methods and `app` is a `var` binding. The app keeps the handler, so the parameter is `take` and the lambda is a closure (Section 5.7).
- An error the handler returns is matched by type against the handlers registered with `app.on_error((e: T) -> http.Response)`. `json.Error` and `ParseError` have default handlers that answer 400 with the issues as a JSON body. Any other unmatched error produces a 500, is logged with the request's trace identifier, and never reaches the client in detail.
- Each request runs in its own **isolated** task (Section 5.9). A panic in a handler produces a 500 and a log entry with the panic message and logical stack; no other request is affected and the server keeps running.
- `input: NewOrder = req.json()?` decodes the body into the expected type (Section 10.4). `http.json(value)` and `http.json(value, status: 201)` encode a response. `req.json` is generic in its result, which must satisfy `json.Decode`, and `http.json` in its argument, which must satisfy `json.Encode`. Bodies are streams: `req.body()` and `http.stream(...)` read and write without buffering the whole body.
- Each request's context (Section 7.7) carries a deadline taken from the server configuration (30 s unless set) and, for gRPC, the client's `grpc-timeout`. A request whose client disconnects is cancelled.
- `app.serve(port:)` runs until SIGTERM or SIGINT, then stops accepting, waits for in-flight requests up to the configured drain time, and returns.
- On `wasm32-wasip3`, `app.serve` registers the app as the component's `wasi:http` handler and returns when the host shuts the component down; `port:` is ignored. The same source is a native server and an edge function.

**`std/sql`**

- Query functions take a `sql.Query`, not a `string`. `sql.Query` is a template type (Section 3.3): in `db.query("select * from orders where id = {id}")` the literal's text becomes the statement with a placeholder, and `id` becomes a bound parameter. An interpolated value must satisfy `sql.Param`, as the built-in types, `[byte]`, optionals of them and distinct types over them do. A value can never become part of the statement text, so the natural way to write a query is the injection-proof way. Passing a `string` where a `sql.Query` is expected is a compile error; `sql.Query.raw(s)` exists for generated SQL and is flagged by `koo lint`.
- `db.query_one(q) -> Row? ! sql.Error` returns the first row or `none`. `db.query_row(q) -> Row ! sql.Error` is for a statement that must produce a row, and fails if it produces none. `db.query(q) -> sql.Rows ! sql.Error` returns a resource that streams rows as they arrive, consumed with `while row := rows.next()?` (Section 6.7); `rows.all()` collects them into a `[Row]`. `db.exec(q) -> int ! sql.Error` returns the affected row count.
- Queries compose without leaving the template type: `q.append(" and status = {status}")` adds text and parameters to a `var q: sql.Query`.
- Every failure is a `sql.Error`, whose `kind` is an open enum (`Connection`, `Constraint`, `Syntax`, `Serialization`, `NoRows`, ...), so new kinds do not change any signature (Section 6.3).
- `sql.Pool` is a handle (Section 5.8) to a runtime-managed set of connections, not an actor: with `pool: 20`, twenty queries run concurrently on twenty connections. Each call checks out a `sql.Conn` resource and returns it when the call, or the `sql.Rows` it produced, is finished. The pool is the standard example of state that must *not* be an actor: an actor would serialize every query behind one mailbox.
- `db.transaction(tx -> { ... })` runs the lambda on one connection, commits when it returns normally, and rolls back when it fails, panics or is cancelled. The lambda is a block (Section 5.7), so it may assign the caller's variables. The rollback runs in the cleanup window of Section 7.7.

### 10.4 JSON

JSON needs no annotation, no import beyond `std/json` and no schema file. For every type whose fields are serializable, the following exist, generated when first used:

```koo
fn T.from_json(bytes: take [byte]) -> T ! json.Error   // decode and validate
fn (self: T) to_json() -> [byte]                       // encode
fn (self: T) write_json(w: var Writer) ! IoError       // encode to a stream without an intermediate buffer
fn T.json_schema() -> Json                             // JSON Schema 2020-12, for OpenAPI and tool definitions
```

They are the members of two interfaces: `json.Encode` (`to_json`, `write_json`, `json_schema`) and `json.Decode` (`from_json`). A serializable type satisfies both automatically, as it satisfies `Hashable` (Section 4.9), and generic code asks for them as constraints: `fn (self: Request) json<T: json.Decode>() -> T ! json.Error`. A type that declares one of these functions by hand keeps its own, and the rest are still generated.

**Mapping.** The mapping is fixed, so two Koo services agree without configuration:

| Koo | JSON | Notes |
| --- | --- | --- |
| struct | object | Keys are field names after the rename policy. Fields beginning with `_` are never encoded or decoded. |
| `T?` | value or absent | Encode omits a `none` field. Decode reads a missing key or `null` as `none`. |
| field with a default | value | Decode uses the default when the key is missing. A field with neither `?` nor a default is required. |
| `json.Field<T>` | value, `null` or absent | An enum `Missing`, `Null`, `Value { value: T }`, for PATCH bodies where the three differ. |
| enum, all variants field-less | string | The variant name in snake_case: `"cash"`. |
| enum with fields | object with a tag | `{"type": "card", "last4": "4242", "brand": "visa"}`. The tag key is `"type"` unless `@json(tag: "kind")`; a variant field with the same name as the tag key is a compile error. |
| `int` and sized integers | number | Decode rejects fractions and out-of-range values. `@json(string)` encodes as a string for consumers limited to 2^53. |
| `float` | number | NaN and infinities encode as `null`; `koo lint` flags a `float` field that is not `float?` for this reason. |
| `Decimal` | string | `"19.99"`, scale preserved. Decode accepts a string or a number and reads the digits exactly, never through a float. `@json(number)` encodes as a number. |
| `string`, `char`, `bool` | string, string, boolean | A `char` is a one-character string. |
| `Time` | string | RFC 3339 in UTC: `"2026-03-01T12:00:00.25Z"`. |
| `Duration` | string | Literal form: `"250ms"`, `"1h30min"`. `@json(seconds)` and `@json(millis)` encode as a number. |
| `[byte]` | string | Base64, standard alphabet, padded. |
| `[T]`, `Set<T>`, `[T; N]`, tuple | array | A set encodes in insertion order. |
| `[K: V]` | object | `K` is a string, an integer, a field-less enum, or a distinct type over one of them. Encodes in insertion order. |
| distinct type | as its base | |
| `Json` | itself | |

Resources, function values, results, nested optionals (`T??`), channels and actor handles are not serializable; using a JSON function of a type that contains one is a compile error naming the field.

**Attributes.** On a field: `@json("createdAt")` sets the key; `@json(skip)` omits the field; `@json(flatten)` inlines a struct field's keys into the parent object; `@json(rest)` on a `[string: Json]` field collects keys that match no other field. On a type: `@json(rename: camel)` applies a policy to every field (`snake`, the default; `camel`; `pascal`; `kebab`); `@json(strict)` makes an unknown key a decode error (the default is to ignore it); `@json(tag: "kind")` sets an enum's tag key. A package sets the default rename policy for the types it declares once, with `json.rename = "camel"` in `koo.toml`, so a service that speaks camelCase writes no attributes at all.

**Errors.** `json.Error` carries `issues: [json.Issue]`, each `{ path: string, expected: string, found: string }` with a JSON Pointer path such as `/items/3/qty`. A syntax error stops decoding with one issue. Type and validation errors do not: the decoder continues and reports up to 32 issues, so a client sees every problem with its request at once.

**Validation.** If a type has a method `fn (self: T) validate() ! Invalid`, every decoder calls it after the type's fields are decoded, inner values first. `Invalid { field: string, message: string }` is in the prelude; a failure becomes an issue at that field's path. The same hook runs in `from_row` and `std/config`.

**Dynamic values.** `Json` is an enum for data whose shape is not known at compile time:

```koo
@closed
pub enum Json {
    Null
    Bool { value: bool }
    Int { value: int }
    Float { value: float }
    String { value: string }
    Array { items: [Json] }
    Object { fields: [string: Json] }
}

body := json.parse(bytes)?                                  // Json ! json.Error
city := body.at("user", "address", "city").string() else "unknown"
reply: Json = ["ok": true, "items": [1, 2, 3], "next": none]   // literals type as Json when Json is expected
```

`j.at(keys: ...Any) -> Json` walks object keys and array indexes and yields `Null` where the path does not exist. `j.string()`, `j.int()`, `j.float()`, `j.bool()`, `j.array()` and `j.object()` return `T?`. When the expected type of a literal is `Json`, list, map, string, number, boolean and `none` literals are `Json` values, and a non-literal expression `e` inside one stands for `Json.from(e)`. `Json.from(value)` and `T.from_json_value(j)` convert between typed and dynamic forms.

**Performance.** Decoding validates UTF-8 and structure in one pass using SIMD where the target has it. A decoded `string` that contains no escape sequence is a substring of the input buffer (Section 4.4); `from_json(bytes)` therefore takes its argument as `take`, and a long-lived value decoded from a large body should call `s.detach()` on strings it keeps (Section 4.4).

### 10.5 Other formats and derivation

**Rows.** `T.from_row(row: sql.Row) -> T ! sql.Error` exists for every struct whose fields are SQL-mappable, matching columns to field names; `@sql("column_name")` renames one. **CSV** works the same way through `std/encoding` with `@csv("header")`. **Protobuf** is not derived from field names, because a Protobuf field is identified by a number that must stay stable: `koo gen` generates Koo types and codecs from `.proto` files.

**Derivers.** Every codec above is produced by one mechanism. A *deriver* is a function, exported by a package, that receives a description of a type (its name, type parameters, fields or variants, their types and attributes) and returns Koo source. A deriver, and everything it calls, must be core-mode code (Section 9.1). The compiler compiles the deriver to WebAssembly and runs it in an embedded engine with no imports: it cannot perform I/O, is deterministic, and its output is cached by the hash of its input. The output may declare only functions and methods on the derived type; it may not declare types, top-level bindings or imports. For a generic type the output is generic, and exists for the type arguments that satisfy the same interface (Section 4.9). The compiler runs a deriver when checking first needs one of its functions for a type (Section 13.1).

The derivers for JSON, rows and CSV ship in `std` and are applied implicitly on first use. Any other deriver is applied explicitly: `@derive(yaml.codec)` on a type runs the deriver `codec` exported by `koo-language/yaml`. This is the closed form of code generation that replaces macros (open question 5).

### 10.6 Time

`Time` is a UTC instant at nanosecond precision; a `Zone` converts it for display and parsing. There is no local-time type that depends on the machine's setting. `Duration` is a signed count of nanoseconds with the literal suffixes of Section 3.3; there is no day unit, because a calendar day is not a fixed duration. `Duration * int`, `Duration / int` and `Duration + Duration` are `Duration`. `Time + Duration` is a `Time`, `Time - Time` is a `Duration`, `Time + Time` is a compile error.

## 11. Foreign function interface

Koo calls C without hand-written bindings and can be called from C. FFI is the one place memory safety depends on the programmer, so every foreign call sits inside `unsafe`.

### 11.1 Calling C

```koo
import std/ffi
import c "zlib.h" link "z"

fn compress(data: [byte]) -> [byte] ! CompressError | Overflow {
    unsafe {
        // zlib writes at most `bound` bytes into `out`, and `out` has exactly `bound` bytes.
        len := ffi.ULong.from(data.len)?
        bound := c.compressBound(len)
        var out := List<byte>.with_size(int.from(bound)?)
        var out_len := bound
        rc := c.compress(out.ptr(), out_len.ptr(), data.ptr(), len)
        if rc != c.Z_OK { fail CompressError { code: int.from(rc) } }
        return out[0..int.from(out_len)?]
    }
}
```

`import c "header.h"` runs the compiler's built-in C header parser (libclang) at build time and exposes the header's functions, structs, enums and constants under the name `c`, or under another name given with `as` (`import c "sqlite3.h" as sqlite link "sqlite3"`). C imports without `as` share `c`.

Names keep their C spelling and are always written after that prefix, where case carries no meaning: `c.order` is a type and `c.Z_OK` a constant because the header says so. A C name never takes type arguments or a struct literal; a C struct is made with `ffi.zero()` and its fields are then assigned. The types that describe C to Koo are, by contrast, ordinary Koo types in `std/ffi` and follow Koo's naming. Conversions between C integer types and Koo's follow Section 4.2.

A C call runs to completion on the calling worker thread and cannot suspend. A C function that may block for long (a disk read, a lock, a network call made by a C library) is called through `task.blocking(() -> ...)`, which runs the closure on the runtime's blocking pool and suspends the calling task until it returns. Tasks move between worker threads at suspension points; a C library with thread affinity must be driven from a single `task.blocking` closure or from one dedicated actor that never suspends between related calls.

### 11.2 Type mapping

| C | Koo | Notes |
| --- | --- | --- |
| `int8_t` … `uint64_t`, `float`, `double` | `i8` … `u64`, `f32`, `float` | Exact |
| `int`, `long`, `size_t`, `unsigned long` | `ffi.Int`, `ffi.Long`, `ffi.Size`, `ffi.ULong` | Platform-width; converted by the rules of Section 4.2: `int.from(x)?`, `ffi.ULong.from(n)?` |
| `T*` | `ffi.Ptr<T>` | Raw pointer; dereference only in `unsafe` |
| `char*` (string) | `ffi.Str` | `s.to_string()` copies into a Koo `string`; `string.to_c()` returns an `ffi.CString` resource holding a NUL-terminated copy, freed at scope end |
| `struct foo` | `c.foo` | Same layout; field access is safe, pointers inside are not |
| function pointer | `ffi.Fn<fn(A, B) -> R>` | A Koo function marked `@c` may be passed |

`value.ptr()` yields an `ffi.Ptr` to a Koo value and is legal only inside `unsafe`; the compiler keeps the value alive to the end of that block. A pointer C retains beyond the call is the programmer's responsibility, stated in the block's required comment.

### 11.3 Exporting to C

```koo
@c pub fn koo_parse_order(json: ffi.Str, out: ffi.Ptr<c.order>) -> i32 { ... }
```

An `@c` function is called by code that knows nothing of tasks. If its body may suspend, the call runs it as a task of its own and the calling thread waits for that task to finish; otherwise it is an ordinary function. Either way it is never cancelled. WebAssembly has no second thread to wait on, so there an `@c` function that may suspend is a compile error; an export that suspends is written with `@export` (Section 9.5). A panic that reaches its boundary cannot unwind into C: it is reported and the process aborts, or in a core-mode build the panic handler is called.

`@c` gives a function the C calling convention and an unmangled name. `koo build --lib` produces a static or shared library plus a generated header. Exported signatures may use only the types in Section 11.2. This is how Koo is embedded in Python, Node, Java or Go. On `wasm32-none`, `@c` functions are the module's exports. WebAssembly components use WIT instead of C headers (Section 9.5).

### 11.4 Linking

`link "z"` adds `-lz`. On Linux, Koo binaries are fully static by default (musl), suitable for `FROM scratch` containers; `--dynamic` links glibc dynamically for programs that need `dlopen` or NSS. On macOS, `libSystem` is linked dynamically, as Apple requires. `koo.toml` may specify library search paths.

### 11.5 Limits

No automatic C++ binding (only `extern "C"` surfaces). No calling convention other than the platform C ABI. No automatic ownership transfer: memory allocated by C is freed by C, and the compiler never frees an `ffi.Ptr`.

## 12. Tooling

One binary, `koo`, does everything. A developer who installs Koo has the formatter, test runner, package manager, documentation generator and language server already working.

### 12.1 Commands

| Command | Does |
| --- | --- |
| `koo new orders-service` | Project skeleton with `koo.toml`, `main.koo`, a test, `.gitignore` and a Dockerfile |
| `koo run` | Debug build and run; `--watch` re-runs on change |
| `koo build` | Debug build; `--release`; `--target <triple>` (Section 13.3); `--lib`; `--show-copies`; `--traces` |
| `koo test` | Runs `*_test.koo` in parallel; `--bench`; coverage built in |
| `koo fmt` | Formats in place; `--check` for CI; no options |
| `koo lint` | Checks beyond the compiler: `unsafe` comments, `Any` outside `std`, I/O inside actor methods, `std/core/sync` outside `std`, unused `pub`, a plain `else` that discards more than one error type, `sql.Query.raw`, a parameter that is always copied and should be `take` |
| `koo add acme/stripe` | Adds a dependency with the latest release as its minimum and updates the lockfile |
| `koo update` | Raises minimum versions within declared majors |
| `koo deps` | Dependency tree with capabilities per package |
| `koo doc` | Renders `///` comments; `--serve` for local browsing |
| `koo gen` | Generates typed Koo from Protobuf, OpenAPI, SQL schema or WIT files (Section 9.5) |
| `koo publish` | Publishes to pkg.koo.io after the API compatibility check |
| `koo lsp` | Starts the language server |
| `koo deploy` | Builds a minimal OCI image from a release build and pushes it to any container registry; hosting providers may register additional targets |

### 12.2 Compile time

Fast compilation is a language requirement because slow compiles are the most common reason developers abandon Rust and C++ for Go. Targets, measured in CI on the reference machine named in the benchmark suite. A "clean" build compiles the service and all of its dependencies from source with an empty cache; the standard library and runtime are prebuilt and ship with the compiler, except for the generic instantiations the service itself requires.

| Scenario | Target |
| --- | --- |
| Clean debug build, 50,000-line service with 20 dependencies | under 5 s |
| Incremental debug rebuild after a one-function change | under 1 s |
| `koo test` turnaround, one test file changed | under 1 s |
| Clean release build, same service | under 60 s |

Design choices that make this achievable: per-package compilation with cached interface files; no macros, so parsing never runs user code; `@derive` output cached by input hash; generics monomorphized per package with an instantiation cache; parameter kinds, which keep the ownership check local to each function; two inferred summaries per function, *may suspend* and *may keep* a borrowed argument, which keep suspension lowering and ownership optimization local to each package; single-pass code generators for debug builds (Section 13.2); and a type system without higher-kinded types or specialization.

### 12.3 Language server

`koo lsp` ships with the compiler and shares its front end, so editor diagnostics are the compiler's. It provides completion, go-to-definition, references, rename, hover with inferred types and error sets, inline copy markers (Section 5.1), and a "handle this result" action that inserts a `match` with every arm. First-party extensions for VS Code, JetBrains, Neovim, Zed and Helix ship with 1.0.

### 12.4 Testing

Tests are functions named `test_*` in `*_test.koo` files in the same package as the code under test, so they see unexported declarations. `std/test` provides `assert`, `assert_eq` with structural diffs, table-driven tests, property-based tests with shrinking, and benchmarks with statistical reporting. `std/http` and `std/sql` expose in-memory fakes. Tests run in parallel by default; `@serial` runs a test alone.

### 12.5 Debugging and profiling

Debug builds emit DWARF; `lldb` and `gdb` work with Koo names and types. Because a suspended task is a chain of frames on the heap rather than a thread stack (Section 13.5), the compiler emits the frame layouts and ships debugger extensions that print a task's logical call stack; a panic report and `koo build --traces` print the same logical stack. `koo run --profile` writes a CPU and allocation profile viewable with `koo profile view` or exportable to pprof. Because tasks and actors are runtime concepts, the profiler shows time per actor and mailbox depth.

### 12.6 Formatting

`koo fmt` has no configuration. Indentation (4 spaces), brace placement, line width (100), one statement per line, and import ordering are decided by the language.

### 12.7 Error messages

```
error: unhandled result: NotFound | sql.Error
  --> main.koo:14:5
   |
14 |     get_order(db, id)
   |     ^^^^^^^^^^^^^^^^^ this is an `Order ! NotFound | sql.Error`, which may be an error
   |
   = note: a result is a value or an error, and the error may not be ignored (Section 6.4)
   = help: to handle it here, give a value in its place with `else`, or inspect it with `match`
help: or pass the error on to the caller: write `?`, and let `show` fail with `NotFound | sql.Error`
   |
12 | fn show(db: sql.Pool, id: Id) ! NotFound | sql.Error {
   |                              ++++++++++++++++++++++
14 |     get_order(db, id)?
   |                      +
```

An error has these parts, and a compiler for Koo gives each of them wherever it applies:

- **The message**: what is wrong, in one line, in the words of the program: its names and its types.
- **The place**, marked with `^`, and beside the marks what they point out.
- **Related places**, marked with `-`: the declaration that set the expected type, the first of two declarations, the parameter an argument is for, the bracket that was never closed. An error that is about two places shows both.
- **Notes**, which state the rule, with its section, and **help**, which says what to do.
- **The correction**, where there is exactly one thing the programmer can have meant: the line as it should read, with `+` under what is added, or the old line and the new one marked `-` and `+`.

Three rules hold for all of them. One mistake is one error: the parser and the checker recover so that what follows a mistake is not reported for it. Text in the syntax of another language (`let`, `function`, `elif`, `x++`, the `for` of C) is reported as that, with the Koo that was meant, and not as an unexpected token. And an error never points into the standard library: a mistake that shows up inside a library function is reported at the call the program made.

Messages are colored when standard error is a terminal. `NO_COLOR` turns the colors off and `CLICOLOR_FORCE` turns them on.

## 13. Compiler architecture

The compiler is a multi-stage pipeline with three code generators behind one intermediate representation: Cranelift for fast native debug builds, LLVM for optimized builds of every target, and a direct WebAssembly emitter for fast Wasm debug builds.

### 13.1 Pipeline

1. **Lex and parse** each file to a syntax tree. Error-tolerant, so the language server works on broken code.
2. **Name resolution** across the package; imports resolved from the lockfile.
3. **Declaration checking**: types, signatures, interfaces and error sets of every declaration.
4. **Body checking**: type inference and checking of every function body. Error-set completeness, result handling and `T?` handling are checked here. When a body first needs a derived function of a type, the deriver runs in the sandbox (Section 10.5) and its output is parsed and checked like any other source; because derivers add only functions, nothing already checked changes.
5. **Ownership checking**: with parameter kinds as input, the per-function exclusivity check (Section 5.4), resource moves and last-use detection. Everything that can reject a program has now run, on the code as written: no error depends on how a generic is instantiated or on what the optimizer did. The one later error concerns a target, not the language: an `@c` function that may suspend on WebAssembly (Section 11.3).
6. **Monomorphization** of generics, interface-typed parameters and lambda arguments per package, with an instantiation cache. Produces KIR, in which every value has an explicit owner and every copy, move, freeze and free is explicit.
7. **KIR optimizations**: inlining; then ownership optimization (copy-on-write placement, reference-count elision, escape analysis), using the *may keep* summaries of Section 5.3; bounds-check and overflow-check elimination; collection-chain fusion; dead code removal.
8. **Suspension lowering**: the instantiated, inlined functions that may still suspend become state machines (Section 13.5).
9. **Code generation**: Cranelift, LLVM or the WebAssembly emitter.
10. **Link**: bundled `lld` for native targets and `wasm-ld` plus component encoding for Wasm; no system linker required.

### 13.2 Code generators

| | Cranelift | WebAssembly emitter | LLVM |
| --- | --- | --- | --- |
| Used for | Debug builds of native targets: `koo build`, `koo run`, `koo test` | Debug builds of Wasm targets | `--release` on every target, `koo deploy` |
| Code quality | Target: at least 70 percent of LLVM `-O2` throughput on the benchmark suite | Unoptimized; one pass from KIR | `-O3`, ThinLTO, PGO |
| Compile speed | Target: at least 10 times faster than LLVM on the reference service | Target: as Cranelift | Section 12.2 |

Cranelift generates native code only, which is why Wasm debug builds have their own emitter. All three consume the same KIR after step 8. A behavioral difference between them is a compiler bug.

LLVM is the first of the three to exist, because the language's first claim is speed and that claim can only be tested against the optimizer Rust uses. Until milestone 0.2 it also makes the debug builds, and the compiler drives it by writing LLVM IR as text and running the system's `clang`, which keeps the compiler free of a build-time dependency on LLVM.

### 13.3 Targets

Target names are `<arch>-<os>[-<libc>]`.

| Target | Mode | Tier at 1.0 |
| --- | --- | --- |
| `x86_64-linux-musl` (static, default), `x86_64-linux-gnu` (dynamic) | runtime, core | 1 |
| `arm64-linux-musl`, `arm64-linux-gnu` | runtime, core | 1 |
| `arm64-macos` | runtime, core | 1 |
| `wasm32-wasip3` (a WebAssembly component; single-threaded scheduler) | runtime | 1 |
| `wasm32-none` (a core WebAssembly module with no imports) | core | 1 |
| `x86_64-macos` | runtime, core | 2 |
| `x86_64-windows`, `arm64-windows` | runtime, core | 2 |
| `arm64-none`, `x86_64-none`, `riscv64-none` (freestanding) | core | 3 |

Tiers are defined in `GOVERNANCE.md`. Cross-compilation to any target is a flag; the compiler ships every target's standard library and runtime and links without a target system. Section 9.5 specifies what differs on WebAssembly.

### 13.4 Implementation language

The compiler and the runtime are written in **Rust** through 1.0. For the compiler this gives memory safety and direct access to Cranelift. For the runtime it gives a scheduler, I/O layer and TLS stack built on components with years of production use (Section 10.1), instead of ones written in a language whose compiler is itself new. The runtime is shipped as a prebuilt static library per target and linked into every runtime-mode binary; release builds use cross-language LTO.

Self-hosting the compiler and rewriting the runtime in Koo core mode are goals after 1.0, not prerequisites (open question 9).

### 13.5 Runtime design

This section describes the reference implementation. The language guarantees are those of Sections 5 and 7; the mechanisms here may change without a proposal as long as those hold.

- **Tasks are stackless.** Pipeline step 8 turns every function that may suspend into a state machine whose frame holds exactly the locals that are live across a suspension. A task is a chain of such frames. A function that cannot suspend, which is most code, is an ordinary function on the worker thread's stack. Frames along a statically known call chain are laid out in one allocation; a recursive call or an indirect call (through an interface value or a function value) to a function that may suspend allocates its frame separately.
- **Suspension is inferred.** "May suspend" is one bit per function, computed bottom-up from the suspending primitives in the runtime and stored in the package interface file. It is never written by the programmer and is not part of a function's type. An indirect call, through an interface value or a stored function value, may reach a callee that suspends, so a function that makes one is compiled as one that may suspend. Each vtable entry and function value carries one bit saying whether its target can; when it cannot, which is always so in a core-mode build, the call is an ordinary call with no frame. The compiler-provided methods of `Failure` never suspend. Hot paths seldom meet the question, because interface-typed parameters and lambda arguments are resolved statically (Sections 4.9 and 4.11).
- **No pointer into a thread stack survives a suspension.** Nothing is relocated, no stack maps are required of the backends, and the same lowering serves Cranelift, LLVM and WebAssembly.
- **Scheduler:** work-stealing, one worker thread per core; a single worker on WebAssembly.
- **Fairness without preemption.** A task keeps its worker until it suspends. A monitor thread watches for a worker that has run one task for more than 10 ms and starts a replacement worker, up to `runtime.max_workers`, so one long computation does not delay other tasks; the replacement retires when the computation ends. On WebAssembly there is one thread and a long computation delays other tasks until it suspends or calls `task.yield()`.
- **Cancellation checks.** The compiler emits a check (one load and one predictable branch) at every suspension point, on every loop back-edge and in the prologue of every function that can be reached recursively. A cancelled task unwinds from the check. The check does not suspend, so it does not make a function a state machine.
- **Unwinding.** Panics and cancellation unwind through landing pads at call sites: table-based unwinding on native targets, the WebAssembly exception-handling feature on Wasm. A backend without landing pads may instead return a hidden flag that every call site tests; the behavior is the same. A function that may suspend catches the unwind in its own frame, runs its cleanup, which may itself suspend inside the cleanup window (Section 7.7), and then resumes its caller's frame in the unwinding state, so between heap frames unwinding is an ordinary state transition. Unwinding stops at a task boundary, at the `with_timeout` or `scope` that caused it (Section 7.7), and at an `@c` boundary, where a panic aborts (Section 11.3). A core-mode build has no unwinding (Section 9.1).
- **I/O:** epoll (Linux default), `io_uring` opt-in, kqueue (macOS), IOCP (Windows), WASI 0.3 asynchronous functions and streams (Wasm). System calls that cannot be made asynchronous run on a separate blocking pool.
- **Actors:** a bounded lock-free multi-producer single-consumer mailbox. An actor with an empty mailbox occupies no scheduler slot. An ask is a message carrying a one-shot reply slot and the asker's ask chain (Section 7.3).
- **Shared cells** (Section 7.5): an atomic pointer to a frozen value. Writers swap the pointer; the old value is reclaimed by epoch only after every reader that might have loaded it has taken its reference, so a reader never touches freed memory and never waits.
- **Stack depth.** A function that can be reached recursively or through an indirect call compares the remaining worker stack against a limit in its prologue; frames of suspending functions count against the task's memory limit (Section 7.1). Exceeding either panics rather than faulting.
- **Allocator:** mimalloc, statically linked, on native targets; a single-threaded size-class allocator on Wasm.
- **Timers:** hierarchical timing wheel.
- **C calls** are ordinary calls on the worker thread's stack. A C call that blocks is wrapped in `task.blocking` (Section 11.1), which runs it on the blocking pool.

### 13.6 Correctness and performance infrastructure

- A conformance suite in `tests/spec`, one test per numbered section of this document, run on every Tier 1 target.
- Differential testing between the Cranelift, LLVM and WebAssembly backends on every merge.
- Fuzzers for the parser, the type checker, the ownership pass and suspension lowering.
- The runtime is tested under a thread sanitizer and a deterministic scheduler that explores interleavings.
- A **benchmark suite** with regression gates in CI, run on a named reference machine against pinned baseline versions. Targets at 1.0:

| Benchmark | Baseline | Target |
| --- | --- | --- |
| TechEmpower-style JSON, plaintext, single query, multiple queries: throughput | Rust (axum + tokio), Go (net/http) | Within 10 percent of Rust on every test; at or above Go |
| The same tests: p99 and p99.9 latency at 80 percent of peak load | Rust, Go | Within 10 percent of Rust; below Go |
| Resident memory per idle keep-alive connection and per parked task | Rust, Go | Within 25 percent of Rust; below Go |
| Allocation-heavy microbenchmarks (parse, transform, serialize 1 MB JSON) | Rust (serde) | Within 15 percent |
| Reference-count operations executed per request on the service benchmarks | — | Reported on every run; regressions gate merges |
| Task spawn and switch | Go goroutine | Within 20 percent |
| Actor ask round trip, same core | — | Under 300 ns |
| `Shared.get`, uncontended and with 64 readers | — | Under 20 ns; under 50 ns |
| Overflow and cancellation checks | Same code with checks disabled | Under 2 percent combined |
| Wasm component size, reference example | — | Under 2 MB |
| Compile times | — | Section 12.2 |

Section 5.11 lists where Koo's design costs more than Rust's. The targets above are the evidence for design goal 1; a target that is missed at a milestone gate is reported in the release notes, not removed.

### 13.7 Milestones

| Milestone | Scope |
| --- | --- |
| 0.1 | Parser, declaration and body checking, ownership pass with the exclusivity check, LLVM backend and release builds, core types, results, `T?`, resources, core mode, `koo run`, `koo test`, `std/test`. Hello world. **Gate 1:** memory-model benchmarks (copies, reference-count operations, allocation microbenchmarks) published against Rust and Swift. |
| 0.2 | Cranelift backend for debug builds. Generics, interfaces and lambdas. FFI with header import. |
| 0.3 | Suspension lowering. Runtime: tasks, scheduler, context, deadlines and cancellation, scopes, channels, `map_parallel`, `all`, `race`, `with_timeout`, `isolate`. WebAssembly emitter; `wasm32-none` and `wasm32-wasip3` pass the conformance suite for everything implemented so far. The deriver sandbox and the JSON deriver. `std/net`, `std/http` (HTTP/1.1), `std/json`. **Gate 2:** task and HTTP benchmarks published, with compile time and binary size; open questions 8 and 10 settled. |
| 0.4 | Actors, supervision, `Shared`. `std/sql/postgres` with the row deriver, `std/log`. The reference example runs end to end natively and as a `wasi:http` component. Section 14 settled. |
| 0.5 | `koo fmt`, `koo lsp`, `--show-copies`, cross-compilation. Package manager, pkg.koo.io, `koo publish`, `@derive` for third-party derivers. |
| 0.6 | `std/grpc`, HTTP/2, TLS, `std/metrics`, `std/trace`. Windows targets. |
| 0.7 to 0.9 | Public beta. Stabilize syntax, remove features that did not earn their place, complete `std`. |
| 1.0 | Stability commitment begins. |

## 14. Open questions

Decisions this version leaves open, to be settled by proposal before milestone 0.4.

| # | Question | Options | Current recommendation |
| --- | --- | --- | --- |
| 1 | Should the error separator `!` be kept, or should errors join the success type with `\|` only? | Keep `!`; or `Order \| NotFound` with `?` unwrapping the first type | Keep `!`. One symbol buys clarity at every call site. |
| 2 | Should actors be distributable across processes and machines? | Local only at 1.0; or location-transparent handles | Local only at 1.0; add as a first-party package later, explicitly. |
| 3 | Should `Money` carry its currency in the type (`Money<USD>`) rather than as data? | Type-level: mismatches are compile errors, but currencies read from a database need a runtime step; data-level: simpler, runtime check | Data-level, as written. |
| 4 | Interface satisfaction: structural only, or require `type X: Interface`? | Structural (Go); declared (Rust) | Structural with optional declaration, as written. |
| 5 | How much may a deriver generate? | Codecs only; functions on the derived type (as written); general compile-time code generation | Functions on the derived type. Codecs alone exclude schemas and validators; anything wider is macros by another name. |
| 6 | Should actor methods be forbidden from suspending on I/O, rather than warned? | Hard error; or lint warning | Warning. An actor that owns one connection is a legitimate use. |
| 7 | Default mailbox size | 1,024; or unbounded with a lint | 1,024. Backpressure by default is the safer failure mode. |
| 8 | Should an interface-typed parameter be compiled statically, as a type parameter, or through a vtable (Section 4.9)? | Static (as written): fastest, more generated code; dynamic: less code, an indirect call | Static, unless the milestone 0.3 gate shows compile time or binary size missing its target. |
| 9 | Self-hosting timeline for the compiler and the runtime | At 1.0; after 1.0; never | After 1.0 for both (Section 13.4). |
| 10 | Task implementation, if suspension lowering (Section 13.5) misses its targets | Stackless lowering (as written); fixed-size virtual-memory stacks natively with a transform on Wasm | Stackless. It is the only option that is identical on every target. Decided by the milestone 0.3 benchmark gate. |
| 11 | Should integer overflow panic in release builds? | Panic always (as written); wrap in release, as Rust does | Panic always. Task isolation (Section 5.9) bounds the damage, and a wrong total is worse than a failed request. |
| 12 | Default supervision policy | Bounded restart (as written); stop | Bounded restart. An actor that stays down fails every later request. |

### 14.1 How to participate

1. Read this specification and open an issue in `koo-language/koo` for anything unclear, inconsistent or missing.
2. Propose a language change by opening a pull request against `koo-language/proposals`.
3. Contribute to the implementation: the conformance suite in `tests/spec` lists, section by section, what is implemented and what is not.

## 15. Changes

### 15.1 Changes in 0.5

Version 0.5 is the result of reading 0.4 as its implementer would. It settles every place where 0.4 contradicted itself or left the compiler to guess, by the rule that the simpler and more uniform answer wins. Section numbers are this version's.

**Grammar.** Newlines are significant only directly inside block braces; a leading `-` never continues a line; an expression statement must be a call or end in `?` or `else` (3.7). The kind of a brace is decided by what precedes it, and `for` joins the struct-literal rule (3.9). Primitive type names are predeclared (3.1). `List<T>` and `Map<K, V>` name the collection types for type-level calls (3.9). C names live behind their import's prefix, where case means nothing, and the types that describe C moved to `std/ffi` (11). Blocks never have values; `if` and `match` used for a value take single-expression branches (6.6). Tuples are taken apart by destructuring only (4.5).

**Types.** The four widenings are stated, with where they apply and how inference treats them (4). `Equatable` exists; operators mean in generic code what they mean outside it, and sorting uses `compare` (3.4, 4.2). Interfaces may declare type-level functions; an interface-typed parameter is static; an interface that mentions `Self` cannot be an interface value (4.9). Derived codecs are interfaces satisfied automatically, so generic code can ask for them (4.9, 10.4). Type parameters may be results, and nested results are defined (4.10, 6.3). The built-in conversions are the one family with several signatures (4.2). Template types have a stated protocol (3.3). Structs may be recursive (4.7). A pattern on another package's struct ends with `..` (6.6).

**Resources.** `resource` now has four uses and one meaning, exactly one owner: a resource type, a `resource interface`, the constraint `T: resource` and `resource fn` (4.9, 4.10, 4.11). They replace the special provision by which the built-in generics accepted resources. An overlap that copying cannot resolve is a compile error, and a loop over an iterator is a write (5.4, 6.7). A fallible `close` is required on normal exits only (6.8).

**Lambdas.** A lambda passed to a borrow parameter is a block that works on its caller's variables; every other lambda captures by value (5.7). This settles open question 8 of 0.4. A borrowed function parameter cannot be kept (5.3). `with_timeout` runs its lambda in the current task, so it works with resources the caller holds (7.2, 7.7).

**Compilation.** Core is a restriction on what a package uses, not a way of compiling it (9.1). A second inferred summary, *may keep*, makes reference-count elision sound across calls (5.3, 5.6). An indirect call tests one bit to learn whether it needs a frame (13.5). Unwinding through suspended frames and at `@c` boundaries is specified (13.5, 11.3). The pipeline checks everything before it instantiates anything, and derivers run on demand (13.1). Call depth has one description (7.1, 13.5).

**Smaller fixes.** `Decimal` division by zero (4.2, 4.3). Chain fusion no longer requires lambdas that cannot panic (4.5). Actor handles satisfy interfaces; a task spawned by an actor starts with an empty ask chain; a `send` to `self` on a full mailbox panics (7.3). What the compiler derives for a public type is part of its API (8.5). Derivers are functions with function names and must be core code (10.5). `Json` is `@closed` (10.4). Milestones are reordered so that nothing precedes what it depends on, and specification versions are distinguished from milestones (13.7).

**Settled by the grammar.** Writing `spec/grammar.ebnf` and the parser that implements it (`compiler/koo-syntax`, whose tests parse every code block in this document) fixed the points the prose had left open. The primitive type names may name members, as reserved words do (3.1, 3.2). The escapes `\r`, `\0` and `\'` exist; a duration literal uses whole numbers; an interpolation is written on one line (3.3). Every comma-separated list may end with a comma, angle brackets do not suspend newlines, and an opening brace is on the line of its statement (3.7). The body of a lambda, a `match` arm or a `select` arm is a block or one simple statement (3.9, 6.6, 7.6). `!` applies to a whole `match` arm (6.4). `resource`, `enum` and `actor` declarations may state the interfaces they satisfy (4.9). A receiver is named `self` (4.6). A top-level binding is one name and never `var` (3.5). `:=` takes a tuple pattern or `_` wherever it takes a name (6.7). Attribute arguments have a grammar (3.8). `!` after a function type's return type belongs to the function type (4.11). Imports come first in a file (8.2). A `match` guard and a `select` operand end at the arm's `=>`, and a statement that begins with `if` or `match` ends at its closing brace (3.9). An underscore in a number separates two digits (3.3). A syntax tree is at most 512 levels deep (`spec/grammar.ebnf`). The four signatures in 10.4 are now written as Koo.

**Settled by the compiler.** The first code generator is LLVM, not Cranelift, and Gate 1 moves to milestone 0.1 with it (13.2, 13.7). An interpolation format is an optional alignment (`<`, `>`, `^`), an optional `0` that pads a number with zeros after its sign, an optional width counted in characters, and for a `float` an optional `.` and a number of decimals; without an alignment a number goes to the right and anything else to the left (3.3). The default of a parameter is a constant: a literal, `none`, a list or map of constants, or a field-less enum variant. It is checked where it is declared, and a `var` or `take` parameter has none (3.9). A binding condition takes a tuple pattern: `if (key, value) := text.cut("=")` (6.7). A name that contains `__` is reserved for the standard library. `koo test` runs each test in a process of its own, so a test that panics fails alone (12.4). `IoError { message: string }` is the error of `std/io`, and it is predeclared. `koo run file.koo` runs one file as a program, and `koo run dir` runs the package in a directory; `koo test dir` adds its `*_test.koo` files (8.1, 12.4). An import belongs to its file. Exclusivity extends to blocks: a call that changes a variable through a `var` argument or a `var` receiver may not be given a block that uses that variable, and where the variable belongs to an enclosing function it may not be given a lent function either, since that one may be such a block (5.4, 5.7). Whether a function parameter is lent is decided by its declared type: a parameter declared with a type parameter is not, whatever the type argument (5.7). Text that arrives from outside the program (arguments, the environment, standard input) becomes a `string` with U+FFFD in place of bytes that are not UTF-8 (4.3). A hand-written `hash` takes nothing and returns `int` (4.9). A generic function or type is made for types nested at most 24 deep and of at most 512 parts. A method named `compare`, `hash` or `to_string` takes its receiver and its parameter to read, never `var` or `take`: `<`, a map and interpolation call it where nothing says a value may change (4.6, 4.9). `sort`, `sorted`, `min` and `max` order floats by `compare`, so NaN comes last (4.2). A chain of `map` and `filter` on a list, alone or ended by `fold`, `sum`, `count`, `any`, `all` or `find`, is fused when every function in it is a lambda or a named function that the compiler can see does no input or output, changes no variable outside itself and calls nothing unknown; a function received as a parameter or kept in a variable is unknown, so a chain that uses one runs stage by stage (4.5). `xs.sort()` on a list of integers need not be stable, since equal integers cannot be told apart; every other sort is (4.5). The elements of a tuple are not reached by number: `pair.0` is an error that says to destructure (4.5). `value.to_json()` returns a `string` and `T.from_json(text)` takes one, and both exist without an import; the error is `JsonError { message, path }`, predeclared, with one issue and the JSON Pointer to it (10.4). The value of a `:name` in a route is `req.param("name")`, and `req.path` is the path itself; `req.query(name)` and `req.header(name)` are optional (10.3). A handler has the type `fn(http.Request) -> http.Response ! Failure`, so it may fail with any error, and a function that cannot fail, or fails with fewer errors, stands where one that may fail is wanted (4.11). A `JsonError` that leaves a handler is answered with 400 and a body `{"error", "path"}`; any other error with 500, written to standard error and not to the client (10.3). `app.serve(port:, workers:)` runs one event loop in each of `workers` processes, one for every core by default, until tasks exist to do the same in one process; a handler shares nothing with another request either way (7.1, 10.3). A field that holds a function is called as it is named: `route.handler(request)` (4.6). With more than one worker, the process that called `serve` answers nothing and replaces a worker that dies, which is how a panic in a handler fails its own requests and not the server, until isolated tasks do the same for one request (5.9, 10.3). `req.rest()` is what a `*` at the end of a route stands for, and is `none` for a path that climbs out with `..`; `app.files(path, directory)` and `http.file(path)` serve files; `req.form(name)` and `req.cookie(name)` are optional (10.3). `koo build --static` links a program with everything it needs, on Linux (13.3). An actor is held by one thread at a time, for as long as a message of it runs. To ask is to hold the actor, call the method on its state and let go, on the asker's own thread; an asker that finds the actor held waits, and if the wait could never end, because the thread that holds it waits in turn for an actor this thread holds, that is the cycle of Section 7.3, and panics. What is told is kept by the actor and run by a pool of threads, one for each processor, in the order told and before the actor is next asked; the program ends when `main` has returned and all of it has run. A mailbox holds 1,024 messages, or what `@mailbox(n)` says; a thread that tells an actor whose mailbox is full waits for room, a worker leaving another thread in its place meanwhile, and a wait that could not end, as when an actor tells itself, panics; `try_send` returns whether there was room. `Duration` and `Time` are built, with the literals and operators of Sections 3.3 and 10.6; `std/time` has `time.now()`, `time.uptime()`, which is the monotonic clock as the `Duration` since the program started, and `time.sleep(length)`; `Duration.parse`, `Time.parse` (RFC 3339), `Time.from_unix` and `unix` convert, and a duration has `seconds`, `millis`, `micros` and `nanos` (10.6). A monitor thread starts another worker when actors have waited ten milliseconds with every worker busy (13.5). What crosses from one actor to another is not copied: it is frozen, counted atomically from then on, and never changed where it is, a change making a copy first as for any value held twice (5.1). A function cannot cross, since what it captured cannot be frozen. A panic in a message that was told, or that came from another process, ends the message: the actors it was in the middle of start again from the state they started with, and what the message held is not freed. Another process (a worker of a server) asks through the socket of the process that has the actor, with the arguments as text, so a message is of the types that have a form as JSON, with handles among them (7.3, 7.4). The functions of the compiler's own making have names with `__`, and the types `Actor<Name>Send` and `Actor<Name>State` belong to the actor `Name` (7.3). For `compare`, every NaN is one value that follows every number, whatever its sign: `totalOrder` tells NaNs apart by a sign that processors choose differently, and a program sorts the same everywhere (4.2). `std/sql` and `std/sql/postgres` are built, with the client for PostgreSQL in the support library, not yet in Koo. `db.query` returns `[sql.Row]`, all at once, until resources exist to stream them; a value of a row is read by its column's name and the type wanted, `row.int("qty")`, with `row.maybe_int` for a column that may be NULL, and `T.from_row(row)` is written for a struct whose fields are `int`, `float`, `bool`, `string`, `Time` or optionals of them, which are also the types of a parameter. A parameter is sent as text with its type left to the server. `sql.ErrorKind` has `Timeout` as well, for a statement that ran out of time, and a `sql.Error` has the server's `code`. `db.transaction(body, isolation:)` takes one of `sql.Isolation.ReadCommitted`, `RepeatableRead` and `Serializable`, has the value of `body`, fails with the errors of `body` and with `sql.Error`, and refuses to commit a transaction in which a statement failed even if `body` swallowed the error; a `sql.Tx` cannot be sent to an actor. `postgres.connect` makes its first connection before it returns, takes session settings as `options` in the address, and gives a server ten seconds to accept a connection; a worker process of a server has a pool of its own (10.3). A type parameter written after `!` stands for the errors of the function passed where it appears, which may be none; what the generic function does with an error of that function is then left out of the function made for the call. Where a function parameter's type lists its errors, a lambda that fails with fewer is accepted. `fail e` passes on an error bound by `! e` or `else e =>`, of whichever of several types, when the function's set has them all (6.3, 6.4). `()` is a pattern for the value of a function that returns nothing. A variant of an enum of a standard package is written `sql.ErrorKind.Constraint`, in expressions and in patterns, and a struct of one `http.Response { ... }` (8.2). Of `std/hash` there is `hash.sha256`; of `std/encoding`, `hex`, `base64`, `base64_url` and `url`, each with a `from_` that fails with `ParseError`; of `std/crypto`, `hmac_sha256`, `equal`, and `hash_password` with `verify_password`, which are PBKDF2 with HMAC-SHA-256 and 600,000 rounds until Argon2 is linked. All take and return `[byte]`, and the digests are in the support library, by the processor's instructions for SHA-2 where an arm64 processor has them. `std/random` is a package of its own: `random.int(low, high)`, which leaves `high` out as a range does, `random.float()`, `random.bytes(count)` and `random.uuid()`, with `pick()` and `shuffle()` on a list, all from the system's source of randomness for keys, of which a thread takes 256 bytes at a time (10.1). `std/log` is built: `log.debug`, `log.info`, `log.warn` and `log.error` take a `log.Message`, the second template type, whose values become fields named by the expression where it is a name or a path of fields and `value1`, `value2` by place otherwise; a call of one of the four is compiled as a test of the level around the call, so a line that is not wanted evaluates nothing. Lines go to the standard error output, each in one write; `KOO_LOG_LEVEL`, `KOO_LOG` (`text` or `json`) and `KOO_LOG_TIME` in the environment decide what is written, and `log.set_level` overrides the first (3.3, 10.1). `std/sql/postgres` encrypts with the OpenSSL of the system the program runs on, loaded when a connection first needs it and not linked in, until rustls is: `sslmode` in the address is `disable`, `prefer` (the default: encrypted if the server will and the library is there), `require`, `verify-ca` or `verify-full`, with `sslrootcert` for a file of authorities, which makes `require` mean `verify-ca` as it does for libpq; TLS 1.2 is the oldest version spoken. A program linked with `--static` loads nothing and has no TLS (10.1, 10.3).

### 15.2 Changes in 0.4

Section numbers in 15.2, 15.3 and 15.4 refer to the versions they describe.

Version 0.4 responds to a design audit of 0.3 against the language's stated goals. Unlike 0.3, it changes design decisions.

**Soundness.** Exclusivity (5.4) closes the hole by which a `var` argument or a loop body could free a value that a borrow still referred to: an overlapping read is copied, and two overlapping writes are rejected. Freezing is specified as deep, with its cost (5.6); 0.3 froze one buffer and left inner reference counts racing. Chain fusion is restricted to effect-free lambdas (4.5). Absence nests (4.8), because a non-nesting `T?` is unsound in generic code. The 0.3 reference example violated 0.3's own borrow rule; the rule is gone (5.3).

**Simpler ownership.** A borrowed parameter may be used freely; a use that keeps it copies it, and `take` is an optimization except for resources (5.3). `var` is written at the call site (5.3). The cost model is stated (5.11).

**Errors.** Results are ordinary values that cannot be ignored, rather than values that cannot be stored (6.3). `fail` returns an error; `return` no longer does both jobs (6.1). `else name =>` binds the error (6.4). Named error sets and `! Failure` (6.2, 6.3). Adding an error type to a public set is a breaking change, with no exception to the stability promise; packages grow their failures through open enums (4.7, 6.3, 8.5). `if name := e` and `while name := e` (6.7). `defer`, fallible `close`, and a cleanup window in which cleanup may perform I/O under cancellation (6.8, 7.7).

**Types and syntax.** `char`, text blocks and interpolation formats (3.3). `()` is the empty tuple, so generic code can take functions that return nothing. Enum variants are namespaced, and public enums are open unless `@closed` (4.7). `Self` in interfaces; user types become ordered through `compare`; `float` keeps IEEE operators and is no longer `Hashable`, since its hash could not agree with its equality (4.2, 4.9). Error arms in a `match` on a result begin with `!`, and a lowercase name in a pattern is always a binding (6.4, 6.6). Inline struct types (4.6). Type-level function declarations and receiver kinds in interfaces (6.1, 4.9). An iteration protocol (6.7). String positions are byte offsets (4.4). `float.from(int)` is infallible; `Decimal` arithmetic follows IEEE 754 instead of panicking past 34 digits (4.2, 4.3). The day unit is removed, ending the clash between `7d` and `110d`. Template types make interpolated SQL parameterized by construction (3.3, 10.3). `else` is right-associative; `|` continues a line.

**Concurrency.** Tasks are stackless and suspension is inferred by the compiler, replacing copying stacks and signal preemption, neither of which exists on WebAssembly or is well supported by the code generators (7.1, 13.5). `isolate` turns a panic at a task boundary into an error, and `std/http` isolates every request (5.9, 10.3); in 0.3 one out-of-range index stopped the server. `Shared<T>` gives read-mostly state without an actor's queue (7.5). Actors can start tasks, have a defined lifetime, are never cancelled part-way through a method, and detect ask cycles of any length (7.3, 7.7). The default supervision policy is a bounded restart, and an actor started with a resource cannot be restarted (7.4). `race` returns the first success. `map_parallel` has a default limit and a variant that returns every outcome (7.2).

**JSON.** Section 10.4 is new: a fixed mapping for every type, rename policies, optional and three-state fields, tagged enums, error paths, validation, streaming, schemas, and `Json` literals. `std/json` is available in core mode. Protobuf is generated, not derived. All codecs come from one deriver mechanism, which runs as WebAssembly (10.5).

**WebAssembly.** Tier 1, with its own section (9.5): `wasm32-wasip3` components served through `wasi:http`, and `wasm32-none` modules for plugins. Cranelift does not emit WebAssembly, as 0.3 assumed; debug Wasm builds have a direct emitter (13.2).

**Scope.** The compiler and the runtime are in Rust through 1.0; TLS, cryptographic primitives and the allocator wrap vetted implementations (10.1, 13.4). Kernel modules and `no_alloc` are removed from 1.0. LLVM moves to 0.2 and each early milestone ends in a published benchmark gate (13.7). Benchmarks add tail latency, memory and reference-count traffic (13.6).

### 15.3 Changes in 0.3

A consistency pass over 0.2. No design direction changed; the following were contradictory, ambiguous or undefined and are now fixed.

**Grammar.** Added an operator precedence table (3.4), an attribute grammar (3.8), a disambiguation section (3.9) and a prelude (4.1). The keyword `new` is replaced by `distinct`, freeing `new` for constructors. `is` tests a type and binds nothing; binding is `match` only. Struct literals in conditions must be parenthesized. Labeled loops have syntax. Line continuation depends only on the start of the next line. Contextual keywords are `link`, `after`, `all`, `race`; `@mailbox` and `@supervise` are attributes. Lists are sliced with `xs[a..b]`, strings with `s.slice(a, b)`; `[byte]` is sliced, not `.slice`d, in Section 11.1.

**Types.** Conversions are `T.from(x)` with stated fallibility, including `float.from(int)`. `Ordered` and `Hashable` for `float` and `Decimal` are specified. `Money` has methods, not operators, because operators cannot return errors. Distinct types inherit methods. Resources: `close` consumes `self`; containers of resources are move-only. `T??` is a compile error. Fields invisible at a construction site must have defaults. `to_string()` replaces `string()`. Error types satisfy `Failure`; `Any` is defined once.

**Memory model.** Section 5.5 now states the two buffer states (owned, non-atomic count; frozen, atomic count) and the one transition between them, resolving the contradiction between 0.2's Sections 5.5 and 5.7. Borrow parameters may be copied into locals; the restriction and its reason are stated. Receiver kinds follow parameter kinds. Panic propagation is specified for each kind of task.

**Errors.** Error-set results are not storable values; the four consuming operations are `?`, `else`, `match`, `wrap`. Error conversion is `wrap(fn)`, replacing `as`, which did not parse as 0.2 claimed. `?` on `none` outside an `T?`-returning function is a compile error. `main` may declare an error set. Variadics are defined. The `match` example no longer contains a semicolon.

**Concurrency.** Actor method parameters are implicitly `take`. Start arguments are the fields without defaults. A panic inside an ask propagates to the asker as a panic, not as cancellation. `race` returns the first result, success or error. `with_timeout` is the only timeout mechanism; `task.deadline` is removed. `spawn` outside `scope` is a compile error; `?` and `return` inside `scope` are specified. Channel `close` semantics are Go's; the unimplementable "error if a sender remains" is removed. `recv()` returns `T?`. `for_parallel` is removed as undefined.

**Packages and runtime.** The import major must match the manifest. Capability verification is defined in terms of imports and `unsafe`. Core mode's available `std` packages are those marked *core* in 10.1. `sql.Pool` is a handle and `sql.Conn` the resource, consistently. `std/http` handler and `on_error` types are stated; `on_error` takes a typed lambda rather than a type as a value. Target names are uniform (13.3). The pipeline checks declarations before derivation and bodies after. Vague quantities ("a few hundred bytes", "roughly") are replaced by stated targets or removed.

### 15.4 Changes in 0.2

Version 0.2 responded to an external design review of 0.1: the two-word parameter convention and `resource` types; cancellation and panics as unwinding rather than values; `else` as the fallback operator; removal of `|>` and the percent literal; `[K: V]` maps and `=>` arms; bounded mailboxes and specified supervision; the connection pool as a non-actor; `all` and `race` as intrinsics; `scope` as a block; copying stacks and preemption; task context; `std` reduced to protocols with vendor APIs as first-party packages; `@derive`; core mode requiring an allocator; static Linux binaries; `io_uring` opt-in; Wasm at Tier 2; and benchmark-defined performance targets.
