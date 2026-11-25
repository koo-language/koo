# Appendix B: The Standard Library

This appendix is a summary of the library: the functions and methods that are available in every program, and those of the standard packages. Each is given with its types, in the notation of a function's first line, where `T` and `U` stand for any type. After each table is a program that calls everything in it.

A method that is marked `var` changes the value it is called on, and can be called only on a variable.

## B.1 Everywhere

| | |
| --- | --- |
| `print(value: T)` | writes the text form of the value, and a newline, on the standard output |
| `panic(message: string)` | writes the message on the standard error output and ends the program with status 101 |
| `value.to_string() -> string` | the text form of a value of any type |

## B.2 Numbers

| `int` | |
| --- | --- |
| `int.parse(s: string) -> int ! ParseError` | the integer that the text is, in decimal, with an optional sign |
| `int.from(b: byte) -> int` | the value of a byte |
| `n.abs() -> int` | the absolute value |
| `n.pow(exponent: int) -> int` | `n` raised to a power that is not negative |
| `n.min(other: int) -> int`, `n.max(other: int) -> int` | the smaller, the larger |
| `n.clamp(low: int, high: int) -> int` | `n`, or the bound it is beyond |
| `n.compare(other: int) -> int` | negative, zero or positive, as `n` is less than, equal to or greater than the other |
| `n.wrapping_add(m: int) -> int`, `wrapping_sub`, `wrapping_mul` | arithmetic that wraps around where the operators would panic |
| `n.bit_and(m: int) -> int`, `bit_or`, `bit_xor` | bitwise operations |
| `n.shl(count: int) -> int`, `n.shr(count: int) -> int` | shifts; the right shift copies the sign bit |
| `n.hash() -> int` | a hash value |

| `float` | |
| --- | --- |
| `float.parse(s: string) -> float ! ParseError` | the number that the text is |
| `float.from(n: int) -> float` | the nearest `float` to an integer |
| `x.abs() -> float`, `x.sqrt() -> float` | the absolute value, the square root |
| `x.min(other: float) -> float`, `x.max`, `x.clamp(low, high)` | as for `int` |
| `x.compare(other: float) -> int` | as for `int` |
| `x.round() -> int ! Overflow` | the nearest integer; halves go away from zero |
| `x.floor() -> int ! Overflow`, `x.ceil()`, `x.trunc()` | the integer below, above, toward zero |

| `byte` | |
| --- | --- |
| `byte.from(n: int) -> byte ! Overflow` | the byte with a value from 0 to 255 |

```koo
fn main() {
    print("{int.parse("-42") else 0} {int.from(byte.from(65) else 0)}")
    print("{(-7).abs()} {2.pow(8)} {3.min(5)} {3.max(5)} {12.clamp(0, 10)} {3.compare(5)}")
    big := 9223372036854775807
    print("{big.wrapping_add(1)} {0.wrapping_sub(1)} {big.wrapping_mul(2)}")
    print("{12.bit_and(10)} {12.bit_or(10)} {12.bit_xor(10)} {1.shl(4)} {(-32).shr(2)}")
    print(7.hash() == 7.hash())

    print("{float.parse("2.5") else 0.0} {float.from(3)}")
    print("{(-2.5).abs()} {16.0.sqrt()} {1.5.min(2.5)} {1.5.max(2.5)} {7.5.clamp(0.0, 1.0)} {2.5.compare(1.5)}")
    print("{2.5.round() else 0} {(-2.5).round() else 0} {2.7.floor() else 0} {2.2.ceil() else 0} {(-2.7).trunc() else 0}")
    print(1e30.round() else -1)
}
```

```text
-42 65
7 256 3 5 10 -1
-9223372036854775808 -1 -2
8 14 6 16 -8
true
2.5 3.0
2.5 4.0 1.5 2.5 1.0 1
3 -3 2 3 -2
-1
```

## B.3 Optionals

| `T?` | |
| --- | --- |
| `o.is_some() -> bool`, `o.is_none() -> bool` | whether there is a value |
| `o.map(f: fn(T) -> U) -> U?` | `f` of the value, if there is one |

The operator `else`, the binding condition and `match` are the other ways to use an optional (Section 5.11).

## B.4 Lists

