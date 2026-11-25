# A tour of Koo

This tour covers the part of Koo that the compiler builds today. Every program on this page is run by the test suite, and the output shown under each one is checked, so what you read here works.

This is the short way in. [The Koo Programming Language](book/README.md) is the long one: a book that covers the same language in full, with a reference manual and a summary of the library. The whole language, including the parts that are not built yet, is described in the [specification](../SPEC.md).

## Running a program

Put this in `hello.koo`:

```koo
fn main() {
    print("Hello from Koo")
}
```

```text
Hello from Koo
```

Then:

```sh
koo run hello.koo                       # build and run
koo build --release hello.koo -o hello  # an optimized executable
koo check hello.koo                     # report errors, build nothing
koo test hello_test.koo                 # run the tests of a file
```

A program is a file with a `main` function. There are no semicolons, and a line break ends a statement.

## Names and values

`:=` gives a name to a value. The name cannot be given a new value unless it is declared with `var`.

```koo
fn main() {
    name := "Ada"
    var visits := 1
    visits += 1

    price: float = 5.0
    ratio := 2.0 / 3.0
    done := false

    print("{name} has visited {visits} times")
    print("price {price}, ratio {ratio:.3}, done {done}")
    print("[{visits:5}] [{visits:<5}] [{visits:05}] [{name:>8}]")
}
```

```text
Ada has visited 2 times
price 5.0, ratio 0.667, done false
[    2] [2    ] [00002] [     Ada]
```

- The basic types are `int` (64 bits), `float`, `bool`, `byte` and `string`.
- Text between `{` and `}` in a string is an expression. After a colon comes a format: a width, an alignment (`<`, `>`, `^`), a leading `0` to pad a number with zeros, and `.n` for the decimals of a float.
- Integer arithmetic is checked. A sum that does not fit stops the program with a message and a line number. It never wraps around silently.

## Functions

```koo
fn area(width: float, height: float) -> float {
    return width * height
}

fn greet(name: string, greeting: string = "Hello") -> string {
    return "{greeting}, {name}!"
}

fn main() {
    print(area(3.0, 4.0))
    print(greet("Ada"))
    print(greet("Ada", greeting: "Welcome back"))
}
```

```text
12.0
Hello, Ada!
Welcome back, Ada!
```

A parameter can have a default, and an argument can be passed by name.

## Structs and methods

```koo
type Point = { x: int, y: int }

fn (self: Point) length_squared() -> int {
    return self.x * self.x + self.y * self.y
}

fn (self: var Point) move_by(dx: int, dy: int) {
    self.x += dx
    self.y += dy
}

fn Point.origin() -> Point {
    return Point { x: 0, y: 0 }
}

fn main() {
    var p := Point { x: 3, y: 4 }
    print(p.length_squared())
    p.move_by(1, 1)
    print(p)
    print(p == Point { x: 4, y: 5 })
    print(Point.origin())
}
```

```text
25
Point { x: 4, y: 5 }
true
Point { x: 0, y: 0 }
```

A method names its receiver `self`. A method that changes the receiver says `var`. Every value can be printed and compared with `==` without writing any code for it.

## Values are values

This is the idea the rest of the language is built on. Assigning a value or passing it to a function never creates a second name for the same storage. A copy is a copy.

```koo
fn add_bonus(scores: var [int], bonus: int) {
    for i in 0..scores.len {
        scores[i] += bonus
    }
}

fn total(scores: [int]) -> int {
    var sum := 0
    for score in scores { sum += score }
    return sum
}

fn main() {
    var scores := [70, 85, 90]
    before := scores

    add_bonus(var scores, 5)

    print(before)
    print(scores)
    print(total(scores))
}
```

```text
[70, 85, 90]
[75, 90, 95]
260
```

- `before` keeps the old scores. Changing `scores` later cannot reach it.
- A function can change an argument only if the parameter says `var`, and the call says `var` too, so you can see at the call that `scores` may change.
- Copies are cheap. The storage is shared until one side changes it, and only then copied. Passing a list to a function copies nothing.

There is no garbage collector and there are no lifetimes to write. Memory is freed when the last value using it goes away.

## Enums and `match`

```koo
enum Shape {
    Circle { radius: float }
    Rect { width: float, height: float }
    Dot
}

fn area(shape: Shape) -> float {
    return match shape {
        Circle { radius } => 3.14159 * radius * radius
        Rect { width, height } => width * height
        Dot => 0.0
    }
}

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
    shapes := [Shape.Circle { radius: 1.0 }, Shape.Rect { width: 2.0, height: 3.0 }, Shape.Dot]
    for shape in shapes {
        print("{area(shape):.2}")
    }
    print(describe(7))
    print(describe(100))
    print(describe(-3))
}
```

