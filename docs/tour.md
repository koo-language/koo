# A tour of Koo

This tour covers the part of Koo that the compiler builds today. Every program on this page is run by the test suite, and the output shown under each one is checked, so what you read here works.

The whole language, including the parts that are not built yet, is described in the [specification](../SPEC.md).

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

An error names the place and says what to do about it. A name that is nearly right gets a suggestion, and so does a habit from another language:

```text
error: `[string]` has no method `frist`
  --> main.koo:4:17
   |
 4 |     print(names.frist())
   |                 ^^^^^
   |
   = help: did you mean `first`?

error: unknown name `nil`
  --> main.koo:7:10
   |
 7 |     x := nil
   |          ^^^
   |
   = help: Koo has no null: a value that may be absent has the type `T?`, and its absence is `none`
```

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

The compiler does not build these parts of the specification yet, and says so if you use them: importing one package of your own from another, most of the standard library (HTTP, JSON, SQL, time), tasks and actors, interface values (a list of different shapes), `Decimal`, resources and `defer`, and the WebAssembly targets. The [README](../README.md) keeps the current list.