| `[T]` | |
| --- | --- |
| `List<T>.with_size(n: int) -> [T]` | `n` zeros, or `false`; `T` is `int`, `float`, `bool` or `byte` |
| `List<T>.with_capacity(n: int) -> [T]` | an empty list with room for `n` elements |
| `v.len` | the number of elements |
| `v.is_empty() -> bool` | whether there are none |
| `v.get(i: int) -> T?` | element `i`, or `none` |
| `v.first() -> T?`, `v.last() -> T?` | the first and the last element |
| `v.contains(x: T) -> bool` | whether an element is equal to `x` |
| `v.index_of(x: T) -> int?` | the position of the first element equal to `x` |
| `v.slice(from: int, to: int) -> [T]` | the elements from `from` up to but not including `to` |
| `v.take(n: int) -> [T]`, `v.skip(n: int) -> [T]` | the first `n` elements, and the rest |
| `v.reversed() -> [T]` | the elements in the opposite order |
| `v.sorted() -> [T]` | the elements in increasing order |
| `v.sorted_by(less: fn(T, T) -> bool) -> [T]` | the elements in the order that `less` gives; stable |
| `v.enumerate() -> [(int, T)]` | each element with its position |
| `v.zip(other: [U]) -> [(T, U)]` | pairs of corresponding elements |
| `v.join(separator: string) -> string` | the text forms of the elements, with the separator between |
| `v.sum() -> T` | the sum of the elements, zero if there are none |
| `v.min() -> T?`, `v.max() -> T?` | the smallest and the largest element |
| `v.map(f: fn(T) -> U) -> [U]` | `f` of each element |
| `v.filter(keep: fn(T) -> bool) -> [T]` | the elements for which `keep` is true |
| `v.fold(start: U, f: fn(U, T) -> U) -> U` | the elements combined, from `start` |
| `v.each(f: fn(T))` | calls `f` with each element |
| `v.any(test: fn(T) -> bool) -> bool`, `v.all(test)` | whether `test` is true of some element, of every element |
| `v.count(test: fn(T) -> bool) -> int` | how many elements `test` is true of |
| `v.find(test: fn(T) -> bool) -> T?` | the first element `test` is true of |
| `v.position(test: fn(T) -> bool) -> int?` | its position |
| `var` `v.push(x: T)` | adds an element at the end |
| `var` `v.pop() -> T?` | removes the last element and returns it |
| `var` `v.insert(i: int, x: T)` | puts `x` at position `i`, moving the rest up |
| `var` `v.remove(i: int) -> T` | removes element `i` and returns it |
| `var` `v.extend(other: [T])` | adds the elements of another list at the end |
| `var` `v.swap(i: int, j: int)` | exchanges two elements |
| `var` `v.reverse()` | reverses the list in place |
| `var` `v.sort()` | sorts the list in place |
| `var` `v.sort_by(less: fn(T, T) -> bool)` | sorts the list in place, by `less`; stable |
| `var` `v.clear()` | removes every element |
| `v.pick() -> T?` | an element chosen by chance, or `none` of an empty list |
| `var` `v.shuffle()` | puts the elements in an order chosen by chance |

`contains` and `index_of` need a type with `==`; `sorted`, `sort`, `min` and `max` a type that is ordered; `sum` a list of `int` or `float`.

```koo
fn main() {
    var v := [5, 3, 8]
    print("{v.len} {v.is_empty()} {v.get(1)} {v.get(9)} {v.first()} {v.last()}")
    print("{v.contains(8)} {v.index_of(8)} {v.slice(1, 3)} {v.take(2)} {v.skip(2)}")
    print("{v.reversed()} {v.sorted()} {v.sorted_by((a, b) -> a > b)}")
    print("{v.enumerate()} {v.zip(["a", "b"])} {v.join("+")}")
    print("{v.sum()} {v.min()} {v.max()}")
    print("{v.map(n -> n * 2)} {v.filter(n -> n > 4)} {v.fold(0, (a, n) -> a + n)}")
    print("{v.any(n -> n > 7)} {v.all(n -> n > 7)} {v.count(n -> n > 4)} {v.find(n -> n > 5)} {v.position(n -> n > 5)}")

    v.push(1)
    print(v.pop())
    v.insert(0, 9)
    print(v.remove(1))
    v.extend([4, 4])
    v.swap(0, 1)
    print(v)
    v.reverse()
    print(v)
    v.sort()
    print(v)
    v.sort_by((a, b) -> a > b)
    print(v)
    v.each(n -> print(n))
    v.shuffle()
    print("{v.sorted()} {v.contains(v.pick() else 0)}")
    v.clear()
    print(v)
    print("{List<float>.with_size(2)} {List<string>.with_capacity(8).len}")
}
```

