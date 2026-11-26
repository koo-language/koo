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
| `"{req}"` | the request as text: its method and its path, as `GET /orders/7` |
| `req.cookie(name)` | a cookie, as an optional |
| `req.body()` | the body, as a string |
| `req.bytes()` | the body, as the bytes it is, for one that is not text |
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
| `http.bytes(body, content_type: "image/png")` | a body that is bytes, and need not be text; `application/octet-stream` unless a type is said |
| `http.status(code, message)` | a status with a short message, or with none |
| `http.redirect(to)` | a redirection to another address |
| `http.file(path)` | a file, sent as it is |

Each takes an optional `status:`, where the default is not wanted. A response has a method `header(name, value)` that returns the response with a header added, and `cookie(name, value)` that returns it with a cookie set. A header `Content-Type` takes the place of the type the response had, so there is never more than one; the length of the body and the other headers that say how the response is carried are written by the server, whatever a handler adds.

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

Most services speak JSON, and Section 8.7 has already said nearly all there is to say. A value is sent by `http.json(value)`. A body is read by `req.json()`, which returns a value of whatever type the handler expects, the type being stated where the result is declared: a structure or an enumeration, or a list or a map with string keys of them, as `orders: [NewOrder] = req.json()?` for a body that is an array.

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

What several handlers must all do first, such as asking who the client is, is written once, in a function that takes a handler and returns another. Its parameter is `take`, because the handler it is given is kept inside the one it returns (Section 4.6):

```koo
import std/http

fn members_only(handler: take fn(http.Request) -> http.Response ! Failure) -> fn(http.Request) -> http.Response ! Failure {
    return req -> {
        if (req.query("key") else "") != "s3cret" {
            return http.status(401, "members only")
        }
        return handler(req)?
    }
}

fn main() ! http.Error {
    var app := http.App.new()
    app.get("/", req -> http.text("welcome\n"))
    app.get("/accounts", members_only(req -> http.text("the accounts\n")))
    app.serve(port: 8080)?
}
```

```http
> GET /
200
welcome
> GET /accounts
401
members only
> GET /accounts?key=s3cret
200
the accounts
```

The handler that `members_only` returns answers for itself where it must, and otherwise passes the request on, and whatever the handler inside answers or fails with comes back through it. What is done after a handler, such as adding a header to every response or writing how long the answer took, is written the same way, around the call.

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
| `port:` | the port to listen on | what the environment variable `PORT` says, and 8080 if it is not set |
| `workers:` | how many processes answer requests | what `WEB_CONCURRENCY` says, which is one or more, and one for each processor if it is not set |
| `max_body:` | the largest body of a request that is accepted, in bytes | 1048576 |
| `log:` | whether to write a line on standard error for each request | `false` |

A server started with the default `workers:` answers requests on every processor of the machine. It does so with *processes*: `serve` starts one worker process for each processor, all of them listening on the one port, and itself stays behind to watch over them. The processors that count are those the program may use. On Linux a program in a container is often allowed less than the machine has, a processor and a half of sixteen, say, and then there is a worker for each processor it is allowed, a part counting as a whole: two, in that case, and not sixteen. A handler runs in one of the workers. If a worker dies, the process that watches starts another in its place.

A panic in a handler does not bring a server down, nor even a worker. A panic is a bug in the handler, and what it costs is the request that met it: the panic is written on the standard error output, as always, the client is answered with 500, and the worker goes on to its next request. Whatever the handler was in the middle of is abandoned: a transaction it had open is undone (Section 11.5), and an actor it was asking starts again (Section 9.8).

One panic costs more. A handler that calls itself too deeply runs out of stack. Its request is answered with 500 like the others, and the handlers that were waiting in the same worker are answered as they would have been; but a worker that has been stopped at such a moment is not to be trusted with new requests, so it takes none, answers what it has in hand, within ten seconds at most, and ends. (A handler that runs out of stack in the middle of printing, or of asking an actor, cannot be brought back at all: it is answered, and its worker ends at once.) A method of an actor that runs out of stack is a panic in the actor like any other: the handler that asked answers 500, the actor starts again (Section 9.8), and the server goes on. The process that watches starts another worker in its place; a server of one worker has no such process, and ends with the status of a panic, to be started again by whatever runs it.