```text
3.14
6.00
0.00
small
round
negative
```

A `match` must cover every case. If you add a variant to `Shape`, the compiler points at each `match` that needs a new arm.

## Optionals

There is no null. A value that may be missing has the type `T?`, and the compiler makes you say what happens when it is.

```koo
type User = { name: string, email: string? }

fn find(users: [User], name: string) -> User? {
    for user in users {
        if user.name == name { return user }
    }
    return none
}

fn main() {
    users := [
        User { name: "ada", email: "ada@example.com" },
        User { name: "bob", email: none },
    ]

    if user := find(users, "ada") {
        print("found {user.name}")
    }

    bob := find(users, "bob") else User { name: "nobody", email: none }
    print(bob.email else "no email")

    print(find(users, "cy").is_none())
    print(find(users, "ada").map(u -> u.name.to_upper()))
}
```

```text
found ada
no email
true
ADA
```

- `else` gives the value to use when there is none.
- `if x := e` runs its block only when there is a value, and names it.

## Errors

A function that can fail says so in its type, after `!`. The caller must deal with the error or pass it on with `?`. An error cannot be ignored by accident.

```koo
error Empty
error TooLong { limit: int, found: int }

fn check(name: string) -> string ! Empty | TooLong {
    if name.len == 0 { fail Empty }
    if name.len > 8 { fail TooLong { limit: 8, found: name.len } }
    return name.to_lower()
}

fn register(names: [string]) -> int ! Empty | TooLong {
    var count := 0
    for name in names {
        check(name)?
        count += 1
    }
    return count
}

fn main() {
    for name in ["Ada", "", "Bartholomew"] {
        match check(name) {
            ok => print("ok: {ok}")
            ! Empty => print("a name is required")
            ! TooLong { limit, found } => print("{found} letters is over the limit of {limit}")
        }
    }

    print(check("") else "anonymous")
    print(register(["ada", "bob"]) else 0)
    print(register(["ada", ""]) else -1)
}
```

```text
ok: ada
a name is required
11 letters is over the limit of 8
anonymous
2
-1
```

- `fail` returns an error. `?` passes an error on to the caller.
- `else` turns any error into a value. `match` handles each error separately, and it must cover all of them.
- A function lists the errors it can return, and the compiler checks the list.
- An error caught with `! e` or `else e =>` can be passed on after some cleaning up: `fail e`.

## Lists and lambdas

```koo
type Order = { customer: string, total: float, paid: bool }

fn main() {
    orders := [
        Order { customer: "ada", total: 40.0, paid: true },
        Order { customer: "bob", total: 15.5, paid: false },
        Order { customer: "cy", total: 99.0, paid: true },
    ]

    paid := orders.filter(o -> o.paid)
    names := paid.map(o -> o.customer.to_upper())
    revenue := paid.fold(0.0, (sum, o) -> sum + o.total)

    print(names.join(", "))
    print("{revenue:.2}")
    print(orders.any(o -> o.total > 50.0))
    print(orders.sorted_by((a, b) -> a.total < b.total).map(o -> o.customer))

    var numbers := [5, 3, 8, 1]
    numbers.sort()
    numbers.push(13)
    print(numbers)
    print("{numbers.len} numbers, sum {numbers.sum()}, largest {numbers.max()}")
    print(numbers.contains(8))
}
```

```text
ADA, CY
139.00
true
["bob", "ada", "cy"]
[1, 3, 5, 8, 13]
5 numbers, sum 30, largest 13
true
```

These methods chain, and a chain costs what one loop would:

```koo
type Order = { customer: string, total: float, paid: bool }

fn main() {
    orders := [
        Order { customer: "ada", total: 40.0, paid: true },
        Order { customer: "bob", total: 15.5, paid: false },
        Order { customer: "cy", total: 99.0, paid: true },
    ]
    taxed := orders.filter(o -> o.paid).map(o -> o.total * 1.2).filter(t -> t > 50.0).sum()
    print("{taxed:.2}")
}
```

```text
118.80
```

Each method means what it says, one list after another. But when the functions in a chain only compute (they print nothing, and change no variable outside themselves), no program could tell whether the lists in between were ever built, so the compiler does not build them: the chain above is one pass over `orders` and allocates nothing. A chain whose functions do print, or count as they go, runs step by step exactly as written.

A lambda is `x -> expression`, `(a, b) -> expression`, or the same with a block in braces. A lambda can use the variables around it:

