# Chapter 11: A Database

The server of the last chapter kept its orders in an actor, and lost them when it stopped. A service that means to keep what it is told puts it in a database. This chapter is about the standard package `std/sql`, with which a program sends statements to a PostgreSQL database and reads what comes back.

The programs of this chapter need a database to talk to. Each takes its address from the environment variable `DATABASE_URL`, as services usually do, and each was run, for the output shown here, against a database with nothing in it. One way to have such a database for an afternoon is

```sh
docker run --rm -p 5432:5432 -e POSTGRES_PASSWORD=secret postgres:16
export DATABASE_URL=postgres://postgres:secret@localhost:5432/postgres
```

The reader is assumed to know some SQL. This chapter is about how Koo and a database meet, not about databases.

## 11.1 Getting Started

```koo
import std/os
import std/sql
import std/sql/postgres

fn main() ! sql.Error {
    url := os.env("DATABASE_URL") else "postgres://localhost/shop"
    db := postgres.connect(url)?

    db.exec("create table items (sku text primary key, price int not null, stock int not null)")?
    added := db.exec("insert into items values ({"hammer"}, {1250}, {8}), ({"nails"}, {300}, {500})")?
    print("{added} items added")

    limit := 1000
    for row in db.query("select sku, price from items where price > {limit}")? {
        print("{row.string("sku")?} costs {row.int("price")?}")
    }
}
```

```text
2 items added
hammer costs 1250
```

There are two imports. `std/sql` has the types that every database shares: a pool of connections, a query, a row, an error. `std/sql/postgres` has the one function that is particular to PostgreSQL, `postgres.connect`, which takes an address of the form

    postgres://user:password@host:port/database

and returns a `sql.Pool`. A part that is left out has its usual value: the user `postgres`, no password, the host `localhost`, the port 5432, and a database with the user's name. A character that would be mistaken for punctuation, such as an `@` in a password, is written as `%` and its two hexadecimal digits, as in any address.

`connect` makes one connection before it returns, so that an address that leads nowhere is found out at once, where the program starts, and not at the first request an hour later. Everything in this package fails with one error type, `sql.Error`, which Section 11.4 describes; here `main` passes it on.

`db.exec` runs a statement for its effect, and returns the number of rows that it changed. `db.query` runs one for its rows, and returns them as a list.

## 11.2 Queries and Parameters

Look again at the statements of the first program. They are written as string literals with values in braces, like the argument of any `print`. They are not strings.

The parameter of `exec` and `query` has the type `sql.Query`, and a string literal written where a `sql.Query` is expected is not put together into text. The compiler takes it apart instead: the text between the braces becomes the statement, with a numbered placeholder where each value stood, and the values are sent to the database beside the statement, as *parameters*. The method `text` of a query shows the statement as it is sent:

```koo
import std/sql

fn find(name: string, year: int) -> sql.Query {
    return "select * from students where name = {name} and year > {year}"
}

fn main() {
    print(find("Ada", 2020).text())
    print(find("Robert'); drop table students;--", 2020).text())
}
```

```text
select * from students where name = $1 and year > $2
select * from students where name = $1 and year > $2
```

Both calls print the same thing, and that is the point. Whatever is in `name`, it is never part of the statement, so it cannot end a quotation and begin a statement of its own. The mistake known as SQL injection is made by building a statement out of text that came from outside; here the natural way to write a query does not build it from text at all.

A string that is not a literal is not a query, and the compiler says what to do:

```koo
import std/os
import std/sql
import std/sql/postgres

fn main() ! sql.Error {
    db := postgres.connect(os.env("DATABASE_URL") else "postgres://localhost/shop")?
    name := os.args().get(0) else "hammer"
    statement := "select * from items where sku = '" + name + "'"
    print(db.query(statement)?.len)
}
```

```error
error: expected `sql.Query`, found `string`
 --> main.koo:9:20
  |
9 |     print(db.query(statement)?.len)
  |                    ^^^^^^^^^ this is a `string`
  |
  = help: a query is written as a string literal where one is expected, so that the values in it are sent apart from its text; for text the program has put together itself there is `sql.Query.raw(text)` (Section 10.3)

1 error
```

