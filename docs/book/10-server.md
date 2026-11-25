# Chapter 10: A Server

In *The C Programming Language* the last chapter shows what C was made for: it uses the system calls of UNIX to write parts of the standard library. Koo was made for writing services, programs that sit behind a network address and answer requests. This chapter writes one, with the standard package `std/http`, and in doing so it uses nearly everything in the book: functions as values, structures, optionals, errors, JSON, and an actor.

A program in this chapter does not end, so it does not have output in the usual sense. Instead, after each program we show some requests, on lines that begin with `>`, and under each the status code and the body of the server's answer. Like every output in this book they were put there by running the program. To try a program yourself, run it and make the requests with any HTTP client, for instance

```sh
koo run hello.koo &
curl localhost:8080/hello/Ada
```

## 10.1 Getting Started

Here is a complete server.

```koo
import std/http

fn main() ! http.Error {
    var app := http.App.new()
    app.get("/", req -> http.text("hello, world\n"))
    app.get("/hello/:name", req -> http.text("hello, {req.param("name")}\n"))
    app.serve(port: 8080)?
}
```

```http
> GET /
200
hello, world
> GET /hello/Ada
200
hello, Ada
> GET /goodbye
404
Not Found
> POST /
405
Method Not Allowed
```

An `http.App` is a value that holds the *routes* of a server. Each call of `app.get` adds one: a pattern for the path, and a *handler*, the function to call for a request whose path fits. A handler takes the request and returns the response. Here the handlers are lambdas, and `http.text` makes a response whose body is plain text.

`app.serve` starts listening, and does not return until the server is told to stop, by Ctrl-C or by the signal that systems send to ask a program to terminate; then it returns, and `main` ends in the ordinary way. It can fail, if the port is taken, say, and `main` passes the failure on.

A segment of a pattern that begins with a colon, like `:name`, matches any one segment of a path, and the handler gets what it matched from `req.param("name")`. The routes are tried in the order in which they were added, and the first that fits answers.

The last two requests show what a server does without being asked. A path that fits no route is answered with status 404, and a path that has a route, but not for the method of the request, with 405.

There is a function like `get` for each of the usual methods: `post`, `put`, `patch` and `delete`, and `any` for a route that answers them all.

## 10.2 Requests

The parameter of a handler is the request. It has two fields, `method` and `path`, and methods for the rest:

| | |
| --- | --- |
| `req.param(name)` | what a `:name` segment of the route matched |
| `req.rest()` | what a `*` at the end of the route matched, as an optional |
| `req.query(name)` | a parameter of the query string, as an optional |
| `req.header(name)` | a header, as an optional |
| `req.cookie(name)` | a cookie, as an optional |
| `req.body()` | the body, as a string |
| `req.form(name)` | a field of a body that is a submitted form, as an optional |
| `req.json()` | the body, read as JSON into the type that is expected |

What the client may have left out is an optional, and the handler says what to do without it in the usual way. `req.form` reads a body of the type `application/x-www-form-urlencoded`, which is what a browser sends when a form is submitted, and what `curl -d` sends.

```koo
import std/http

fn main() ! http.Error {
    var app := http.App.new()
    app.get("/search", req -> {
        term := req.query("q") else {
            return http.status(400, "what are you searching for?")
        }
        limit := int.parse(req.query("limit") else "10") else 10
        return http.text("the first {limit} results for {term}\n")
    })
    app.post("/shout", req -> http.text(req.body().to_upper()))
    app.post("/signup", req -> http.text("welcome, {req.form("name") else "stranger"}\n"))
    app.get("/docs/*", req -> http.text("the page {req.rest() else "?"}\n"))
    app.any("/method", req -> http.text("{req.method} {req.path}\n"))
    app.serve(port: 8080)?
}
```

