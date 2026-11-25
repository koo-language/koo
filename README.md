# Koo

Koo is a compiled programming language for backend services. It aims for the performance of Rust with the ease of a scripting language: no garbage collector, no borrow checker, no `async`/`await`. Programs compile to native binaries for x86-64 and arm64, and to WebAssembly.

> **Status: early.** The language is specified in [`SPEC.md`](SPEC.md) (version 0.5, draft) and its grammar in [`spec/grammar.ebnf`](spec/grammar.ebnf). The compiler builds and runs programs written in the part of the language described under [What works today](#what-works-today). Two documents teach that part, and the test suite runs every program in both: the [tour](docs/tour.md), which is short, and [The Koo Programming Language](docs/book/README.md), a book in the manner of Kernighan and Ritchie's, with a tutorial, a chapter for each part of the language, exercises, a reference manual and a summary of the library. Most of the standard library, tasks and channels, packages and WebAssembly are specified and not yet built. It is not ready for production use.

## Try it

Koo runs on macOS (arm64) and on Linux (x86-64 and arm64). It needs clang 15 or newer on your `PATH` to turn a program into an executable (`xcode-select --install` on macOS, `apt install clang` on Debian and Ubuntu). If your clang has another name, say so: `KOO_CLANG=clang-19`.

**From a release.** There is no release yet. When there is one,

```sh
curl -fsSL https://raw.githubusercontent.com/koo-language/koo/main/install.sh | sh
```

will fetch the compiler for your system, check it against its published checksum, and put it in `~/.koo/bin`. The Linux build is one static binary that runs on any distribution, whichever C library it has. A tag `v...` makes a release ([`release.yml`](.github/workflows/release.yml)).

**From source**, which today is the only way, with a recent stable Rust toolchain:

```sh
cargo build --release
./target/release/koo run examples/hello.koo
./target/release/koo run examples/wordcount.koo -- README.md
./target/release/koo build --release examples/inventory.koo -o inventory
./target/release/koo test tests/test/stack_test.koo
./target/release/koo run tests/packages/shop        # a program in several files
./target/release/koo run examples/http/orders.koo   # a JSON API on localhost:8080
./target/release/koo run examples/http/counter.koo  # a server whose requests share an actor
docker build -f examples/http/Dockerfile -t orders . # the same API as a 2.4 MB container
```

Then read the [tour](docs/tour.md), which goes through the language in about fifteen short programs, or the [book](docs/book/README.md), which goes through it properly.

```koo
type Item = { sku: string, qty: int, price: float }

error UnknownSku { sku: string }
error OutOfStock { sku: string, wanted: int, have: int }

fn find(items: [Item], sku: string) -> int ! UnknownSku {
    for i in 0..items.len {
        if items[i].sku == sku { return i }
    }
    fail UnknownSku { sku }
}

fn sell(items: var [Item], sku: string, qty: int) -> float ! UnknownSku | OutOfStock {
    i := find(items, sku)?
    have := items[i].qty
    if have < qty { fail OutOfStock { sku, wanted: qty, have } }
    items[i].qty -= qty
    return float.from(qty) * items[i].price
}

fn main() {
    var items := [Item { sku: "pen", qty: 120, price: 1.25 }]
    before := items                      // a value: later changes do not touch it
    match sell(var items, "pen", 30) {
        total => print("sold for {total:.2}")
        ! OutOfStock { have, .. } => print("only {have} left")
        ! e: UnknownSku => print("no such item: {e.sku}")
    }
    print("before: {before[0].qty}, after: {items[0].qty}")
}
```

## A service

[`examples/service`](examples/service) is a small orders service of about a hundred lines, and the pattern for one of your own. Its settings are a struct read from the environment, its state is in PostgreSQL, it listens where `PORT` says, it logs lines that a program can read, and it stops cleanly when it is asked to. Its README goes through the twelve factors one by one, and the tests start the service with nothing but its environment and check what it answers.

```koo
type Settings = { database_url: string, pool: int = 10 }

fn main() ! EnvError | sql.Error | http.Error {
    settings := Settings.from_env()?
    db := postgres.connect(settings.database_url, pool: settings.pool)?

    var app := http.App.new()
    app.get("/orders/:id", req -> {
        id := int.parse(req.param("id")) else {
            return http.status(400, "an order has a number\n")
        }
        row := db.query_one("select * from orders where id = {id}")? else {
            return http.status(404, "there is no order {id}\n")
        }
        return http.json(Order.from_row(row)?)
    })
    app.serve()?
}
```

- A handler that waits for the database does not hold up its worker: one worker answers many requests at a time.
- A handler that panics fails its own request, with 500, and nothing else.
- `{id}` in the query is a parameter, never SQL.
- `docker compose -f examples/service/compose.yaml up --build` runs it with a database.

## Where it is going

The specification describes the whole language. This service is its reference example. It does not compile yet as written, for want of `distinct` types, `Decimal` and `app.on_error`; the same service with an `int` for the id and a `match` in the handler does, and [Chapter 11 of the book](docs/book/11-database.md) has it, run against a real database.

```koo
import std/os
import std/http
import std/sql
import std/sql/postgres

pub type Id = distinct int
pub type Order = { id: Id, customer: Id, total: Decimal, placed: Time }
error NotFound { id: Id }

fn get_order(db: sql.Pool, id: Id) -> Order ! NotFound | sql.Error {
    row := db.query_one("select * from orders where id = {id}")? else fail NotFound { id }
    return Order.from_row(row)?
}

fn main() ! sql.Error | http.Error {
    url := os.env("DATABASE_URL") else "postgres://localhost/orders"
    db := postgres.connect(url, pool: 20)?
    var app := http.App.new()
    app.get("/orders/:id", req -> {
        id := Id.parse(req.path("id"))?
        return http.json(get_order(db, id)?)
    })
    app.on_error((e: NotFound) -> http.status(404, "order {e.id} not found"))
    app.serve(port: 8080)?
}
```

- The `{id}` in the query is a bound parameter, not text spliced into SQL.
- `Order` is read from a row and written as JSON with no annotations.
- A malformed id gets a 400 that names the problem, and a bug in one handler fails one request, not the server.
- Nothing is marked `async`, and no value is shared between requests.
- The same source builds as a native server and as a WebAssembly component.

## Design

- **Rust-class performance.** No garbage collector, monomorphized generics, inline value types, optimized code via LLVM. The specification defines the benchmarks that hold this to account and lists where Koo's design costs more than Rust's.
- **Easy to learn.** Values behave like values: assigning or passing one never creates a hidden alias, and the compiler works out ownership. There are no lifetimes to write.
- **Safe by default.** No null, no data races, no ignored errors, no undefined behavior outside `unsafe`.
- **Concurrency without ceremony.** Every function can block and none is colored. State is owned by an actor or published as a snapshot.
- **JSON is native.** Any type decodes from and encodes to JSON, validates, and describes itself as a schema, with no annotation and no dependency.
- **One way.** One string type, one error mechanism, one formatter, one package manager, one test runner, one HTTP server.

## Targets

| Target | Output | Tier at 1.0 |
| --- | --- | --- |
| `x86_64-linux`, `arm64-linux` | Static binary (musl) by default, glibc optional | 1 |
| `arm64-macos` | Native binary | 1 |
| `wasm32-wasip3` | WebAssembly component (WASI 0.3, `wasi:http`) | 1 |
| `wasm32-none` | Core WebAssembly module for plugins and embedding | 1 |
| `x86_64-macos`, `x86_64-windows`, `arm64-windows` | Native binary | 2 |

The table is where the language is going. Today the compiler builds for the machine it runs on, and that machine is macOS on arm64, or Linux on x86-64 or arm64. The whole test suite passes on macOS and on arm64 Linux. On x86-64 Linux it was run under emulation, where everything passes but one test of the parser that the emulator runs out of memory on; it has not been run on x86-64 hardware yet. On Linux, `koo build --static` links everything in, so the program runs in an empty container ([`examples/http/Dockerfile`](examples/http/Dockerfile) makes one of 2.4 MB). Not yet: building for another machine than the one at hand, Windows, and WebAssembly.

## What works today

The compiler implements the part of the language below and rejects everything else with a message that says so.

| Area | In |
| --- | --- |
| Types | `int`, `float`, `bool`, `byte`, `string`, `Duration` (`250ms`, `1h30min`) and `Time`, lists `[T]`, maps `[K: V]`, `Set<T>`, tuples, optionals `T?`, structs with default fields, enums with fields, error types and named error sets, results `T ! E`, function types |
| Functions | Free functions, methods, type-level functions, the three parameter kinds (borrow, `var`, `take`), named arguments, default values |
| Generics | Generic functions, structs and enums, compiled once for each type they are used with; interfaces as constraints, satisfied by having the methods; `Ordered`, `Equatable` and `Hashable` derived for every type that can have them; a type parameter for the errors of a function that is passed, as in `fn retry<T, E>(body: fn() -> T ! E) -> T ! E` |
| Lambdas | `x -> e` and `(a, b) -> { ... }`. One passed straight to a function works on the variables around it; one that is kept holds copies |
| Values | Value semantics throughout: copying is lazy, a change copies first if the storage is shared, and the uniqueness check is hoisted out of loops |
| Chains | `xs.filter(f).map(g).sum()` is one pass that builds no list, when the functions only compute; otherwise it runs step by step, as written |
| Errors | `fail`, `?`, `else`, `else e =>`, `match` on results with `!` arms, and `fail e` to pass on an error that was caught; unhandled results and incomplete error sets are compile errors |
| Control flow | `if`, `while`, `for` over ranges, lists, maps and sets, binding conditions (`if x := e`, `if (a, b) := e`), `break`, `continue`, `match` with guards and exhaustiveness checking, `if` and `match` used for a value |
| Safety | Checked integer arithmetic, bounds-checked indexing, no null; a panic reports the file and line |
| Text | Interpolation with formats (`{x:8.2}`, `{id:05}`, `{name:>10}`), text blocks, raw strings, a text form for every value |
| Built in | About sixty methods on lists, strings, optionals and numbers (`map`, `filter`, `fold`, `sort_by`, `join`, `split`, `trim`, `replace`, ...), written in Koo |
| JSON | `value.to_json()` and `Type.from_json(text)` for every struct and enum made of numbers, strings, booleans, lists, optionals, maps with string keys and other such types, with nothing to import or annotate. A text that does not fit is refused with the path to the mistake (`/items/3/qty`) |
| HTTP | `std/http`: a server with routes (`/orders/:id`, `/files/*`), queries, headers, forms, cookies, bodies whole or in chunks, JSON in and out, files from a directory, 404 and 405 by itself, and a log of requests. One event loop per core, each in a process; a handler that waits, for a database, another server or a time, is left on a stack of its own while its worker answers other requests; a handler that panics fails its own request with 500 and nothing else. A body that does not fit its type is answered with 400 and the path |
| HTTP client | `http.get`, `post`, `put`, `patch`, `delete` and `fetch` return the answer of another server: status, headers, body, and `reply.json()` into a type. HTTPS with the certificate checked against the system's authorities and the host's name. One time limit for the whole request, thirty seconds unless it is said. An answer with a status of failure is an answer; an error is no answer, with a `kind` that says whether it was the address, the connection, the time or the protocol. Connections are kept and used again. Redirections are not followed |
| Actors | `actor Name { fields, methods }`, `Name.start(...)`, ask (`handle.method(...)`), tell (`handle.send.method(...)`), `handle.stop()`, typed errors across the boundary, handles in messages, restart after a panic, and a panic in place of a cycle of asks that would wait for ever. What actors are told runs on every core, and what they hand each other is shared between threads without being copied or locked. An actor's mail is limited (`@mailbox(n)`): `send` waits for room and `try_send` does not. One actor serves every process of a server. An actor lives in the process that starts it, and asking it from there costs about what a call does |
| SQL | `std/sql` with `std/sql/postgres`: a client for PostgreSQL written for this runtime, with nothing to install. A query is a string literal whose `{values}` are sent as parameters and never become SQL, so there is nothing to inject; a `string` where a query is wanted is a compile error. Rows by column (`row.int("qty")`) or as a struct (`Order.from_row(row)`), NULL as an optional, `db.transaction(tx -> ...)` that commits when its function returns and rolls back when it fails or panics, three isolation levels, a pool of connections shared by every thread and actor, reconnection after the server restarts, and one error type whose `kind` says what went wrong. Passwords by SCRAM-SHA-256, MD5 or in the clear. TLS by the system's OpenSSL, loaded when first needed, with PostgreSQL's `sslmode` (`prefer` by default, `require`, `verify-ca`, `verify-full`) and `sslrootcert`; a `--static` program has none. Not yet: rows streamed as they arrive |
| Standard packages | `std/os` (arguments, environment, exit), `std/io` (files, standard input and output), `std/time` (the clock, a clock for measuring, sleeping), `std/http`, `std/sql`, `std/hash` (SHA-256), `std/encoding` (hex, Base64, addresses), `std/crypto` (HMAC, a comparison that leaks no timing, password hashing), `std/random` (numbers, bytes and UUIDs from the system's source for keys), `std/log` (levels, text or JSON lines in which the values of a message are fields of their own, nothing computed for a level that is off), `std/test` (assertions) |
| Packages | A directory is a package: its files are one program and share one namespace. `koo run dir`, `koo test dir` |
| Diagnostics | Every error says what is wrong, marks the place, points at the declaration or the earlier line that explains it, and shows the corrected line where there is one thing that was meant. A name that is nearly right gets a "did you mean". Syntax from other languages (`let`, `function`, `class`, `elif`, `&&`, `x++`, `'text'`, `null`, `try`, C's `for`, `int x = 5`, `.length`) is named for what it is and answered with the Koo that was meant |
| Tooling | `koo run`, `koo build`, `koo check`, `koo test`, `koo parse`, with `--release`, `--emit-llvm`, `--sanitize` and `--sanitize=thread` |
| Editors | Syntax highlighting for Visual Studio Code, in [`editors/vscode`](editors/vscode) |

Not yet: importing one package of your own from another, the package manager, the rest of the standard library (dates and time zones, sockets; and in the server HTTPS, HTTP/2 and streamed bodies), interface values, tasks, channels and `select`, and of actors the generic ones and `spawn` inside one, `Decimal`, resources, `defer`, variadic parameters, the WebAssembly targets, Windows, the formatter and the language server.

What has been checked, and how:

- The test suite builds every test program unoptimized and optimized, compares its output, and fails if it ends with memory still allocated. The same programs pass under AddressSanitizer.
- Every program in the [book](docs/book/README.md) and the tour is built and run by the test suite, and what it does is compared to the letter with what the page says: what it prints, what a panic or a failing `main` writes, what the compiler says of a program that is wrong on purpose, what `koo test` reports, and what a server answers to the requests listed. The outputs on those pages are written there by the programs (`KOO_BLESS=1 cargo test the_book_is_true`), not typed.
- The programs that use a database are run against PostgreSQL 16 when the tests are given a server (`KOO_TEST_POSTGRES=postgres://...`), each in an empty schema; without one they are checked by the compiler and not run. The three ways of proving a password were each tried against a server set up to ask for it.
- Every test program with actors is also built with ThreadSanitizer and run, on macOS and Linux, with no report. With the step that freezes a message turned off, the same run reports twelve races, so the check does see what it is there to see.
- The compiler has had four independent adversarial reviews, each of which wrote programs meant to break it. The third found sixteen problems, four of them memory-unsafe, in lambdas, generics and maps. The fourth went back over those fixes with about a hundred new programs and found them sound, and found seven new problems elsewhere: two memory-unsafe ones in `compare`, three ways to crash or stall the compiler with recursive or fast-growing types, and two wrong results (sorting floats with a NaN among them, and a tuple that skipped an element's side effect). All were fixed and became regression tests (`tests/run/blocks.koo`, `tests/run/review4.koo`, `tests/fail/blocks.koo`, `tests/fail/review.koo`, `tests/fail/review4.koo`). The fixes for the fourth have not been reviewed independently yet.
- Known and not fixed: a `var` method cannot be called on a temporary; inside another pattern a name takes an optional as it is, so there is no pattern there that binds the value of an optional only when it has one.
- That is far from proof. The compiler is new, nobody outside the project has used it, and it has not been fuzzed at scale. Expect bugs.

## Benchmarks

`python3 bench/run.py` builds each program in [`bench`](bench) in Koo and in Rust, checks that the two print the same thing, and times them. On an Apple Silicon Mac with Apple clang 21 and rustc 1.95, best of five runs:

| Benchmark | What it stresses | Koo | Rust | Rust with overflow checks | Koo / Rust | Koo / Rust with checks |
| --- | --- | --- | --- | --- | --- | --- |
| `sort` | Sorting three million integers | 0.045 s | 0.063 s | 0.064 s | 0.72 | 0.71 |
| `wordfreq` | A map from strings to counts | 0.100 s | 0.120 s | 0.119 s | 0.83 | 0.84 |
| `strings` | Building and formatting text | 0.283 s | 0.308 s | 0.313 s | 0.92 | 0.91 |
| `sieve` | A large list of flags | 0.552 s | 0.547 s | 0.542 s | 1.01 | 1.02 |
| `nbody` | Floating point, a list of structs | 0.669 s | 0.645 s | 0.645 s | 1.04 | 1.04 |
| `fib` | Calls and integer arithmetic | 0.890 s | 0.709 s | 0.888 s | 1.26 | 1.00 |
| `matmul` | Nested lists | 0.099 s | 0.077 s | 0.077 s | 1.29 | 1.29 |
| `collatz` | Integer arithmetic | 0.957 s | 0.621 s | 0.942 s | 1.54 | 1.02 |
| `sortby` | Sorting 1.5 million records by a key | 0.069 s | 0.044 s | 0.044 s | 1.58 | 1.58 |
| `pipeline` | `map`, `filter` and `fold` over a list | 0.027 s | 0.016 s | 0.028 s | 1.66 | 0.95 |
| `errors` | Results and `?` in a hot loop | 0.068 s | 0.038 s | 0.052 s | 1.78 | 1.31 |

- Koo is level with Rust or ahead of it on five of the eleven. Its map, which is written in Koo, is faster on `wordfreq` than Rust's `HashMap` with its default hasher.
- On `fib`, `collatz` and `pipeline` it is level with Rust built with `-C overflow-checks=on`. Koo checks integer overflow in every build (Section 4.2 of the specification) and Rust's release builds do not; the gap to default Rust on those three is the price of that check. On `pipeline` the check also keeps the loop from being vectorized, which Rust's is.
- `pipeline` was seven times slower than Rust until the compiler learned to fuse a chain: `xs.map(f).filter(g).fold(...)` is now one loop that builds no list, when `f`, `g` and the last function only compute. A chain whose functions print or change variables still runs step by step.
- `sort` on a list of integers uses the support library's unstable sort, which is sound because equal integers cannot be told apart. Every other sort is the merge sort written in Koo, which `sortby` measures: 1.6 times slower than Rust's.
- On `matmul` and `errors` it is about 1.3 times slower than Rust even with checks on. In `matmul` the inner loop reads a list's length from memory on every pass, where Rust keeps it in a register. In `errors` the whole loop is inlined and allocates nothing; the cause has not been tracked down.
- These are eleven small single-threaded programs, not the benchmark suite the specification defines in Section 13.6. They say nothing yet about services, concurrency or large programs, and the optimizations the specification plans for reference counts are mostly not implemented.

### A server

`sh bench/http/run.sh` starts the same "Hello, world!" server in Koo, in Rust (axum 0.7 on tokio) and in Node 24, each held to one core, and sends each requests over 16 kept-open connections from one load generator ([`bench/http/load.rs`](bench/http/load.rs)) on the same machine:

| Server | Requests a second |
| --- | --- |
| Koo, one worker | 236,000 |
| Rust, axum, one thread | 200,000 |
| Node | 112,000 |

- The three move together from one run of the script to another with what else the machine is doing: an earlier session measured 305,000, 270,000 and 133,000. What holds is the order and roughly the ratios: Koo a little ahead of axum, and about twice Node.
- This measures the server's own cost for a request: reading it, routing it, calling the handler and writing the response. It says nothing about what a handler does, and a real handler does more than return thirteen bytes.
- A handler that asks a database waits, and a handler that waits does not hold up its worker: it is left on a stack of its own while the worker answers other requests. With PostgreSQL 16 in a container on the same machine, 64 connections from `ab` and a pool of 16, one worker answered 11,900 requests a second that each read a row by its key, and 1,160 that each ran a statement of 10 ms (the pool allows 1,600); four workers answered 17,800 and 2,400. Before handlers could wait side by side the same four workers answered 6,200 and 280. A handler that never waits pays nothing for this: the "Hello, world!" figure is the same with it and without.
- The load generator runs on the same eight cores as the server, so the numbers are bounded by it as well. With every core serving, Koo and axum measure about the same as each other and no more than on one core: the generator, not either server, is then what is being measured.
- Koo's server is one event loop per process (`kqueue` on macOS, `epoll` on Linux), with parsing and routing in the support library, and by default one process per core under one that restarts them. Nothing in it waits on a slow client, but a handler that itself waits (on a file, say) holds up the other requests of its worker until tasks exist.

### Actors

`sh bench/actors/run.sh` runs five programs in Koo and in Erlang (OTP 29, a `gen_server` for each actor), on the same machine with 8 cores. Times are from start to end by the clock, the best of three runs.

| | Koo | Erlang |
| --- | --- | --- |
| Ten million asks of one actor, one after the other | 0.08 s (8 ns each) | 8.3 s (830 ns each) |
| A million actors started, asked twice and kept | 0.11 s, 194 MB | 3.7 s, 2,800 MB |
| A message passed round a ring of a thousand actors, twenty million times | 0.99 s (50 ns a hop) | 5.0 s (250 ns a hop) |
| Sixteen actors tell sixty-four others 12.8 million things at once | 0.48 s, 44 MB | 1.1 s, 224 MB |
| Sixteen actors each count the primes among a million numbers | 0.68 s | 2.4 s |

- An ask in Koo runs on the asker's own thread, with the actor held meanwhile: when nobody else wants the actor that is two atomic exchanges around a call. A call in Erlang always crosses between two processes that the virtual machine schedules. That is the first row, and it is the widest gap.
- What is told runs on a pool of threads, one for each processor. A message is a small function value put on the actor's list, with nothing copied: what it carries is frozen, and shared. In Erlang a message is copied to the heap of the process it goes to.
- The last row is mostly the difference between compiled code and Erlang's virtual machine, with both using every core.
- An actor's mail has a limit, 1,024 messages unless it says otherwise with `@mailbox(n)`, and a sender that reaches it waits. That is why the fourth program holds 44 MB: before there was a limit it held 590 MB, with the same time.
- A method that computes for long keeps its thread, and when every thread is busy and actors wait, another is started within a hundredth of a second, so one slow actor does not hold up the rest.
- Where Erlang is ahead: it can interrupt a process that runs too long, where Koo can only work around one; it keeps each process's memory apart so that one can be collected or killed alone, has supervision trees, and runs across machines. Koo does none of that yet.
- A worker of a server is a process, and asks an actor through the process that owns it, over a socket, which costs a few microseconds: the server in [`examples/http/counter.koo`](examples/http/counter.koo) answers about 125,000 requests a second with an ask in each.

Compile times are not comparable yet: every program here is small enough that both compilers finish in about a tenth of a second.

## Roadmap

| Milestone | Scope |
| --- | --- |
| 0.1 | Parser, type checker, ownership pass, LLVM backend, `koo run`, `koo test`, first published benchmarks against Rust |
| 0.2 | Cranelift debug backend, generics, interfaces, lambdas, C FFI (generics, interface constraints and lambdas are built already) |
| 0.3 | Tasks and scheduler, WebAssembly targets, `std/http`, `std/json` |
| 0.4 | Actors and supervision, `std/sql/postgres`; the reference example runs natively and as a Wasm component |
| 0.5 to 0.9 | Formatter, language server, package manager, gRPC, TLS, Windows, public beta |
| 1.0 | Stability commitment begins |

Section 13.7 of the specification has the full list. The compiler is part of the way through milestone 0.1.

## Building and testing

```sh
cargo test                # everything, including the programs in tests/, docs/tour.md and every code block in SPEC.md
python3 bench/run.py      # the benchmarks, against Rust
```

The end-to-end tests build each program in `tests/run` unoptimized and optimized, run it, and fail if it prints the wrong thing, exits with the wrong status, or ends with memory still allocated. Each program in `tests/fail` must be rejected with exactly the errors it lists, the 151 mistakes in `tests/diag` must be explained word for word as the files there say, a server is started and sent some forty requests, well formed and not, on one process and on three, and each program in the tour must print what the tour says.

| Path | Contents |
| --- | --- |
| `SPEC.md` | The language specification |
| `spec/grammar.ebnf` | The grammar, normative |
| `docs/tour.md` | A tour of the language as it is built today |
| `compiler/koo-syntax` | Lexer, syntax tree and parser |
| `compiler/koo-compiler` | Type checker, LLVM code generator, and the prelude (`src/prelude.koo`) |
| `compiler/koo-rt` | The support library linked into every program |
| `compiler/koo` | The `koo` command |
| `examples`, `tests`, `bench` | Example programs, end-to-end tests, benchmarks |

## Contributing

The most useful contribution today is review of the specification: open an issue for anything unclear, inconsistent or wrong. Changes to the language's syntax or semantics go through the proposal process described in [`GOVERNANCE.md`](GOVERNANCE.md). Commits are signed off under the [Developer Certificate of Origin](https://developercertificate.org); there is no contributor license agreement.

## License

Koo is dual-licensed under the [Apache License 2.0](LICENSE-APACHE) and the [MIT License](LICENSE-MIT), at your option. See [`LICENSE`](LICENSE).

The project commits to never relicensing under a source-available, non-commercial or otherwise restrictive license.