A value in a query is an `int`, a `float`, a `bool`, a `string` or a `Time`, or an optional of one of these, and an optional that is `none` is sent as NULL. A list of them is sent as an array, so that the rows with any of several ids are asked for by `where id = any({ids})`, with `ids` an `[int]`. Because a value is sent as it is, a format (Section 8.2) would mean nothing, and is an error.

The database decides what type a parameter has from the place it stands in: compared with a column, it has the column's type. In the few places where a statement gives a parameter no type, as in `select {n}`, PostgreSQL takes it for text, and a cast says otherwise: `select {n}::int`.

A query is a value. It can be kept in a variable, returned from a function, and added to with `append`, which takes another query, parameters and all. This is how a statement that depends on what was asked is written:

```koo
import std/sql

fn search(name: string?, cheaper_than: int?) -> sql.Query {
    var query: sql.Query = "select sku, price from items where true"
    if wanted := name {
        query.append(" and sku like {wanted + "%"}")
    }
    if limit := cheaper_than {
        query.append(" and price < {limit}")
    }
    query.append(" order by sku")
    return query
}

fn main() {
    print(search(none, none).text())
    print(search("ha", none).text())
    print(search("ha", 2000).text())
}
```

```text
select sku, price from items where true order by sku
select sku, price from items where true and sku like $1 order by sku
select sku, price from items where true and sku like $1 and price < $2 order by sku
```

For the rare statement that a program must put together as text, such as one whose table name is computed, `sql.Query.raw(text)` makes a query of a string. What is in that string is then the program's to answer for.

## 11.3 Rows

A row is a `sql.Row`. A value is taken from it by the name of its column, with a method that says what type is wanted: `row.int("price")`, `row.float`, `row.bool`, `row.string` and `row.time`. Each fails, with a `sql.Error`, if the row has no such column, if the value is not of that type, or if it is NULL. For a column that may be NULL there are `maybe_int`, `maybe_float`, `maybe_bool`, `maybe_string` and `maybe_time`, which return an optional. `row.columns()` is the list of the columns' names.

Besides `query` there are two methods for a statement of which one row is wanted. `query_one` returns the first row, or `none` if there is none; `query_row` is for a statement that must produce a row, and fails if it does not.

```koo
import std/os
import std/sql
import std/sql/postgres

fn main() ! sql.Error {
    db := postgres.connect(os.env("DATABASE_URL") else "postgres://localhost/shop")?
    db.exec("create table people (id serial primary key, name text not null, born int, email text)")?
    db.exec("insert into people (name, born, email) values ({"Ada"}, {1815}, {"ada@example.com"})")?
    nobody: int? = none
    db.exec("insert into people (name, born) values ({"Homer"}, {nobody})")?

    count := db.query_row("select count(*) as n from people")?
    print("{count.int("n")?} people")

    for id in [1, 2, 3] {
        row := db.query_one("select * from people where id = {id}")? else {
            print("nobody has the number {id}")
            continue
        }
        name := row.string("name")?
        born := row.maybe_int("born")?
        email := row.maybe_string("email")? else "no address"
        print("{name}, {born}, {email}")
    }
    print(db.query_row("select * from people limit 1")?.columns())
}
```

```text
2 people
Ada, 1815, ada@example.com
Homer, none, no address
nobody has the number 3
["id", "name", "born", "email"]
```

`query_one` returns a `sql.Row? ! sql.Error`, an optional inside a result, and the line that calls it takes the two layers off one after the other: the `?` passes on a failure of the database, and the `else` deals with there being no row. The same line with `else fail NotFound { id }` is how a function says that the thing asked for does not exist.

More often than a value at a time, a row is wanted as a structure. For a structure whose fields are of the types a column can hold, the compiler writes a function `from_row`, as it writes `from_json` (Section 8.7): each field is read from the column of its name, and an optional field may be NULL. A field that is a list of `int`, `float`, `bool` or `string` is read from a column that is an array, as `row.ints`, `row.floats`, `row.bools` and `row.strings` read one.