```koo
fn scale_by(factor: int) -> fn(int) -> int {
    return n -> n * factor
}

fn main() {
    triple := scale_by(3)
    print(triple(14))

    limit := 3
    print([1, 2, 3, 4, 5].filter(n -> n > limit))

    var total := 0
    [1, 2, 3].each(n -> { total += n })
    print(total)
}
```

```text
42
[4, 5]
6
```

A lambda passed straight to a function, like the one given to `each`, works on the variables around it directly, so it can change `total`. A lambda that is kept, like the one `scale_by` returns, holds its own copy of what it uses.

## Strings

```koo
fn main() {
    line := "  Koo, a language for services  "
    text := line.trim()

    print(text.len)
    print(text.to_upper())
    print(text.contains("language"))
    print(text.replace("services", "backends"))
    print(text.split(" "))
    print(text.starts_with("Koo"))

    if (key, value) := "port=8080".cut("=") {
        print("{key} is {int.parse(value) else 0}")
    }

    report := """
        Name:  {text.slice(0, 3)}
        Words: {text.split(" ").len}
        """
    print(report)
}
```

```text
28
KOO, A LANGUAGE FOR SERVICES
true
Koo, a language for backends
["Koo,", "a", "language", "for", "services"]
true
port is 8080
Name:  Koo
Words: 5
```

There is one string type. It is always valid UTF-8, and `len` counts bytes. A text block between `"""` lines drops the indentation it was written with.

## Maps and sets

```koo
fn main() {
    var stock := ["pen": 120, "ink": 8]
    stock["paper"] = 500
    stock["ink"] = 7

    print(stock["pen"])
    print(stock["glue"])
    print(stock["glue"] else 0)
    print(stock.len)

    for (item, count) in stock {
        print("{item:<6}{count:4}")
    }

    var words: [string: int] = [:]
    for word in "the cat saw the other cat".split(" ") {
        words.update(word, 0, n -> n + 1)
    }
    print(words)

    var seen := Set<string>.new()
    for word in ["a", "b", "a"] {
        if not seen.insert(word) { print("{word} again") }
    }
}
```

```text
120
none
0
3
pen    120
ink      7
paper  500
["the": 2, "cat": 2, "saw": 1, "other": 1]
a again
```

- `m[key]` is an optional, because the key may be absent.
- A map remembers the order in which its keys were added, so a loop over it gives the same order every time.
- A key can be any value that can be hashed: numbers, strings, structs, enums and tuples of them.

## Tuples

```koo
fn min_max(numbers: [int]) -> (int, int) {
    var lo := numbers[0]
    var hi := numbers[0]
    for n in numbers {
        if n < lo { lo = n }
        if n > hi { hi = n }
    }
    return (lo, hi)
}

fn main() {
    (lo, hi) := min_max([4, -2, 9, 0])
    print("from {lo} to {hi}")

    for (i, name) in ["ada", "bob"].enumerate() {
        print("{i}: {name}")
    }
}
```

```text
from -2 to 9
0: ada
1: bob
```

## Generics and interfaces

A type or a function can take type parameters. A constraint after the colon says what the type must be able to do.

```koo
type Stack<T> = { items: [T] }

fn Stack<T>.new() -> Stack<T> {
    return Stack { items: [] }
}

fn (self: var Stack<T>) push(item: T) {
    self.items.push(item)
}

fn (self: var Stack<T>) pop() -> T? {
    return self.items.pop()
}

fn largest<T: Ordered>(items: [T]) -> T? {
    if items.len == 0 { return none }
    var best := items[0]
    for item in items {
        if item > best { best = item }
    }
    return best
}

interface Shape {
    fn area() -> float
    fn name() -> string
}

type Square = { side: float }
fn (self: Square) area() -> float { return self.side * self.side }
fn (self: Square) name() -> string { return "square" }

type Circle = { radius: float }
fn (self: Circle) area() -> float { return 3.14159 * self.radius * self.radius }
fn (self: Circle) name() -> string { return "circle" }

fn describe(shape: Shape) -> string {
    return "a {shape.name()} of area {shape.area():.1}"
}

fn main() {
    var stack := Stack<string>.new()
    stack.push("a")
    stack.push("b")
    print(stack.pop())

    print(largest([3, 9, 4]))
    print(largest(["pear", "apple"]))

    print(describe(Square { side: 2.0 }))
    print(describe(Circle { radius: 1.0 }))
}
```

```text
b
9
pear
a square of area 4.0
a circle of area 3.1
```