```text
3 false 3 none 5 8
true 2 [3, 8] [5, 3] [8]
[8, 3, 5] [3, 5, 8] [8, 5, 3]
[(0, 5), (1, 3), (2, 8)] [(5, "a"), (3, "b")] 5+3+8
16 3 8
[10, 6, 16] [5, 8] 16
true false 2 8 2
1
5
[3, 9, 8, 4, 4]
[4, 4, 8, 9, 3]
[3, 4, 4, 8, 9]
[9, 8, 4, 4, 3]
9
8
4
4
3
[3, 4, 4, 8, 9] true
[]
[0.0, 0.0] 0
```

## B.5 Strings

Positions in a string are counted in bytes.

| `string` | |
| --- | --- |
| `string.from_bytes(bytes: [byte]) -> string ! ParseError` | the string with these bytes, if they are valid UTF-8 |
| `s.len` | the number of bytes |
| `s.count() -> int` | the number of characters |
| `s.is_empty() -> bool` | whether there are none |
| `s.bytes() -> [byte]` | the bytes |
| `s.chars() -> [string]` | the characters, each as a string |
| `s.contains(needle: string) -> bool` | whether `needle` occurs in `s` |
| `s.find(needle: string) -> int?` | the position of its first occurrence |
| `s.starts_with(prefix: string) -> bool`, `s.ends_with(suffix: string) -> bool` | whether `s` begins or ends so |
| `s.strip_prefix(prefix: string) -> string?`, `s.strip_suffix(suffix: string) -> string?` | `s` without the beginning or end, if it has it |
| `s.slice(from: int, to: int) -> string` | the bytes from `from` up to but not including `to` |
| `s.cut(separator: string) -> (string, string)?` | the parts before and after the first occurrence of the separator |
| `s.split(separator: string) -> [string]` | the parts between occurrences of the separator |
| `s.lines() -> [string]` | the lines |
| `s.trim() -> string` | `s` without white space at either end |
| `s.to_upper() -> string`, `s.to_lower() -> string` | `s` in upper or lower case |
| `s.replace(from: string, to: string) -> string` | `s` with every occurrence of `from` replaced |
| `s.repeat(times: int) -> string` | `s` so many times over |
| `s.compare(other: string) -> int` | negative, zero or positive, by the order of `<` |
| `s.hash() -> int` | a hash value |

```koo
fn main() {
    s := "  Hello, Wörld  "
    t := s.trim()
    print("{string.from_bytes("hi".bytes()) else "?"} {t.len} {t.count()} {t.is_empty()}")
    print("{"hi".bytes()} {"wö".chars()}")
    print("{t.contains("lo")} {t.find("W")} {t.starts_with("Hell")} {t.ends_with("x")}")
    print("{t.strip_prefix("Hello, ")} {t.strip_suffix("?")} {t.slice(0, 5)}")
    print("{t.cut(", ")} {"a b c".split(" ")} {"one\ntwo".lines()}")
    print("{t.to_upper()} {t.to_lower()} {t.replace("l", "L")} {"ab".repeat(3)}")
    print("{"a".compare("b")} {"a".hash() == "a".hash()}")
}
```

```text
hi 13 12 false
[104, 105] ["w", "ö"]
true 7 true false
Wörld none Hello
("Hello", "Wörld") ["a", "b", "c"] ["one", "two"]
HELLO, WÖRLD hello, wörld HeLLo, WörLd ababab
-1 true
```

## B.6 Maps