```koo
import std/os
import std/sql
import std/sql/postgres

type Person = { id: int, name: string, born: int?, email: string? }

error NotFound { id: int }

fn person(db: sql.Pool, id: int) -> Person ! NotFound | sql.Error {
    row := db.query_one("select * from people where id = {id}")? else fail NotFound { id }
    return Person.from_row(row)?
}

fn add(db: sql.Pool, name: string, born: int?) -> Person ! sql.Error {
    row := db.query_row("insert into people (name, born) values ({name}, {born}) returning *")?
    return Person.from_row(row)?
}

fn main() ! sql.Error {
    db := postgres.connect(os.env("DATABASE_URL") else "postgres://localhost/shop")?
    db.exec("create table people (id serial primary key, name text not null, born int, email text)")?

    print(add(db, "Ada", 1815)?)
    print(add(db, "Homer", none)?.to_json())
    for id in [2, 7] {
        match person(db, id) {
            found => print(found.name)
            ! NotFound { id } => print("there is no person {id}")
            ! e: sql.Error => print("the database failed: {e}")
        }
    }
}
```

```text
Person { id: 1, name: "Ada", born: 1815, email: none }
{"id":2,"name":"Homer"}
Homer
there is no person 7
```

The statement of `add` ends in `returning *`, which makes PostgreSQL answer an `insert` with the row it made, number and all. And since a `Person` is an ordinary structure, it has a form as JSON, so that what comes out of the database can go straight out of a server.

## 11.4 Errors

Everything that can go wrong between a program and its database is a `sql.Error`, which has three fields:

    error sql.Error { kind: sql.ErrorKind, message: string, code: string }

The `message` says what happened, in the server's words when it is the server that refused. The `code` is the five characters by which PostgreSQL names the condition, such as `23505` for a value that had to be unique and was not; it is empty for an error that did not come from the server. And the `kind` sorts the errors into the few groups that a program treats differently:

| | |
| --- | --- |
| `Connection` | the database could not be reached, would not let the program in, or went away |
| `Constraint` | the statement would have broken a rule of the table: a duplicate key, a missing reference, a NULL where none is allowed |
| `Syntax` | the statement is wrong: bad SQL, or a table or column that does not exist |
| `Serialization` | the transaction could not be completed beside another, and may be tried again (Section 11.5) |
| `Timeout` | the statement ran longer than it was allowed, or waited too long for a lock (Section 11.8) |
| `NoRows` | `query_row` found no row |
| `Other` | anything else, including a value in a row that is not of the type asked for |

```koo
import std/os
import std/sql
import std/sql/postgres

error Taken { sku: string }

fn add(db: sql.Pool, sku: string, price: int) ! Taken | sql.Error {
    db.exec("insert into items values ({sku}, {price})") else e => {
        if e.kind == sql.ErrorKind.Constraint {
            fail Taken { sku }
        }
        fail e
    }
}

fn main() ! sql.Error {
    db := postgres.connect(os.env("DATABASE_URL") else "postgres://localhost/shop")?
    db.exec("create table items (sku text primary key, price int not null check (price > 0))")?

    for sku in ["hammer", "nails", "hammer"] {
        match add(db, sku, 100) {
            () => print("added {sku}")
            ! Taken { sku } => print("there is a {sku} already")
            ! e: sql.Error => print("failed: {e}")
        }
    }
    match db.query("select * from itmes") {
        rows => print(rows.len)
        ! e => print("{e.kind}, {e.code}: {e.message}")
    }
    match db.exec("insert into items values ({"saw"}, {-5})") {
        n => print(n)
        ! e => print("{e.kind}, {e.code}: {e.message}")
    }
}
```

```text
added hammer
added nails
there is a hammer already
Syntax, 42P01: relation "itmes" does not exist
Constraint, 23514: new row for relation "items" violates check constraint "items_price_check"
```

The function `add` turns the one failure it understands into an error of the program's own, and passes any other on unchanged. A program that tells its users "there is a hammer already" has matched on the kind; one that printed the message of the database to them would be telling them the names of its tables.

Interpolated into a string, a `sql.Error` is its message, followed by its code in parentheses if it has one.

## 11.5 Transactions