A type satisfies an interface by having its methods. Nothing has to be declared. Generic code is compiled once for each type it is used with, so it runs as fast as code written by hand for that type.

A type parameter can also stand for the errors of a function that is passed in, so that one helper serves callers that fail in different ways, or not at all:

```koo
error Refused { attempt: int }
error GaveUp { after: int }

fn retry<T, E>(times: int, body: fn(int) -> T ! E) -> T ! E | GaveUp {
    for attempt in 1..=times {
        match body(attempt) {
            value => return value
            ! e => print("attempt {attempt}: {e}")
        }
    }
    fail GaveUp { after: times }
}

fn connect(attempt: int) -> string ! Refused {
    if attempt < 3 { fail Refused { attempt } }
    return "connected"
}

fn main() {
    match retry(5, connect) {
        said => print(said)
        ! GaveUp { after } => print("gave up after {after}")
        ! e: Refused => print("refused")
    }
    print(retry(3, attempt -> attempt * 10) else 0)
}
```

```text
attempt 1: Refused { attempt: 1 }
attempt 2: Refused { attempt: 2 }
connected
10
```

`retry(5, connect)` can fail with `Refused` or `GaveUp`, and the `match` must cover both. The lambda in the last line cannot fail, so that call can fail only with `GaveUp`.

## Files, arguments and input

```koo
import std/io
import std/os

fn main() ! IoError {
    dir := os.env("TMPDIR") else "/tmp"
    path := "{dir}/koo_tour.txt"

    io.write_file(path, "first\nsecond\n")?
    text := io.read_file(path)?
    print(text.lines())

    print(os.args().len)

    match io.read_file("/no/such/file") {
        _ => print("unexpected")
        ! e => print(e.message)
    }
}
```

```text
["first", "second"]
0
cannot read /no/such/file: No such file or directory
```

- `main` may return an error. The program then prints it and exits with status 1.
- `io.read_line()` reads a line of standard input and gives `none` at the end. `io.read_lines()` reads all of them.
- `koo run file.koo -- one two` passes `one` and `two` as arguments.

## JSON

A struct or an enum is JSON as it stands. There is nothing to import, annotate or derive: `value.to_json()` writes it and `Type.from_json(text)` reads it.

```koo
enum Status { Open, Shipped }

type Item = { sku: string, qty: int = 1, note: string? }
type Order = { id: int, items: [Item], status: Status }

fn main() {
    order := Order { id: 7, items: [Item { sku: "a-1", qty: 2, note: none }], status: Status.Open }
    print(order.to_json())

    text := "\{\"id\": 8, \"items\": [\{\"sku\": \"b-2\"}], \"status\": \"shipped\"}"
    match Order.from_json(text) {
        back => print("order {back.id}: {back.items[0].qty} of {back.items[0].sku}")
        ! e => print("{e.path}: {e.message}")
    }

    bad := "\{\"id\": 9, \"items\": [\{\"sku\": \"c-3\", \"qty\": \"two\"}], \"status\": \"open\"}"
    match Order.from_json(bad) {
        back => print("order {back.id}")
        ! e => print("{e.path}: {e.message}")
    }
}
```

```text
{"id":7,"items":[{"sku":"a-1","qty":2}],"status":"open"}
order 8: 1 of b-2
/items/0/qty: expected a whole number, found a string
```

- A struct is an object, a list is an array, and a map with string keys is an object.
- An optional field that is `none` is left out, and a missing key or `null` reads as `none`. A field with a default takes it when its key is missing. Any other missing key is an error.
- An enum without fields is its name in snake_case, as a string. An enum with fields is an object with the name under `"type"`.
- Keys the type does not have are passed over.
- A text that does not fit is refused with a `JsonError`, whose `path` leads to the mistake.
- Numbers, strings, booleans, lists and maps have `to_json` too: `[1, 2, 3].to_json()`.

## Time

A length of time is a `Duration`, written with its unit, and an instant is a `Time`. Neither is a number, so a timeout of `30` cannot be mistaken for seconds or for milliseconds.

```koo
import std/time

type Booking = { room: string, starts: Time, lasts: Duration }

fn main() {
    timeout := 30s
    print(timeout / 4)
    print(1h30min + 45min)

    booking := Booking { room: "Lovelace", starts: Time.parse("2026-03-01T09:30:00Z") else Time.from_unix(0), lasts: 90min }
    print(booking.starts + booking.lasts)
    print(booking.to_json())

    started := time.uptime()
    time.sleep(10ms)
    print(time.uptime() - started >= 10ms)
}
```

