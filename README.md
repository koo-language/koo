# Koo

Koo is a compiled programming language for backend services. It aims for the performance of Rust with the ease of a scripting language: no garbage collector, no borrow checker, no `async`/`await`. Programs compile to native binaries for x86-64 and arm64, and to WebAssembly.

> **Status: design stage.** The language is specified in [`SPEC.md`](SPEC.md) (version 0.5, draft). There is no compiler yet, and the performance figures in the specification are targets, not measurements.

## A complete service

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

## Roadmap

| Milestone | Scope |
| --- | --- |
| 0.1 | Parser, type checker, ownership pass, debug backend, `koo run`, `koo test` |
| 0.2 | LLVM release builds, C FFI, first published benchmarks against Rust |
| 0.3 | Tasks and scheduler, WebAssembly targets, `std/http`, `std/json` |
| 0.4 | Actors and supervision, `std/sql/postgres`; the example above runs natively and as a Wasm component |
| 0.5 to 0.9 | Formatter, language server, package manager, gRPC, TLS, Windows, public beta |
| 1.0 | Stability commitment begins |

Section 13.7 of the specification has the full list.

## Contributing

The most useful contribution today is review of the specification: open an issue for anything unclear, inconsistent or wrong. Changes to the language's syntax or semantics go through the proposal process described in [`GOVERNANCE.md`](GOVERNANCE.md). Commits are signed off under the [Developer Certificate of Origin](https://developercertificate.org); there is no contributor license agreement.

## License

Koo is dual-licensed under the [Apache License 2.0](LICENSE-APACHE) and the [MIT License](LICENSE-MIT), at your option. See [`LICENSE`](LICENSE).

The project commits to never relicensing under a source-available, non-commercial or otherwise restrictive license.