The actors of the program live in the process that watches, whether `main` started them or a handler does: an actor that a handler starts, one for each session or each account, is started there, and what it is started with is sent there as a message is, so it must be what a message may be (Section 9.6). A handler that asks an actor is asking across processes, which is slower than an ask within one, by a few millionths of a second, but is otherwise the same thing: the workers share the one actor, and the actor still answers one message at a time. A tell crosses the same way, and comes back when the actor's process has the message, or has said that there is no room for it yet (Section 9.9), so from a handler it costs what an ask does. That is how the orders of the last section stay in one list though they arrive at several processes.

`workers: 1` keeps everything in one process, with no process watching over it. That is the simplest arrangement to debug.

A worker answers many requests at one time, though it is one thread. Most of what a handler does is wait: for a database to answer, for another server, for a time to pass. A handler that waits is left where it is, and the worker goes on with other requests; when what the handler waited for has come, the worker comes back to it and the handler goes on, never knowing that it stood still. So a slow query holds up the request that made it and no other, and there is nothing to write differently for it: `db.query`, `http.get` and `time.sleep` are the same calls as in any program. What a worker cannot do meanwhile is compute. A handler that spends a second on arithmetic keeps its worker for that second, and it is for such handlers, and for the processors a machine has, that there are several workers. A worker has room for 127 handlers that wait; requests beyond those wait their turn. Before it comes to that, a worker with about a hundred handlers in hand leaves new connections to its neighbours, so that waiting requests are shared out among the workers and a quick one finds a worker with room. A handler opens files of its own, to ask an actor, a database or another server, and where the system allows a process few files a worker takes only as many connections as leave its handlers theirs. An ask of an actor is a wait of this kind too, where the actor is in the process that watches: a handler that asks an actor which takes its time is left where it is, and its worker answers other requests until the actor has answered. And the process that watches answers for its actors on several threads, one for each actor that is busy, so that an actor which takes its time holds up those who ask it, one after another, and nobody who asks another actor. An actor is held by its method and not by the sending of the answer: a long answer to a worker that is slow to take it goes out as the worker takes it, and the actor answers its next message meanwhile. In a server of one worker the actors are in the worker's own process and an ask is a call (Section 9.3): there the worker waits with the handler, so an actor that such a server's handlers ask should be quick itself, and leave what is slow, a query or a call to another server, to the handlers.

A server that is told to stop, by the signal that systems send for the purpose, takes no new connection, answers the requests it has in hand, waiting handlers and all, and ends; `serve` then returns, and `main` goes on to whatever it has left to do. A request that was already on its way is still answered, with a header that says the connection ends with it. An answer that is still going out, to a client that takes it slowly, is sent to the end, for as long as half a minute.

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

`app.files(prefix, directory)` serves the files under a directory at the paths under a prefix, with `index.html` for a directory itself, the one at the prefix or any inside it; what is served is what is really in the directory, so a link in it is followed while it leads to something inside, and is nothing when it leads out, and tells the client the type of each file from its name. A path cannot climb out of the directory with `..`, as the last request tried to.

On Linux, a server can be built so that the executable needs nothing else to be installed where it runs:

```sh
koo build --release --static server.koo
```

And it can be built for Linux on another machine, a Mac included, with everything linked in the same way; Section C.1 says what that takes.

```sh
koo build --release --target linux-x86-64 server.koo
```

## 10.8 Asking Other Servers

A service is seldom alone. To answer its own requests it asks other services, over the same protocol, and `std/http` is a client as well as a server. `http.get(url)` makes a request and returns the answer, a `http.Reply`, which has the fields `status`, `headers` and `body`, and three methods: `reply.header(name)` is a header, if the answer has it, however its name is written; `reply.ok()` says whether the status is one of success, 200 to 299; and `reply.json()` reads the body into the type that is expected, like `req.json()`.

This program is two services in one, so that it can be run by itself: one that knows what things cost, and one, for people, that asks it.

```koo
import std/encoding
import std/http

type Price = { sku: string, cents: int }
type Quote = { sku: string, qty: int, total: string }

// What an item costs, by the price service; `none` if it has no such item.
fn price(sku: string) -> Price? ! http.Error | JsonError {
    reply := http.get("http://localhost:8080/prices/{encoding.url(sku)}", timeout: 2s)?
    if reply.status == 404 {
        return none
    }
    found: Price = reply.json()?
    return found
}

fn main() ! http.Error {
    var app := http.App.new()

    // The price service.
    app.get("/prices/:sku", req -> {
        sku := req.param("sku")
        cents := ["hammer": 1250, "nails": 300][sku] else {
            return http.status(404, "no such item\n")
        }
        return http.json(Price { sku, cents })
    })

    // The service that people use.
    app.get("/quote/:sku", req -> {
        qty := int.parse(req.query("qty") else "1") else 1
        match price(req.param("sku")) {
            found => {
                item := found else {
                    return http.status(404, "we do not sell that\n")
                }
                total := "{float.from(item.cents * qty) / 100.0:.2}"
                return http.json(Quote { sku: item.sku, qty, total })
            }
            ! e: http.Error => {
                if e.kind == http.ErrorKind.Timeout {
                    return http.status(504, "the price service is slow today\n")
                }
                return http.status(502, "the price service cannot be reached\n")
            }
            ! e: JsonError => return http.status(502, "the price service made no sense\n")
        }
    })
    app.serve(port: 8080, workers: 2)?
}
```