```text
7s500ms
2h15min
2026-03-01T11:00:00Z
{"room":"Lovelace","starts":"2026-03-01T09:30:00Z","lasts":"1h30min"}
true
```

- Durations add, subtract, and multiply and divide by an `int`. A time and a duration add to a time, and two times subtract to a duration.
- `time.now()` is the clock, `time.uptime()` a clock that only goes forward, for measuring, and `time.sleep(length)` waits.
- Both are JSON, as strings: `"1h30min"` and RFC 3339.

## Digests, encodings and chance

Four small packages hold what a service needs to sign a message, keep a password and make a token.

```koo
import std/crypto
import std/encoding
import std/hash
import std/random

fn main() {
    body := `{"event": "paid", "order": 17}`
    signature := crypto.hmac_sha256("a shared secret".bytes(), body.bytes())
    print(encoding.hex(signature))
    print(crypto.equal(signature, crypto.hmac_sha256("a shared secret".bytes(), body.bytes())))

    print(encoding.hex(hash.sha256("abc".bytes())))
    print(encoding.base64("hello".bytes()))
    print(encoding.url("a b&c"))

    kept := crypto.hash_password("correct horse")
    print(crypto.verify_password("correct horse", kept))
    print(crypto.verify_password("wrong horse", kept))

    roll := random.int(1, 7)
    print(roll >= 1 and roll <= 6)
    print(random.uuid().len)
    print(encoding.base64_url(random.bytes(24)).len)
}
```

```text
25baade0d563f806ccb0b795eaf04fa786eb7ff3aef5dda0e579c67d76495b9a
true
ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad
aGVsbG8=
a%20b%26c
true
false
true
36
32
```

- They work on lists of bytes, `[byte]`; `text.bytes()` is the bytes of a string.
- `crypto.equal` compares two digests without telling, by how long it takes, where they differ. Use it, not `==`, for anything secret.
- `hash_password` returns the text to store in place of a password: PBKDF2 with a fresh salt and 600,000 rounds.
- Every random number comes from the operating system's source for keys, so `random.bytes` is fit for tokens and there is no separate "secure" generator to remember. A list has `pick()` and `shuffle()`.

## Logging

`std/log` has `log.debug`, `log.info`, `log.warn` and `log.error`. A line is written like a string, `log.info("order {order.id} paid")`, and like a query it is not one: with `KOO_LOG=json` in the environment each line is a JSON object in which `order.id` is a field of its own, as a number, beside the message. `KOO_LOG_LEVEL` says which levels are written, and a line of a level that is not costs nothing, since its values are not even computed. [Section 8.10 of the book](book/08-io.md) shows both forms.

## A web server

`std/http` is a server in a few lines. A handler is a function from a request to a response.

```koo
import std/http

type Item = { sku: string, qty: int = 1 }
type NewOrder = { customer: string, items: [Item] }
type Order = { id: int, customer: string, items: [Item] }

fn main() ! http.Error {
    var app := http.App.new()
    app.get("/", req -> http.text("Hello, world!"))
    app.get("/hello/:name", req -> http.text("Hello, {req.param("name")}!"))
    app.post("/orders", req -> {
        input: NewOrder = req.json()?
        return http.json(Order { id: 1, customer: input.customer, items: input.items }, status: 201)
    })
    app.serve(port: 8080)?
}
```

```sh
$ curl localhost:8080/hello/Ada
Hello, Ada!
$ curl localhost:8080/orders -d '{"customer": "Bob", "items": [{"sku": "a-1"}]}'
{"id":1,"customer":"Bob","items":[{"sku":"a-1","qty":1}]}
$ curl localhost:8080/orders -d '{"customer": "Bob", "items": [{"sku": "a-1", "qty": "two"}]}'
{"error":"expected a whole number, found a string","path":"/items/0/qty"}
```