```http
> GET /search?q=koo&limit=3
200
the first 3 results for koo
> GET /search?q=koo
200
the first 10 results for koo
> GET /search
400
what are you searching for?
> POST /shout hello, server
200
HELLO, SERVER
> POST /signup name=Ada&plan=free
200
welcome, Ada
> GET /docs/guide/intro.html
200
the page guide/intro.html
> DELETE /method
200
DELETE /method
```

A handler that is a block returns its response with `return`, like any function. The first handler leaves early, from inside the `else`, when the query has no `q`.

## 10.3 Responses

A response is a value, made by one of these functions:

| | |
| --- | --- |
| `http.text(body)` | plain text |
| `http.html(body)` | HTML |
| `http.json(value)` | the value as JSON |
| `http.send(body, content_type)` | a body of any other type |
| `http.status(code, message)` | a status with a short message, or with none |
| `http.redirect(to)` | a redirection to another address |
| `http.file(path)` | a file, sent as it is |

Each takes an optional `status:`, where the default is not wanted. A response has a method `header(name, value)` that returns the response with a header added, and `cookie(name, value)` that returns it with a cookie set.

```koo
import std/http

type Version = { name: string, major: int, minor: int }

fn main() ! http.Error {
    var app := http.App.new()
    app.get("/", req -> http.html("<h1>Koo</h1>\n"))
    app.get("/version", req -> http.json(Version { name: "koo", major: 0, minor: 1 }))
    app.get("/old", req -> http.redirect("/"))
    app.get("/data.csv", req -> http.send("a,b\n1,2\n", "text/csv"))
    app.post("/things", req -> http.text("made\n", status: 201).header("Location", "/things/1"))
    app.get("/teapot", req -> http.status(418, "I am a teapot"))
    app.serve(port: 8080)?
}
```

```http
> GET /
200
<h1>Koo</h1>
> GET /version
200
{"name":"koo","major":0,"minor":1}
> GET /old
302
> GET /data.csv
200
a,b
1,2
> POST /things
201
made
> GET /teapot
418
I am a teapot
```

## 10.4 JSON In and Out

Most services speak JSON, and Section 8.7 has already said nearly all there is to say. A value is sent by `http.json(value)`. A body is read by `req.json()`, which returns a value of whatever type the handler expects, the type being stated where the result is declared.

```koo
import std/http

type Item = { sku: string, qty: int = 1 }
type NewOrder = { customer: string, items: [Item] }
type Receipt = { customer: string, lines: int, pieces: int }

fn main() ! http.Error {
    var app := http.App.new()
    app.post("/orders", req -> {
        order: NewOrder = req.json()?
        pieces := order.items.map(item -> item.qty).sum()
        return http.json(Receipt { customer: order.customer, lines: order.items.len, pieces }, status: 201)
    })
    app.serve(port: 8080)?
}
```

```http
> POST /orders {"customer": "Bob", "items": [{"sku": "a-1"}, {"sku": "b-2", "qty": 3}]}
201
{"customer":"Bob","lines":2,"pieces":4}
> POST /orders {"customer": "Bob", "items": [{"sku": "a-1", "qty": "three"}]}
400
{"error":"expected a whole number, found a string","path":"/items/0/qty"}
> POST /orders {"customer": "Bob"}
400
{"error":"`items` is missing","path":"/items"}
> POST /orders this is not JSON
400
{"error":"expected an object, found something that is not JSON","path":""}
```

Look at what the handler does not contain. There is no code that checks that the body is JSON, that `customer` is present and is a string, that `items` is a list, that each `qty` is a number. All of that is the type `NewOrder`. The one line

    order: NewOrder = req.json()?

either produces a `NewOrder`, every field of which is what its type says, or fails; and a handler that fails with a `JsonError` is answered for you, with status 400 and a body that tells the client what was wrong and where. The three bad requests above got three precise answers, and the handler has no line for any of them.

## 10.5 Handlers That Fail

The `?` in the last program tells us that a handler may fail. The type of a handler is in fact

    fn(http.Request) -> http.Response ! Failure