| `[K: V]` | |
| --- | --- |
| `m[key]` | the value for a key, as a `V?` |
| `m[key] = value` | adds or replaces an entry |
| `m.len` | the number of entries |
| `m.is_empty() -> bool` | whether there are none |
| `m.get(key: K) -> V?` | the same as `m[key]` |
| `m.contains(key: K) -> bool` | whether there is an entry for the key |
| `m.keys() -> [K]`, `m.values() -> [V]`, `m.entries() -> [(K, V)]` | the keys, the values, and both, in the order the keys were added |
| `var` `m.insert(key: K, value: V)` | the same as `m[key] = value` |
| `var` `m.remove(key: K) -> V?` | removes an entry, and returns its value if there was one |
| `var` `m.update(key: K, start: V, f: fn(V) -> V)` | replaces the value `v` for the key by `f(v)`, or sets it to `f(start)` if there is no entry |

```koo
fn main() {
    var m := ["one": 1, "two": 2]
    m["three"] = 3
    m.insert("four", 4)
    print("{m["one"]} {m["nine"]} {m.len} {m.is_empty()} {m.get("two")} {m.contains("two")}")
    print("{m.remove("one")} {m.remove("one")}")
    m.update("two", 0, n -> n * 10)
    m.update("five", 0, n -> n + 5)
    print("{m.keys()} {m.values()}")
    print(m.entries())
}
```

```text
1 none 4 false 2 true
1 none
["two", "three", "four", "five"] [20, 3, 4, 5]
[("two", 20), ("three", 3), ("four", 4), ("five", 5)]
```

## B.7 Sets

| `Set<T>` | |
| --- | --- |
| `Set<T>.new() -> Set<T>` | an empty set |
| `Set<T>.from_list(items: [T]) -> Set<T>` | the set of the elements of a list |
| `s.len` | the number of members |
| `s.is_empty() -> bool` | whether there are none |
| `s.contains(item: T) -> bool` | whether the value is a member |
| `s.items() -> [T]` | the members, in the order they were added |
| `var` `s.insert(item: T) -> bool` | adds a member; `true` if it was not one already |
| `var` `s.remove(item: T) -> bool` | removes a member; `true` if it was one |

```koo
fn main() {
    var s := Set<int>.from_list([3, 1, 3])
    print("{s.len} {s.is_empty()} {s.contains(1)} {s.items()}")
    print("{s.insert(2)} {s.insert(2)} {s.remove(3)} {s.remove(3)}")
    print("{s} {Set<string>.new().len}")
}
```

```text
2 false true [3, 1]
true false true false
Set[1, 2] 0
```

## B.8 JSON

| | |
| --- | --- |
| `value.to_json() -> string` | the value as JSON; for `int`, `float`, `bool`, `string`, lists, optionals, maps with string keys, and the structures and enumerations made of them |
| `Type.from_json(text: string) -> Type ! JsonError` | the value that the text is; for structures, enumerations and `int` |

`JsonError` has the fields `message: string` and `path: string`. Section 8.7 gives the correspondence between types and JSON.

## B.9 Durations and Times

| `Duration` | |
| --- | --- |
| `Duration.parse(text: string) -> Duration ! ParseError` | the duration that the text is, in the form of a constant |
| `d.seconds() -> float` | the length in seconds |
| `d.millis() -> int`, `d.micros() -> int`, `d.nanos() -> int` | the length in whole thousandths, millionths and thousand-millionths of a second |
| `d.abs() -> Duration` | the length without its sign |
| `d.compare(other: Duration) -> int` | negative, zero or positive, by the order of `<` |

| `Time` | |
| --- | --- |
| `Time.parse(text: string) -> Time ! ParseError` | the time that a text in the form of RFC 3339 names |
| `Time.from_unix(seconds: int) -> Time` | the time so many seconds after the beginning of 1970 in UTC |
| `t.unix() -> int` | the whole seconds from the beginning of 1970 to the time |
| `t.unix_nanos() -> int` | the same in nanoseconds |
| `t.compare(other: Time) -> int` | negative, zero or positive, by the order of `<` |

Both have `to_string`, `to_json` and `from_json`. The operators on them are in Section A.5.3.

```koo
fn main() {
    d := Duration.parse("1min30s") else 0s
    print("{d} {d.seconds()} {d.millis()} {d.micros()} {d.nanos()} {(-d).abs()} {d.compare(2min)}")
    t := Time.parse("2026-01-31T12:00:00.25Z") else Time.from_unix(0)
    print("{t} {t.unix()} {t.unix_nanos()} {t.compare(Time.from_unix(0))} {Time.from_unix(86400)}")
    print("{d.to_json()} {t.to_json()} {Duration.parse("soon") else 0s}")
}
```