```http
> GET /quote/hammer?qty=3
200
{"sku":"hammer","qty":3,"total":"37.50"}
> GET /quote/nails
200
{"sku":"nails","qty":1,"total":"3.00"}
> GET /quote/saw
404
we do not sell that

```

An answer with a status of failure is still an answer. `http.get` fails, with a `http.Error`, only when there is no answer to return, and the error's `kind` says why: `Address`, for a URL that is not one; `Connection`, for a server that could not be found or reached, or whose certificate is not to be trusted; `Timeout`, for one that did not answer in time; and `Protocol`, for an answer that is not HTTP. What to make of a 404 or a 500 is left to the program, which here turns a 404 of the price service into `none`.

A request is given a time to finish in, with `timeout:`, which is thirty seconds unless it is said. The time covers everything, from finding the server to the last byte of its answer. A time of zero or less, which what is left of a budget may be, is a timeout at once, and the request is not made. A service that waits without limit on another is a service that hangs when the other does, so the time should be chosen, and should be short.

There is a function for each of the usual methods: `http.get` and `http.delete`, which send no body, and `http.post`, `http.put` and `http.patch`, which take one after the URL, as a string. The body is taken to be JSON unless `content_type:` says otherwise, so that a value is sent with

    http.post("https://api.example.com/orders", order.to_json())

Headers go in a map, `headers: ["Authorization": "Bearer {token}"]`, and `http.fetch(method, url)` is the general form, for any method.

A URL that begins `https` is asked over TLS, and the server's certificate must be one that the system's authorities vouch for and must name the host: there is no way to ask for less. The TLS is the one that the database package uses, with what Section 11.7 says of it.

## 10.9 What Is Not There

The server listens at every address of its machine, of both kinds, IPv4 and IPv6, where the machine has both. The client and the database package reach both kinds as well. A host that has an address of the older kind is reached at that one, and a host that has only an IPv6 address at that; and an IPv6 address written out stands in brackets, in the address of a server as in that of a database: `http://[::1]:8080/`, `postgres://[::1]:5432/shop`.

The server speaks HTTP/1.1. It does not speak HTTPS or HTTP/2; where those are needed, it runs behind a proxy that does, which is how most services are deployed in any case. A body, of a request or a response, is handled whole, and cannot yet be read or written a piece at a time. A handler runs to its end whether or not its client is still there: one that waits on something slow, for a client that has given up, keeps its place among its worker's handlers until what it waits for has come. Clients that give up and ask again can so fill a server whose handlers wait without limit, which is one more reason to give every wait a time.

The client keeps a connection that has carried a request, for a minute, and uses it for the next request to the same server, so that a service which asks another all day does not make a connection each time. An answer that sends the client elsewhere, with a status of 301, 302, 303, 307 or 308, is returned like any other, with the new place in its `location` header, unless the request says how many such answers to follow: `http.get(url, redirects: 5)`. The time given is for the whole journey. On the way, what says who is asking (`Authorization`, `Cookie`) is not passed to a server other than the one it was sent to, and a request that began over `https` is not sent on in the open: it stops at the answer that would have sent it there. A body that is not text is sent with `http.fetch("POST", url, bytes: data)`, and had from an answer with `reply.bytes()`. The specification describes more; this book will describe it when it can be run.

What a server keeps for longer than it runs, it keeps in a database, and that is the subject of the next chapter.

**Exercise 10-1.** Add to the orders service a route that deletes an order, and one that changes the quantity of one of its items. What should each answer when the order does not exist?

**Exercise 10-2.** Add a route `/stats` that reports how many orders have been placed and how many pieces of each `sku` have been ordered. Should the counting be done by the handler, from `orders.all()`, or by a method of the actor? What are the costs of each?

**Exercise 10-3.** Write a server that keeps a set of short notes, each with a title and a text, with routes to add a note, to fetch one by its title, and to list the titles. Reject a note whose title is empty or is already taken, with statuses that say which.