Some things must be done together or not at all. To move money from one account to another is to take it from the first and add it to the second; a program that stopped between the two would have destroyed the money. A *transaction* is a group of statements of which the database promises to do all or none.

`db.transaction` takes a function. It begins a transaction, calls the function with a `sql.Tx`, which has the methods `exec`, `query`, `query_one` and `query_row` of a pool, and then one of two things happens. If the function returns, the transaction is *committed*: what it did is kept. If the function fails, the transaction is *rolled back*: what it did is undone, as though none of it had happened, and the call fails with the error the function failed with.

```koo
import std/os
import std/sql
import std/sql/postgres

error Insufficient { account: string, balance: int, wanted: int }

fn transfer(db: sql.Pool, from: string, to: string, amount: int) ! Insufficient | sql.Error {
    db.transaction(tx -> {
        tx.exec("update accounts set balance = balance + {amount} where name = {to}")?
        row := tx.query_row("select balance from accounts where name = {from} for update")?
        balance := row.int("balance")?
        if balance < amount {
            fail Insufficient { account: from, balance, wanted: amount }
        }
        tx.exec("update accounts set balance = balance - {amount} where name = {from}")?
    })?
}

fn balances(db: sql.Pool) -> string ! sql.Error {
    var parts: [string] = []
    for row in db.query("select name, balance from accounts order by name")? {
        parts.push("{row.string("name")?} has {row.int("balance")?}")
    }
    return parts.join(", ")
}

fn main() ! sql.Error {
    db := postgres.connect(os.env("DATABASE_URL") else "postgres://localhost/bank")?
    db.exec("create table accounts (name text primary key, balance int not null)")?
    db.exec("insert into accounts values ({"ada"}, {100}), ({"bob"}, {20})")?

    for amount in [30, 500] {
        match transfer(db, "ada", "bob", amount) {
            () => print("moved {amount}")
            ! Insufficient { account, balance, wanted } => print("{account} has {balance}, not {wanted}")
            ! e: sql.Error => print("failed: {e}")
        }
        print(balances(db)?)
    }
}
```

```text
moved 30
ada has 70, bob has 50
ada has 70, not 500
ada has 70, bob has 50
```

The second transfer had already added 500 to the second account when it found the first too poor, and failed. The addition was undone with the rest: the balances after it are the balances before.

Notice the error set of `transfer`. The function given to `transaction` fails with `Insufficient`, by its `fail`, and with `sql.Error`, by its `?`; the call fails with what the function fails with, and with `sql.Error` for the beginning and the end of the transaction itself. How a function says that of a function it is given is the subject of Section 7.4.

Three more things are true of a transaction.

Its value is the value of the function, so a transaction can compute something:

    total := db.transaction(tx -> tx.query_row("select sum(balance) as s from accounts")?.int("s")?)?

A statement that fails ends it. After a failed statement PostgreSQL does nothing more in the same transaction, and will not commit it. So a function that swallows the error of a statement with `else` and returns as though all were well gets a `sql.Error` from `transaction` all the same, and its work is undone.

And a panic undoes it too. If the function panics, the connection is closed with the transaction open, and the database rolls it back.

Inside the function, statements are sent with the `sql.Tx` and not with the pool. One sent with the pool is not part of the transaction: it goes out on another connection, and if the pool has none left it would wait for the transaction that is waiting for it. That case, at least, the package sees, and fails the statement with an error that says to use the `sql.Tx`.

A `sql.Tx` is of use only inside the function it was given to, and only to the thread that is running that function: it cannot be sent to an actor, and one that was kept somewhere and used after the function returned gets an error.

**Isolation.** While a transaction runs, others run beside it, and how much each sees of what the others are doing is its *isolation*. PostgreSQL has three levels, and `transaction` takes the one wanted as a second argument: `sql.Isolation.ReadCommitted`, which is the default, `sql.Isolation.RepeatableRead`, and `sql.Isolation.Serializable`. At the last, the database promises that the result is what it would have been had the transactions run one after another, in some order. It keeps the promise by refusing a transaction that would break it, with an error of the kind `Serialization`. Nothing is wrong with such a transaction except its timing, and the thing to do is to run it again:

```koo
import std/os
import std/sql
import std/sql/postgres

// Runs `body` in a serializable transaction, and again if another got in its way, up
// to five times.
fn serializable<T>(db: sql.Pool, body: fn(sql.Tx) -> T ! sql.Error) -> T ! sql.Error {
    var attempt := 1
    while true {
        match db.transaction(body, isolation: sql.Isolation.Serializable) {
            value => return value
            ! e => {
                if e.kind != sql.ErrorKind.Serialization or attempt == 5 {
                    fail e
                }
                print("attempt {attempt} was in another's way")
                attempt += 1
            }
        }
    }
}

fn main() ! sql.Error {
    db := postgres.connect(os.env("DATABASE_URL") else "postgres://localhost/bank")?
    db.exec("create table accounts (name text primary key, balance int not null)")?
    db.exec("insert into accounts values ({"ada"}, {100}), ({"bob"}, {20})")?

    var interfered := false
    total := serializable(db, tx -> {
        total := tx.query_row("select sum(balance) as s from accounts")?.int("s")?
        if not interfered {
            // Another transaction changes what this one has read, before this one ends.
            interfered = true
            serializable(db, other -> {
                other.query("select * from accounts")?
                other.exec("update accounts set balance = balance + 1 where name = {"ada"}")?
            })?
        }
        tx.exec("insert into accounts values ({"total"}, {total})")?
        return total
    })?
    print("the total is {total}")
}
```

```text
attempt 1 was in another's way
the total is 121
```

The first attempt read a total of 120 and was about to record it when another transaction, which had read the same accounts, changed one of them and committed. To record 120 then would be to record something that was true at no moment, and the database refused. The second attempt read 121.

## 11.6 Pools, Actors and Servers

A `sql.Pool` is not one connection but as many as are needed, up to a limit, which is 10 unless `connect` is told otherwise:

    db := postgres.connect(url, pool: 20)?

Each statement takes a connection that is idle, or makes one if all are busy and the limit allows, and gives it back when it has its answer; a transaction keeps one from its beginning to its end. A statement that finds every connection busy, and the limit reached, waits for one, for thirty seconds at the longest, or for what `connect` is told with `wait:`; then it fails, with an error of the kind `Timeout`, since a service that waits without end for its database has stopped without saying so. A connection that the server closed while it was idle is replaced without the program's noticing, so a database that is restarted costs a program the statements that were running at the time and no others.

A statement is never sent twice. If the connection is lost after a statement has gone out and before its answer has come back, the program cannot know whether the database did it, and is told exactly that, by an error of the kind `Connection`: to send it again unasked might be to charge a customer twice. Whether it is safe to try again is for the program to say, which knows what the statement does.

A pool is a *handle*, like the handle of an actor (Section 9.2). To copy it is to have another name for the same pool, not another pool, and it may be passed to a function, kept in a structure, given to an actor as part of its state, and used from all of them at once. It is not itself an actor, and should not be put behind one: an actor does one thing at a time, and the purpose of a pool is to do many. The one thing a pool cannot be is part of a message (Section 9.6): an actor that needs the database is given the pool when it is started, as here.

```koo
import std/os
import std/sql
import std/sql/postgres

actor Clerk {
    db: sql.Pool
    name: string

    fn sell(sku: string, times: int) {
        for i in 0..times {
            db.transaction(tx -> {
                tx.exec("update items set stock = stock - 1 where sku = {sku}")?
                tx.exec("insert into sales (clerk, sku) values ({name}, {sku})")?
            }) else e => print("{name} could not sell: {e}")
        }
    }

    fn done() -> bool {
        return true
    }
}

fn main() ! sql.Error {
    db := postgres.connect(os.env("DATABASE_URL") else "postgres://localhost/shop", pool: 4)?
    db.exec("create table items (sku text primary key, stock int not null)")?
    db.exec("create table sales (id serial primary key, clerk text not null, sku text not null)")?
    db.exec("insert into items values ({"nails"}, {1000})")?

    var clerks: [Clerk] = []
    for i in 0..8 {
        clerks.push(Clerk.start(db: db, name: "clerk {i}"))
    }
    for clerk in clerks {
        clerk.send.sell("nails", 50)
    }
    for clerk in clerks {
        clerk.done()
    }
    left := db.query_row("select stock from items where sku = {"nails"}")?.int("stock")?
    sold := db.query_row("select count(*) as n from sales")?.int("n")?
    print("{sold} sold, {left} left")
}
```

