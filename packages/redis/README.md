# A client for Redis

This directory is a package: a client for Redis, and for what speaks as Redis does,
written in Koo on `std/net` (Section 10.9 of the book). A program has it by having the
directory, beside its own files or wherever its import says (Section 4.4):

```koo
import ./redis

cache := redis.connect(os.env("REDIS_URL") else "redis://localhost:6379")?
cache.set("greeting", "hello", ttl: 60s)?
word := cache.get("greeting")?        // string?
hits := cache.incr("hits")?
job := queue.pop("jobs", wait: 5s)?   // string?, none if none came in five seconds
```

| | |
| --- | --- |
| `redis.connect(url, pool: 10, timeout: 5s, most: 67108864) -> redis.Client ! redis.Error` | `redis://[user:password@]host[:port][/database]`, a password alone as `:password@`, and an `@`, `:`, `/` or `%` inside either written as `%40`, `%3A`, `%2F`, `%25` (a `+` is itself); one command is sent at once, so that a wrong address or password is known when the program starts. Each connection is told who is asking and which database when it is new, and two clients of one server with different users or databases do not share connections |
| `cache.get(key) -> string? ! redis.Error` | the value of a key, if it has one |
| `cache.set(key, value, ttl: 0s)` | gives a key a value, for ever or for so long |
| `cache.del(keys) -> int` | removes keys, and says how many there were |
| `cache.incr(key, by: 1) -> int` | adds to the number a key holds |
| `cache.expire(key, ttl) -> bool` | says how much longer a key is kept |
| `cache.push(key, values) -> int`, `cache.pop(key, wait: 0s) -> string?` | a list as a queue: values in at the end, and out at the beginning, waiting for one if asked to |
| `cache.atomically(commands) -> [redis.Reply]` | several commands done as one, with no other client's between them (`MULTI` and `EXEC`). None is done if the server refuses one as it is given; one that fails when it is done does not undo the others, as a Redis is, and is a failure here that says which |
| `cache.command(words) -> redis.Reply ! redis.Error` | any command, by its words; the answer is `Nothing`, `Text`, `Number` or `Items`. A command that changes what a connection means for what is sent on it next (`MULTI`, `SELECT`, `SUBSCRIBE` and their like) is refused: a command goes out on whichever connection is free |

A program that takes jobs from a list, and finishes the one it is at when it is told
to stop:

```koo
while not os.stopping() {
    job := queue.pop("jobs", wait: 2s)? else { continue }
    work(job)
}
```

A `redis.Error` has a `kind`: `Address`, `Connection`, `Timeout`, `Refused` (the server
would not do it, and `message` is what it said) or `Protocol`. The client as text
(`"{cache}"`) says where it is and who asks, and not the password.

What the package offers is what is listed here; the rest of the file is its own. A
value is text; one that is not is a failure of the kind `Protocol`.

The tests of Koo run it against a real Redis when they are given one
(`KOO_TEST_REDIS=redis://localhost:6379`): by a program, and by the handlers and an
actor of a server with one worker and with three.