so it may fail with any error at all. What the server does with a failure depends on what it is. A `JsonError`, as we saw, means that the client sent something wrong, and is answered with 400. Any other error is taken to be the server's own trouble: the client is answered with status 500 and told nothing more, since the particulars of a failure are not for strangers, and the error is written on the server's standard error output, for whoever runs it.

Failures that the client should hear about are turned into responses by the handler, which is a matter of a `match`, or of an `else`:

```koo
import std/http

error NotFound { name: string }
error Locked { name: string }

fn open(name: string) -> string ! NotFound | Locked {
    return match name {
        "readme" => "Read me first.\n"
        "secret" => fail Locked { name }
        _ => fail NotFound { name }
    }
}

fn main() ! http.Error {
    var app := http.App.new()
    app.get("/pages/:name", req -> {
        return match open(req.param("name")) {
            text => http.text(text)
            ! NotFound { name } => http.status(404, "there is no page called {name}")
            ! Locked { .. } => http.status(403, "that page is locked")
        }
    })
    app.get("/raw/:name", req -> http.text(open(req.param("name"))?))
    app.serve(port: 8080)?
}
```

```http
> GET /pages/readme
200
Read me first.
> GET /pages/missing
404
there is no page called missing
> GET /pages/secret
403
that page is locked
> GET /raw/readme
200
Read me first.
> GET /raw/missing
500
Internal Server Error
```

The second route passes every failure of `open` straight on, with `?`, and so its client gets a bare 500 where the first route's client gets an explanation.

A handler can also panic, if it has a bug. Section 10.7 says what happens then.

## 10.6 State

A handler is a function, and a function in Koo keeps nothing from one call to the next. A lambda that is kept holds copies, so a handler cannot even change a variable of `main`. Where, then, does a server keep what it knows?

In an actor. The server of this section keeps orders. The actor `Orders` owns the list of them; the handlers hold its handle, and ask.

```koo
import std/http

type Item = { sku: string, qty: int = 1 }
type NewOrder = { customer: string, items: [Item] }
type Order = { id: int, customer: string, items: [Item] }

error NoSuchOrder { id: int }

actor Orders {
    var orders: [Order] = []

    fn add(placed: NewOrder) -> Order {
        order := Order { id: orders.len + 1, customer: placed.customer, items: placed.items }
        orders.push(order)
        return order
    }

    fn find(id: int) -> Order ! NoSuchOrder {
        return orders.get(id - 1) else fail NoSuchOrder { id }
    }

    fn all() -> [Order] {
        return orders
    }
}

fn main() ! http.Error {
    orders := Orders.start()
    var app := http.App.new()

    app.post("/orders", req -> {
        placed: NewOrder = req.json()?
        return http.json(orders.add(placed), status: 201)
    })

    app.get("/orders", req -> http.json(orders.all()))

    app.get("/orders/:id", req -> {
        id := int.parse(req.param("id")) else {
            return http.status(400, "the number of an order is a number")
        }
        return match orders.find(id) {
            order => http.json(order)
            ! NoSuchOrder { .. } => http.status(404, "there is no order {id}")
        }
    })

    app.serve(port: 8080)?
}
```

```http
> POST /orders {"customer": "Ada", "items": [{"sku": "pen", "qty": 12}]}
201
{"id":1,"customer":"Ada","items":[{"sku":"pen","qty":12}]}
> POST /orders {"customer": "Bob", "items": [{"sku": "ink"}]}
201
{"id":2,"customer":"Bob","items":[{"sku":"ink","qty":1}]}
> GET /orders/2
200
{"id":2,"customer":"Bob","items":[{"sku":"ink","qty":1}]}
> GET /orders/7
404
there is no order 7
> GET /orders/two
400
the number of an order is a number
> GET /orders
200
[{"id":1,"customer":"Ada","items":[{"sku":"pen","qty":12}]},{"id":2,"customer":"Bob","items":[{"sku":"ink","qty":1}]}]
```