```text
400 sold, 600 left
```

Eight actors, on as many threads as the machine has processors, share four connections, and each sale is a transaction, so the stock and the record of sales agree at every moment.

In a server, the handlers hold the pool as they held the handle of an actor in Section 10.6. Here is the orders service of that section with its orders where they will survive it:

```koo
import std/os
import std/http
import std/sql
import std/sql/postgres

type NewOrder = { customer: string, sku: string, qty: int = 1 }
type Order = { id: int, customer: string, sku: string, qty: int }

error NoSuchOrder { id: int }

fn place(db: sql.Pool, new: NewOrder) -> Order ! sql.Error {
    row := db.query_row("""
        insert into orders (customer, sku, qty)
        values ({new.customer}, {new.sku}, {new.qty})
        returning *
        """)?
    return Order.from_row(row)?
}

fn find(db: sql.Pool, id: int) -> Order ! NoSuchOrder | sql.Error {
    row := db.query_one("select * from orders where id = {id}")? else fail NoSuchOrder { id }
    return Order.from_row(row)?
}

fn main() ! sql.Error | http.Error {
    db := postgres.connect(os.env("DATABASE_URL") else "postgres://localhost/shop")?
    db.exec("""
        create table if not exists orders (
            id serial primary key,
            customer text not null,
            sku text not null,
            qty int not null check (qty > 0)
        )
        """)?

    var app := http.App.new()
    app.post("/orders", req -> {
        new: NewOrder = req.json()?
        match place(db, new) {
            order => return http.json(order).with_status(201)
            ! e => {
                if e.kind == sql.ErrorKind.Constraint {
                    return http.status(422, "an order is for at least one piece\n")
                }
                fail e
            }
        }
    })
    app.get("/orders/:id", req -> {
        id := int.parse(req.param("id")) else {
            return http.status(400, "an order has a number\n")
        }
        match find(db, id) {
            order => return http.json(order)
            ! NoSuchOrder { id } => return http.status(404, "there is no order {id}\n")
            ! e: sql.Error => fail e
        }
    })
    app.serve(port: 8080)?
}
```

```http
> POST /orders {"customer": "ada", "sku": "hammer", "qty": 2}
201
{"id":1,"customer":"ada","sku":"hammer","qty":2}
> POST /orders {"customer": "bob", "sku": "nails"}
201
{"id":2,"customer":"bob","sku":"nails","qty":1}
> POST /orders {"customer": "bob", "sku": "nails", "qty": 0}
422
an order is for at least one piece

> GET /orders/2
200
{"id":2,"customer":"bob","sku":"nails","qty":1}
> GET /orders/9
404
there is no order 9

> GET /orders/nine
400
an order has a number

```

A statement in a text block (Section 2.3) may run over as many lines as it likes, and its values are parameters all the same.

Two things about a server and its database follow from how the server runs (Section 10.7). Each worker is a process, and a connection belongs to the process that made it, so each worker has a pool of its own, which it fills as it needs: the limit given to `connect` is a limit for each worker, and the database sees as many connections as all of them together have made. And a handler that waits for the database does not hold up its worker, which answers other requests meanwhile, so one worker has as many statements in hand at one time as its pool has connections. The pool is the measure of what a service asks of its database, and it is worth choosing: workers times `pool:` is the most connections the service will ever hold.

## 11.7 Encryption

A database on the same machine, or in the same private network, may be spoken to in the clear. One that is reached across a network that others share must not be, and the databases that are rented by the hour refuse to be. Whether what passes between a program and its database is encrypted is said in the address, with `sslmode`, by the names PostgreSQL itself uses:

| | |
| --- | --- |
| `?sslmode=disable` | never |
| `?sslmode=prefer` | if the server will; this is what an address that says nothing means |
| `?sslmode=require` | always, or the connection fails; but whoever answers is believed to be the database, unless `sslrootcert` names an authority, which its certificate must then be signed by |
| `?sslmode=verify-ca` | always, and the server must show a certificate that an authority vouches for |
| `?sslmode=verify-full` | the same, and the certificate must be that of the host in the address |