```text
1min30s 90.0 90000 90000000 90000000000 1min30s -1
2026-01-31T12:00:00.25Z 1769860800 1769860800250000000 1 1970-01-02T00:00:00Z
"1min30s" "2026-01-31T12:00:00.25Z" 0s
```

## B.10 `std/time`

| | |
| --- | --- |
| `time.now() -> Time` | the time, by the clock of the machine |
| `time.uptime() -> Duration` | how long the program has been running, by a clock that only goes forward |
| `time.sleep(length: Duration)` | waits; a length that is not positive does not wait |

## B.11 `std/io`

| | |
| --- | --- |
| `io.read_line() -> string?` | the next line of the standard input, without its newline; `none` at the end |
| `io.read_lines() -> [string]` | the rest of the lines of the standard input |
| `io.write(text: string)` | writes text on the standard output, with no newline |
| `io.eprint(text: string)` | writes text and a newline on the standard error output |
| `io.read_file(path: string) -> string ! IoError` | the contents of a file |
| `io.write_file(path: string, text: string) ! IoError` | makes or replaces a file |

`IoError` has the field `message: string`.

## B.12 `std/os`

| | |
| --- | --- |
| `os.args() -> [string]` | the arguments of the program |
| `os.env(name: string) -> string?` | the value of an environment variable |
| `os.exit(status: int)` | ends the program with a status |
| `T.from_env() -> T ! EnvError` | with no import, for a structure `T` whose fields are `int`, `float`, `bool`, `string`, `Duration` or optionals of them: each field from the environment variable of its name in capitals (Section 8.5) |

## B.13 `std/http`

| | |
| --- | --- |
| `http.App.new() -> http.App` | a server with no routes |
| `var` `app.get(path: string, handler)`, `post`, `put`, `patch`, `delete`, `any` | adds a route for a method, or for all |
| `var` `app.route(method: string, path: string, handler)` | adds a route for a method given by name |
| `var` `app.files(path: string, directory: string)` | serves the files of a directory under a path |
| `app.serve(port: int, workers: int, max_body: int = 1048576, log: bool = false) ! http.Error` | answers requests until the program is told to stop; a `port` that is not given is what the environment variable `PORT` says, or 8080, and `workers` what `WEB_CONCURRENCY` says, or one for each processor |

A handler is a `fn(http.Request) -> http.Response ! Failure`.

| `http.Request` | |
| --- | --- |
| `req.method`, `req.path` | fields: the method and the path, as strings |
| `req.param(name: string) -> string` | what `:name` in the route matched |
| `req.rest() -> string?` | what `*` at the end of the route matched |
| `req.query(name: string) -> string?` | a parameter of the query |
| `req.header(name: string) -> string?` | a header |
| `req.cookie(name: string) -> string?` | a cookie |
| `req.body() -> string` | the body |
| `req.form(name: string) -> string?` | a field of a form |
| `req.json() -> T ! JsonError` | the body, as the type that the context expects |

| `http.Response` | |
| --- | --- |
| `http.text(body: string, status: int = 200)` | plain text |
| `http.html(body: string, status: int = 200)` | HTML |
| `http.json(value: T, status: int = 200)` | a value as JSON |
| `http.send(body: string, content_type: string, status: int = 200)` | a body of a given type |
| `http.status(status: int, message: string = "")` | a status, with a message as the body |
| `http.redirect(to: string, status: int = 302)` | a redirection |
| `http.file(path: string, status: int = 200)` | a file |
| `response.header(name: string, value: string) -> http.Response` | the response with a header added |
| `response.cookie(name: string, value: string, max_age: int = -1) -> http.Response` | the response with a cookie set |
| `response.with_status(status: int) -> http.Response` | the response with another status |

A request of another server returns a `http.Reply`:

| | |
| --- | --- |
| `http.get(url: string, headers: [string: string] = [:], timeout: Duration = 30s) -> http.Reply ! http.Error` | asks for what is at a URL |
| `http.delete(url, headers, timeout)` | the same, with the method `DELETE` |
| `http.post(url: string, body: string, content_type: string = "application/json", headers: [string: string] = [:], timeout: Duration = 30s) -> http.Reply ! http.Error` | sends a body |
| `http.put(url, body, content_type, headers, timeout)`, `http.patch(...)` | the same, with those methods |
| `http.fetch(method: string, url: string, body: string = "", headers: [string: string] = [:], timeout: Duration = 30s) -> http.Reply ! http.Error` | a request with any method |
| `reply.status`, `reply.headers`, `reply.body` | fields: the status, the headers by their names in lower case, and the body |
| `reply.header(name: string) -> string?` | a header, however its name is written |
| `reply.ok() -> bool` | whether the status is from 200 to 299 |
| `reply.json() -> T ! JsonError` | the body, as the type that the context expects |

`http.Error` has the fields `kind: http.ErrorKind` and `message: string`, and `http.ErrorKind` is an enumeration: `Listen`, `Address`, `Connection`, `Timeout`, `Protocol`. Chapter 10 has programs that use these.

## B.14 `std/sql`

`std/sql/postgres` has one function:

| | |
| --- | --- |
| `postgres.connect(url: string, pool: int = 10, wait: Duration = 30s) -> sql.Pool ! sql.Error` | connects to the database at `postgres://user:password@host:port/database`, with at most `pool` connections, for one of which a statement waits `wait` at the longest |

A `sql.Query` is written as a string literal where one is expected: the values in braces become parameters of the statement, each an `int`, a `float`, a `bool`, a `string` or a `Time`, or an optional of one, which is NULL when it is `none`.

| `sql.Query` | |
| --- | --- |
| `sql.Query.raw(text: string) -> sql.Query` | a statement that is all text, with no parameters |
| `var` `query.append(more: sql.Query)` | adds a query to the end of this one, with its parameters |
| `query.text() -> string` | the statement as it is sent, with `$1`, `$2` and so on for the parameters |

| `sql.Pool` and `sql.Tx` | |
| --- | --- |
| `db.exec(query: sql.Query) -> int ! sql.Error` | runs a statement, and returns the number of rows it changed |
| `db.query(query: sql.Query) -> [sql.Row] ! sql.Error` | runs a statement, and returns its rows |
| `db.query_one(query: sql.Query) -> sql.Row? ! sql.Error` | the first row, if there is one |
| `db.query_row(query: sql.Query) -> sql.Row ! sql.Error` | the first row; fails with the kind `NoRows` if there is none |
| `db.transaction(body: fn(sql.Tx) -> T ! E, isolation: sql.Isolation = sql.Isolation.ReadCommitted) -> T ! E \| sql.Error` | of a pool only: runs `body` in a transaction, which is kept if `body` returns and undone if it fails |

`sql.Isolation` is an enumeration: `ReadCommitted`, `RepeatableRead`, `Serializable`.

| `sql.Row` | |
| --- | --- |
| `row.int(column: string) -> int ! sql.Error`, and `float`, `bool`, `string`, `time` | the value of a column; fails if it is NULL |
| `row.maybe_int(column: string) -> int? ! sql.Error`, and `maybe_float`, `maybe_bool`, `maybe_string`, `maybe_time` | the value of a column, or `none` if it is NULL |
| `row.columns() -> [string]` | the names of the columns |
| `T.from_row(row: sql.Row) -> T ! sql.Error` | for a structure `T` whose fields are of the five types or optionals of them: each field from the column of its name |

`sql.Error` has the fields `kind: sql.ErrorKind`, `message: string` and `code: string`, and `sql.ErrorKind` is an enumeration: `Connection`, `Constraint`, `Syntax`, `Serialization`, `Timeout`, `NoRows`, `Other`. Chapter 11 has programs that use these.

## B.15 `std/hash`, `std/encoding` and `std/crypto`