- `app.get`, `post`, `put`, `patch`, `delete` and `any` each take a path and a handler. `:name` in a path matches one segment, read with `req.param("name")`, and `*` at the end matches the rest. The first route that matches answers.
- A request has `req.method`, `req.path`, `req.param(name)`, `req.query(name)` and `req.header(name)`, which are optional where the client may have left them out, `req.body()`, and `req.json()`, which reads the body into the type that is expected.
- A response is made by `http.text`, `http.html`, `http.json`, `http.status(404, "no such order")`, `http.redirect(to)` or `http.send(body, content_type)`. Each takes `status:`, and `.header(name, value)` adds a header.
- A handler may be a lambda or a named function, and it may fail. A body that does not fit its type is answered with 400 and the path to the mistake. Any other error is answered with 500 and written to standard error, not sent to the client.
- `app.files("/static", "public")` serves the files of a directory, with the type each name implies and `index.html` for the directory itself. A path cannot climb out of it. `http.file(path)` answers with one file.
- `req.form(name)` reads a field of a form a page sent, and `req.cookie(name)` a cookie; `response.cookie(name, value, max_age: 3600)` sets one.
- A path with no route is answered with 404, and one with a route for another method with 405.
- `serve` runs until the program is told to stop, with Ctrl-C or a request to terminate, and then returns. It takes `port:`, `workers:`, `max_body:` (the most a request's body may be, 1 MB unless you say) and `log: true`, which writes a line for each request.

The server answers requests on every core: `serve` starts a process for each, all listening on the one port, and stays behind to replace one that dies. A handler cannot tell, because nothing in memory is shared between requests in the first place. So a handler that panics costs the requests its worker was holding, and the server goes on. `workers: 1` keeps to one process, which is the plainest thing to debug, and which a panic ends. On one core it serves a "Hello, world!" as fast as a Rust server built on axum and tokio, and more than twice as fast as Node; the numbers and how they were taken are in the [README](../README.md).

It runs on macOS and Linux alike. On Linux, `koo build --release --static` makes a program with nothing left to install, and [`examples/http/Dockerfile`](../examples/http/Dockerfile) puts one in an empty container.

What it does not do yet: HTTPS and HTTP/2 (put it behind a proxy or a platform that speaks them), streaming a body in either direction, and waiting on something slow without holding up the other requests of the same worker. Working servers are in [`examples/http`](../examples/http).

## A database

`std/sql` talks to PostgreSQL. A query is written like a string with values in braces, but it is not a string: the values are sent to the database beside the statement, as parameters, and never become part of the SQL. There is nothing to escape, and nothing to inject.

```koo
import std/os
import std/sql
import std/sql/postgres

type Account = { name: string, balance: int }

error Insufficient { balance: int, wanted: int }

fn transfer(db: sql.Pool, from: string, to: string, amount: int) ! Insufficient | sql.Error {
    db.transaction(tx -> {
        row := tx.query_row("select * from accounts where name = {from} for update")?
        account := Account.from_row(row)?
        if account.balance < amount {
            fail Insufficient { balance: account.balance, wanted: amount }
        }
        tx.exec("update accounts set balance = balance - {amount} where name = {from}")?
        tx.exec("update accounts set balance = balance + {amount} where name = {to}")?
    })?
}

fn main() ! sql.Error {
    db := postgres.connect(os.env("DATABASE_URL") else "postgres://localhost/bank")?
    db.exec("create table accounts (name text primary key, balance int not null)")?
    db.exec("insert into accounts values ({"ada"}, {100}), ({"bob"}, {20})")?

    for amount in [30, 500] {
        match transfer(db, "ada", "bob", amount) {
            () => print("moved {amount}")
            ! Insufficient { balance, wanted } => print("only {balance} there, not {wanted}")
            ! e: sql.Error => print("the database said: {e}")
        }
    }
    for row in db.query("select * from accounts order by name")? {
        print(Account.from_row(row)?)
    }
}
```

```text
moved 30
only 70 there, not 500
Account { name: "ada", balance: 70 }
Account { name: "bob", balance: 50 }
```

- `postgres.connect` returns a pool of connections. It is safe to hand to every handler of a server and every actor; each statement takes a connection and gives it back.
- `exec` returns the number of rows changed, `query` a list of rows, `query_one` the first row or `none`, and `query_row` a row that must be there.
- `Account.from_row(row)` reads a struct from a row, each field from the column of its name. A row also gives one value at a time: `row.int("balance")`.
- `db.transaction` commits when its function returns and rolls back when it fails, and fails with the function's own error: here `Insufficient` or a `sql.Error`.
- Every failure of the database is a `sql.Error`, whose `kind` says which sort: `Connection`, `Constraint`, `Syntax`, `Serialization`, `Timeout`, `NoRows`.
- The connection is encrypted when the server offers it, and `?sslmode=verify-full` in the address insists on it and on the server's certificate. The TLS is the system's OpenSSL, loaded when it is first needed.
- Passing a `string` where a query is wanted is a compile error. `sql.Query.raw(text)` is there for the rare statement that must be built as text.

The program reads the address of its database from `DATABASE_URL`, as `postgres://user:password@host:5432/name`. [Chapter 11 of the book](book/11-database.md) has the rest.

## Actors

An actor is state with an owner. It answers one message at a time, so its methods need no locks and cannot race, and everything that holds its handle talks to the same one.

```koo
error Empty

actor Account {
    var balance: int = 0
    owner: string

    fn deposit(amount: int) -> int {
        balance += amount
        return balance
    }

    fn withdraw(amount: int) -> int ! Empty {
        if amount > balance { fail Empty }
        balance -= amount
        return balance
    }

    fn close() {
        print("{owner} closes with {balance}")
        balance = 0
    }
}

fn main() {
    account := Account.start(owner: "Ada")
    print("{account.deposit(100)}")
    left := account.withdraw(30) else -1
    print("{left}")
    broke := account.withdraw(500) else -1
    print("{broke}")
    account.send.close()
    print("{account.deposit(1)}")
}
```

```text
100
70
-1
Ada closes with 70
1
```

- `Account.start(...)` takes every field that has no default, best given by name, and returns a handle. A handle is a value like any other: it can be copied, kept in a struct, and sent to another actor.
- `account.deposit(100)` asks: it sends the message and waits for the answer, errors included.
- `account.send.close()` tells: it sends and goes on. Only a method that returns nothing and cannot fail can be told, so nothing is thrown away. What an actor is told, it does in the order told, and before it next answers an ask: that is why `Ada closes with 70` comes before the `1`.
- Inside the actor a field is used by its bare name, and only a `var` field can be changed. A method calls another of the same actor by bare name. `self` is the actor's own handle.
- A message is not copied. It is frozen: the sender and the actor read the same value, and whichever changes it gets a copy of its own first, as with any value that two variables hold. What goes to an actor and comes back is numbers, strings, booleans, lists, optionals, maps with string keys, your structs, enums and errors made of those, and handles.
- If a method panics, the one who asked panics too, and the actor starts again with the state it started with. More than five such restarts in a minute and it stays stopped.
- An ask to an actor that is waiting for the asker's own answer would wait for ever, so it panics and says so, whether the two are on one thread or two. Use `send` on one side.
- An actor stops when `handle.stop()` is called, or at the end of the program. The program ends when `main` has returned and everything that was told has been done.

An actor is how a server keeps anything between requests. In [`examples/http/counter.koo`](../examples/http/counter.koo) every worker of the server asks one `Counter`, and the count is exact.

### Side by side

What actors are told runs on other threads, one for each processor. Eight actors that are each told to count primes count at the same time:

```koo
actor Counter {
    var found: int = 0

    fn count(from: int, to: int) {
        for n in from..to {
            if is_prime(n) { found += 1 }
        }
    }

    fn result() -> int {
        return found
    }
}

fn is_prime(n: int) -> bool {
    if n < 2 { return false }
    var d := 2
    while d * d <= n {
        if n % d == 0 { return false }
        d += 1
    }
    return true
}

fn main() {
    var counters: [Counter] = []
    for i in 0..8 {
        counters.push(Counter.start())
    }
    for i in 0..8 {
        counters[i].send.count(i * 100000, (i + 1) * 100000)
    }
    var total := 0
    for counter in counters {
        total += counter.result()
    }
    print("{total} primes below 800000")
}
```

```text
63951 primes below 800000
```

- Nothing here is locked or shared by hand. Each `count` has its own actor to itself, and `result()` waits until its actor has done what it was told.
- An ask runs on the asker's own thread, with the actor held for as long as the method runs. Another thread that wants the same actor waits its turn.
- It is cheap enough to use freely: an ask costs about 8 nanoseconds, a message told to another actor about 45, and a million actors take about 170 MB. [The README](../README.md#actors) has the same programs in Erlang.

The workers of a server are processes, not threads, and reach an actor through the process that has it, with the arguments sent as text.

An actor's mail has a limit, 1,024 messages unless the actor says otherwise with `@mailbox(n)` before its declaration. `send` to an actor whose mail is full waits for room, so a sender cannot get ahead of an actor without end, and `handle.try_send.method(...)` does not wait: it returns whether there was room.

A method that runs for long keeps its thread, but not the other actors waiting: when every thread is busy, another is started for them.

What is not there yet: nothing interrupts a method that runs for ever, so its own actor answers nobody else; a panic in a message that was told leaks what the message was holding; and generic actors, `spawn` inside an actor, and functions or tuples in messages are not built.

## Tests

A test is a function whose name starts with `test_`. `koo test file.koo` runs each one in a process of its own and reports the ones that fail.

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

fn main() {
    print(slug("A Tour of Koo"))
}
```

```text
a-tour-of-koo
```

When an assertion fails, the report shows both values and the line:

```text
FAILED test_slug_trims
       panic: assertion failed at slug_test.koo:12
          left: "koo "
         right: "koo"
```

## Programs in more than one file

A directory is a package. Every `.koo` file in it is part of one program, and they see each other's declarations with no imports between them.

```text
shop/
  main.koo        fn main() ...
  cart.koo        type Cart, fn (self: var Cart) add(...)
  receipt.koo     fn receipt(cart: Cart) -> string
  cart_test.koo   fn test_checkout_gives_change() ...
```

```sh
koo run shop      # build the package and run its main
koo test shop     # run the tests in its *_test.koo files
koo build shop    # an executable named shop
```

An `import` belongs to the file it is written in, so each file imports the standard packages it uses. A working package like this one is in [`tests/packages/shop`](../tests/packages/shop).

## The compiler tries to help

An error says what is wrong, shows where, and points at the other place that explains it: the declaration that set the type, the first of two declarations, the bracket that was never closed. Where there is one thing you must have meant, it shows the corrected line.

```text
error: `add_all` changes this argument, so it is passed with `var`
  --> main.koo:13:13
   |
 5 | fn add_all(names: var [string], more: [string]) {
   |            ------------------- the parameter `names` is `var`
...
13 |     add_all(names, ["cy"])
   |             ^^^^^
   |
   = note: `var` at the call shows the reader that the argument may change (Section 5.3)
help: write `var` before the argument
   |
13 |     add_all(var names, ["cy"])
   |             ++++

error: `[string]` has no method `frist`
  --> main.koo:14:17
   |
14 |     print(names.frist() else "nobody")
   |                 ^^^^^
   |
help: did you mean `first`?
   |
14 -     print(names.frist() else "nobody")
14 +     print(names.first() else "nobody")

error: expected `float`, found `string`
  --> main.koo:15:32
   |
 1 | fn total(prices: [float], tax: float) -> float {
   |                           ---------- the parameter `tax` is declared here
...
15 |     print("{total([9.5, 20.0], "8%")}")
   |                                ^^^^ this is a `string`
```

`^` marks the mistake and `-` a place that explains it. A line with `+` under it shows what to add; a pair of lines marked `-` and `+` shows a line as it is and as it should be.

The compiler also knows the habits of other languages, and answers each with the Koo that was meant:

```text
error: `let` is not Koo: a variable is declared with `:=`
 --> main.koo:2:5
  |
2 |     let count = 3
  |     ^^^
  |
  = note: `x := 5` declares a variable that does not change, and `var x := 5` one that does
help: declare `count` the Koo way
  |
2 -     let count = 3
2 +     count := 3

error: this is the `for` of another language
 --> main.koo:3:9
  |
3 |     for (i = 0; i < count; i++) {
  |         ^^^^^^^^^^^^^^^^^^^^^^^
  |
  = note: a `for` loop goes over a range or a list: `for i in 0..n { ... }`, `for item in items { ... }`
help: count with a range
  |
3 -     for (i = 0; i < count; i++) {
3 +     for i in 0..count {
```

The same goes for `function` and `def`, `class` and `struct`, `elif`, `&&` and `||`, `!x`, `x++`, `'single quotes'`, `null`, `try` and `throw`, `new`, `# comments`, `: int` for a return type, `int x = 5`, and `.length`, `.append` and their relatives. In a terminal the messages are in color; set `NO_COLOR` to turn that off.

## Blocks and exclusivity

A lambda passed straight to a function works on the variables around it. To keep that safe, the compiler holds one rule: while a call can change a variable, nothing else the call is given may use that variable.

```text
error: this block uses `scores`, which the same call changes
  --> main.koo:3:20
   |
 3 |     scores.sort_by((a, b) -> scores.len > a)
   |                    ^^^^^^^^^^^^^^^^^^^^^^^^
```

Here `sort_by` is changing `scores`, so the block may not look at it. Take what the block needs out of the variable before the call, or work on a copy. The rule is checked when the program is compiled; nothing is checked while it runs.

## When things go wrong

Koo stops a program instead of letting it carry on with bad data. Each of these ends the program with a message and the line it happened on:

- an index outside a list
- integer overflow, or division by zero
- an explicit `panic("...")`

None of them can corrupt memory. Errors you expect, such as a missing file or bad input, are not panics. They are values, as in the section on errors.

## What is not here yet

The compiler does not build these parts of the specification yet, and says so if you use them: importing one package of your own from another, parts of the standard library (an HTTP client, HTTPS in the server), tasks, channels and `select`, interface values (a list of different shapes), `Decimal`, resources and `defer`, and the WebAssembly targets. The [README](../README.md) keeps the current list.