Only the last two keep out someone who stands between the program and the database and pretends to be each to the other, and `verify-full` is the one to use across a network that is not one's own. The authorities are those the system trusts, or, with `&sslrootcert=/path/to/file`, the ones whose certificates are in a file, which is how a database with a certificate of one's own making is trusted.

The encryption is TLS, and it is not built into a program: it is done by the OpenSSL that is installed on the machine where the program runs, found when a connection first needs it. On Linux that is the library `libssl.so.3`, which most systems have and a small container may have to be given (on Debian and Ubuntu it is the package `libssl3`, and the authorities are the package `ca-certificates`); on macOS, the one that Homebrew installs as `openssl@3`. A program built with `--static` cannot load a library, and so cannot encrypt: with `prefer` it connects without, as does any program on a machine with no OpenSSL, and with the three stricter modes it fails with an error that says why.

The password is safe in either case from someone who only listens. PostgreSQL asks for it in one of three ways, and the usual one, SCRAM-SHA-256, never sends the password itself; the package answers whichever it is asked.

## 11.8 What Is Not There

The package speaks to PostgreSQL and to nothing else yet.

Of the parameters an address may have, the package acts on `sslmode`, `sslrootcert` and `options`. The others that PostgreSQL's own client knows, `connect_timeout` or `application_name` or `channel_binding`, are let pass without effect, since the addresses that hosting services hand out carry them. A parameter that client does not know is refused, with the nearest one that it does: `sslmod=verify-full`, which would otherwise ask for nothing, is an error that says `sslmode`.

The rows of `query` arrive all at once, as a list. For an answer too large to hold whole there is `each`, which is given the query and a function, and gives the function the rows one at a time, fetching them from the database a thousand at a time, or as many as `batch:` says, so that no more than that many are in memory at once:

    seen := db.each("select * from orders order by id", row -> {
        total += row.float("total")?
    })?

`each` returns the number of rows. If the function fails, no more are read and the failure is what `each` fails with. On a pool it runs in a transaction of its own; a `sql.Tx` has it too, for reading within one. A value in a row is read as one of the five types of Section 11.2, or as a list of one from an array of one dimension: a column of another type, such as an array of arrays or a `numeric` of more digits than a `float` holds, is read as a `string` in the form PostgreSQL writes it, or cast in the statement to something that fits. Settings for a session, such as a limit on how long a statement may run, go in the address, as `?options=-c%20statement_timeout%3D5s`, where `%20` is a space and `%3D` an equals sign.

There is nothing here that writes SQL for the program, makes tables from structures, or changes a database from one version of its design to the next. Statements are SQL, written by the programmer and checked by the database.

A program has at most sixteen databases open, and no way to close one: `postgres.connect` is called once for each database, when the program starts, and what it returns is kept. `copy ... from stdin`, which would have the program send rows in a form of their own, is refused with an error, since the package has no way to send them; rows are inserted. And a password is sent as it is written: PostgreSQL itself rewrites a few characters of a password when it stores one, a superscript two as a plain two and the like, and a password with such a character in it is refused, where one of ordinary letters, accented or not, is not.

**Exercise 11-1.** Write a program that keeps a table of notes, each with a title and a text, and takes its orders from its arguments: `add title text`, `show title`, `list`, `remove title`. What should `add` say when the title is taken, and how does the program know that it is?

**Exercise 11-2.** In `transfer`, the row of the account that pays is read with `for update`. Remove those two words, and work out what two transfers from the same account, running at once, can then do. Then keep them removed and make the transaction `Serializable` instead. What changes?

**Exercise 11-3.** Change the function `serializable` of Section 11.5 to wait between attempts, twice as long each time, using `time.sleep` (Section 8.8). Why is it better to wait?

**Exercise 11-4.** Add to the orders service a route `GET /customers/:name/orders` that answers with all the orders of a customer, and a route `DELETE /orders/:id`. Give the first a parameter `?limit=n`.