| | |
| --- | --- |
| `hash.sha256(data: [byte]) -> [byte]` | the SHA-256 digest, of 32 bytes |
| `encoding.hex(data: [byte]) -> string` | two hexadecimal digits for each byte |
| `encoding.from_hex(text: string) -> [byte] ! ParseError` | the bytes that hexadecimal text stands for |
| `encoding.base64(data: [byte]) -> string` | Base64 |
| `encoding.from_base64(text: string) -> [byte] ! ParseError` | the bytes that Base64 text stands for |
| `encoding.base64_url(data: [byte]) -> string` | Base64 with `-` and `_`, and no `=` |
| `encoding.from_base64_url(text: string) -> [byte] ! ParseError` | its reverse |
| `encoding.url(text: string) -> string` | the text with every byte but letters, digits and `-._~` written as `%` and two digits |
| `encoding.from_url(text: string) -> string ! ParseError` | its reverse; a `+` is a space |
| `crypto.hmac_sha256(key: [byte], data: [byte]) -> [byte]` | the HMAC-SHA-256 of the data under the key |
| `crypto.equal(a: [byte], b: [byte]) -> bool` | whether the two are the same, in a time that does not tell where they differ |
| `crypto.hash_password(password: string) -> string` | what to keep in place of a password |
| `crypto.verify_password(password: string, kept: string) -> bool` | whether a password is the one that was hashed |

```koo
import std/crypto
import std/encoding
import std/hash

fn main() {
    data := "koo".bytes()
    print(encoding.hex(hash.sha256(data)))
    print("{encoding.hex(data)} {encoding.from_hex("6b6f6f") else []}")
    print("{encoding.base64(data)} {encoding.from_base64("a29v") else []}")
    print("{encoding.base64_url(hash.sha256(data))} {encoding.from_base64_url("a29v") else []}")
    print("{encoding.url("1 + 1 = 2")} {encoding.from_url("1%20%2B%201") else "?"}")
    mac := crypto.hmac_sha256("key".bytes(), data)
    print("{encoding.hex(mac)} {crypto.equal(mac, mac)} {crypto.equal(mac, data)}")
    kept := crypto.hash_password("secret")
    print("{crypto.verify_password("secret", kept)} {crypto.verify_password("Secret", kept)}")
}
```

```text
c9b1366e59ac8faa5f9506b8eab9c0b41b9ff5229bd32e15f1cde55615bf32ca
6b6f6f [107, 111, 111]
a29v [107, 111, 111]
ybE2blmsj6pflQa46rnAtBuf9SKb0y4V8c3lVhW_Mso [107, 111, 111]
1%20%2B%201%20%3D%202 1 + 1
bd257f2ad597d10ff095571e6180ddd586bcfb73402d07f479721a258b2497c8 true false
true false
```

## B.16 `std/random`

| | |
| --- | --- |
| `random.int(low: int, high: int) -> int` | a number from `low` up to, and not including, `high`; panics if there is none |
| `random.float() -> float` | a number from zero up to, and not including, one |
| `random.bytes(count: int) -> [byte]` | so many bytes |
| `random.uuid() -> string` | a UUID of version 4 |

Every one is from the operating system's source of randomness for keys.

```koo
import std/random

fn main() {
    n := random.int(10, 20)
    x := random.float()
    print("{n >= 10 and n < 20} {x >= 0.0 and x < 1.0}")
    print("{random.bytes(8).len} {random.uuid().len}")
}
```

```text
true true
8 36
```

## B.17 `std/log`

| | |
| --- | --- |
| `log.debug(message)`, `log.info(message)`, `log.warn(message)`, `log.error(message)` | writes a line of that level on the standard error output |
| `log.set_level(level: log.Level)` | sets the lowest level that is written: `log.Level.Debug`, `Info`, `Warn`, `Error` or `Off` |

A message is written as a string literal, and the values in it are kept as fields of the line (Section 8.10). The environment variables `KOO_LOG_LEVEL`, `KOO_LOG` and `KOO_LOG_TIME` set the level, the form and whether the time is written.

```koo
import std/log

fn main() {
    count := 3
    log.debug("not written unless asked for")
    log.info("{count} items")
    log.warn("nearly full")
    log.set_level(log.Level.Error)
    log.warn("not written")
    log.error("full")
}
```

```log
INFO  3 items
WARN  nearly full
ERROR full
```

## B.18 `std/test`

| | |
| --- | --- |
| `test.assert(ok: bool)` | panics if the condition is false |
| `test.assert_eq(left: T, right: T)` | panics, showing both values, if they are not equal |
| `test.fail(message: string)` | panics with a message |

Appendix C describes how tests are written and run.