This is a small but complete service, of the shape that most services have. The types at the top are its vocabulary, and are at once the form of its JSON. The actor is its state, and the methods of the actor are the only things that can happen to that state, one at a time. The handlers translate between requests and those methods, and decide what each failure means to a client.

Consider `add`. It computes the number of the new order from the length of the list, and then appends to the list. Two requests may arrive at the same instant, and a server on a machine with many processors really will work on both at once. Yet no two orders can get the same number, and no order can be lost, because `add` is a method of an actor: the second request's `add` begins when the first's has ended. The handlers did not have to do anything to arrange this, and there is nothing they could have forgotten.

## 10.7 How the Server Runs

`serve` takes its settings as named arguments:

| | | |
| --- | --- | --- |
| `port:` | the port to listen on | 8080 |
| `workers:` | how many processes answer requests; 0 means one for each processor | 0 |
| `max_body:` | the largest body of a request that is accepted, in bytes | 1048576 |
| `log:` | whether to write a line on standard error for each request | `false` |

A server started with the default `workers:` answers requests on every processor of the machine. It does so with *processes*: `serve` starts one worker process for each processor, all of them listening on the one port, and itself stays behind to watch over them. A handler runs in one of the workers. This is why a panic in a handler does not bring a server down. The worker in which it happened ends, with the requests it was in the middle of, and the process that watches starts another in its place.

The actors of the program live in the process that watches, where `main` started them. A handler that asks an actor is asking across processes, which is slower than an ask within one, by a few millionths of a second, but is otherwise the same thing: the workers share the one actor, and the actor still answers one message at a time. That is how the orders of the last section stay in one list though they arrive at several processes.

`workers: 1` keeps everything in one process, with no process watching over it. That is the simplest arrangement to debug, and in it a handler's panic ends the server.

A server can also hand out the files of a directory:

```koo
import std/http

fn main() ! http.Error {
    var app := http.App.new()
    app.files("/static", "public")
    app.get("/", req -> http.redirect("/static/"))
    app.serve(port: 8080)?
}
```

With these files beside the program,

```file public/index.html
<h1>The shop</h1>
```

```file public/css/site.css
h1 { color: teal }
```

it answers

```http
> GET /static/
200
<h1>The shop</h1>
> GET /static/css/site.css
200
h1 { color: teal }
> GET /static/missing.png
404
Not Found
> GET /static/../main.koo
404
Not Found
```

`app.files(prefix, directory)` serves the files under a directory at the paths under a prefix, with `index.html` for a directory itself, and tells the client the type of each file from its name. A path cannot climb out of the directory with `..`, as the last request tried to.

On Linux, a server can be built so that the executable needs nothing else to be installed where it runs:

```sh
koo build --release --static server.koo
```

## 10.8 What Is Not There

The server speaks HTTP/1.1. It does not speak HTTPS or HTTP/2; where those are needed, it runs behind a proxy that does, which is how most services are deployed in any case. A body, of a request or a response, is handled whole, and cannot yet be read or written a piece at a time. And while a handler waits for something slow, the other requests that its worker has accepted wait with it.

There is no HTTP client in the library yet. The specification describes all of these; this book will describe them when they can be run.

What a server keeps for longer than it runs, it keeps in a database, and that is the subject of the next chapter.

**Exercise 10-1.** Add to the orders service a route that deletes an order, and one that changes the quantity of one of its items. What should each answer when the order does not exist?

**Exercise 10-2.** Add a route `/stats` that reports how many orders have been placed and how many pieces of each `sku` have been ordered. Should the counting be done by the handler, from `orders.all()`, or by a method of the actor? What are the costs of each?

**Exercise 10-3.** Write a server that keeps a set of short notes, each with a title and a text, with routes to add a note, to fetch one by its title, and to list the titles. Reject a note whose title is empty or is already taken, with statuses that say which.
