# Chapter 9: Actors

Everything in this book so far has rested on one rule: a value belongs to whoever holds it, and nobody else can change it. The rule makes programs easy to reason about, and it leaves a question. Some state is meant to be shared. The stock of a shop is one stock, whoever is buying; a counter of requests is one counter, whichever request is being counted. Where does such state live, when there are no variables outside functions, and how do the parts of a program that run at the same time get at it without getting in each other's way?

The answer in Koo is the *actor*. An actor is a piece of state with an owner. The state can be reached only through the actor's methods, and the actor runs one method at a time, to the end, before it begins the next. Everyone who holds the actor's *handle* talks to the same actor. Since no two things ever happen to an actor's state at once, there is nothing to lock, and since nothing outside the actor can touch the state, there is nothing to forget to lock.

## 9.1 A First Actor

An actor is declared like a structure with its methods inside it.

```koo
actor Counter {
    var count: int = 0

    fn add(n: int) -> int {
        count += n
        return count
    }

    fn total() -> int {
        return count
    }
}

fn bump(counter: Counter) {
    counter.add(10)
}

fn main() {
    counter := Counter.start()
    print(counter.add(1))
    bump(counter)
    other := counter
    other.add(100)
    print(counter.total())
}
```

```text
1
111
```

The fields come first, then the methods. Inside a method a field is used by its bare name, with no `self`, and only a field declared `var` can be changed. A method may call another method of the same actor by its bare name.

`Counter.start()` makes an actor and returns a *handle* to it. The name of the actor is also the type of its handles: `counter` is of type `Counter`. A handle is a value. It can be assigned, passed to a function, put in a list or a structure. But a copy of a handle is a second handle to the *same* actor, as the program shows: `bump` was given `counter` as a plain argument, `other` is a copy of it, and both of them added to the one count. A handle is the one thing in Koo that refers, and the word `actor` at its declaration is the warning.

This is not a hole in the rule about values; it is where the rule leads. The count is not in the variable `counter`, to be copied with it. It belongs to the actor, and `counter.add(1)` does not reach in and change it: it *asks the actor* to, and the actor does so in its own turn.

## 9.2 Starting an Actor

The fields of an actor that have defaults begin with them. Those that do not must be given to `start`. They may be given in the order in which they are declared, but it is better to give them by name:

```koo
actor Account {
    owner: string
    var balance: int = 0
    var history: [string] = []

    fn deposit(amount: int) -> int {
        balance += amount
        note("deposit of {amount}")
        return balance
    }

    fn note(entry: string) {
        history.push("{owner}: {entry}")
    }

    fn entries() -> [string] {
        return history
    }
}

fn main() {
    ada := Account.start(owner: "Ada")
    bob := Account.start(owner: "Bob")
    ada.deposit(100)
    bob.deposit(5)
    ada.deposit(20)
    print(ada.entries())
    print(bob.entries())
}
```

```text
["Ada: deposit of 100", "Ada: deposit of 20"]
["Bob: deposit of 5"]
```

Each call of `start` makes a separate actor, with state of its own. Actors are cheap, and a program may have a great many of them: one for each account, each connection, each game in progress.

## 9.3 Asking

A call of a method through a handle, like `ada.deposit(100)`, is an *ask*. The caller waits until the actor has run the method, and gets its result. If the actor is in the middle of something else, the caller waits for that first.

A method of an actor may fail like any other function, and the errors come back to the asker in the ordinary way:

```koo
error Insufficient { balance: int, wanted: int }

actor Account {
    var balance: int = 0

    fn deposit(amount: int) -> int {
        balance += amount
        return balance
    }

    fn withdraw(amount: int) -> int ! Insufficient {
        if amount > balance {
            fail Insufficient { balance, wanted: amount }
        }
        balance -= amount
        return balance
    }
}

fn main() {
    account := Account.start()
    account.deposit(100)
    for amount in [30, 500] {
        match account.withdraw(amount) {
            left => print("withdrew {amount}, {left} left")
            ! Insufficient { balance, wanted } => print("cannot withdraw {wanted} from {balance}")
        }
    }
}
```

```text
withdrew 30, 70 left
cannot withdraw 500 from 70
```

The method `withdraw` tests the balance and then changes it. In a program where several things happen at once, a test followed by a change is the classic place for a bug: two withdrawals both find the balance sufficient, and both proceed. Here it cannot happen. The whole method is one turn of the actor, and no other message can come between the test and the change.

## 9.4 Telling

Often the caller does not need an answer, and has no reason to wait. To *tell* an actor is to leave it a message and go on. A tell is written with `send` between the handle and the method:

```koo
actor Log {
    var lines: [string] = []

    fn write(line: string) {
        lines.push(line)
        print("logged: {line}")
    }

    fn count() -> int {
        return lines.len
    }
}

fn main() {
    log := Log.start()
    log.send.write("starting")
    log.send.write("working")
    print("{log.count()} lines")
    log.send.write("finishing")
}
```

```text
logged: starting
logged: working
2 lines
logged: finishing
```

Only a method that returns nothing and cannot fail can be told, since there would be nobody to receive what it returned.

Two guarantees make tells easy to reason about. First, an actor does what it is told in the order it was told, and before it answers a later ask: that is why `log.count()` above says 2, and why its line is printed after the first two. Second, a program does not end until everything that was told has been done. The last `write` runs after `main` has returned, and the program waits for it.

## 9.5 Side by Side

A tell does not only save the caller a wait. What actors are told is run by a pool of threads, one for each processor of the machine, so actors that have been told something work at the same time.

```koo
actor Counter {
    var found: int = 0

    fn count(from: int, to: int) {
        for n in from..to {
            if is_prime(n) {
                found += 1
            }
        }
    }

    fn result() -> int {
        return found
    }
}

fn is_prime(n: int) -> bool {
    if n < 2 {
        return false
    }
    var d := 2
    while d * d <= n {
        if n % d == 0 {
            return false
        }
        d += 1
    }
    return true
}

// Count the primes below 800000, eight ranges at a time.
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

`main` tells eight actors to count, one after the other, without waiting; the eight counts proceed together, on as many processors as the machine has; and then `main` asks each for its result, which waits until that actor has finished what it was told. On a machine with several processors the program takes a fraction of the time that one actor would.

Nothing in the program mentions a thread, a lock, or a shared variable. There is no shared variable. Each `count` has an actor's state to itself, and the only thing that crosses from one actor to another is a message.

An ask, by contrast, runs on the thread of the asker, with the actor held for the duration. When nobody else wants the actor it costs a few nanoseconds, not much more than a function call, which is why it is reasonable to put even small pieces of state in actors. The README of the repository has measurements, and the same programs written in Erlang for comparison.

## 9.6 Messages

The arguments of a method, and what it returns, are *messages*: values that go from one actor's world to another's. Two things should be said about them.

The first is that a message is not copied, however large it is. When a value is sent to an actor it is *frozen*: from then on its storage is shared between the sender and the receiver, and neither can change it in place. This costs nothing in the meaning of programs, because of the rule we began with. A value was never something that two holders could change under each other. If the sender or the receiver later changes "its" value, it gets a copy of its own first, exactly as with any value that two variables hold.

```koo
actor Shelf {
    var items: [string] = []

    fn put(item: string) {
        items.push(item)
    }

    fn all() -> [string] {
        return items
    }
}

fn main() {
    shelf := Shelf.start()
    shelf.put("atlas")
    shelf.put("bible")

    var mine := shelf.all()
    mine.push("comics")
    print(mine)
    print(shelf.all())
}
```

```text
["atlas", "bible", "comics"]
["atlas", "bible"]
```

`shelf.all()` hands out the actor's own list. `main` adds to what it was given, and the shelf is none the worse.

The second is that not every type can be a message. A message is made of numbers, bytes, strings, booleans, lists, optionals, maps with string keys, and the structures, enumerations and errors of the program that are made of those; and handles, so that one actor can be told about another. A function cannot be a message, and neither, at present, can a tuple. The compiler rejects a method that asks for one:

```koo
actor Table {
    var cells: [string] = []

    fn set(at: (int, int), value: string) {
        cells.push("{at}: {value}")
    }
}

fn main() {
    table := Table.start()
    table.set((1, 2), "x")
}
```

```error
error: this type cannot be sent to an actor, or back from one
 --> main.koo:4:16
  |
4 |     fn set(at: (int, int), value: string) {
  |                ^^^^^^^^^^
  |
  = help: a message is made of numbers, bytes, strings, booleans, lists, optionals, maps with string keys, the program's structs, enums and errors made of those, and the handles of actors; a function or a tuple cannot cross from one actor to another

1 error
```

## 9.7 Actors That Know Actors

Since a handle can be a field and a message, actors can be composed. Here an `Auditor` is given the handle of a `Store` when it is started, and asks the store a question whenever it has something to record:

```koo
type Item = { name: string, qty: int }

actor Store {
    var items: [string: Item] = [:]

    fn put(item: Item) {
        items[item.name] = item
    }

    fn total() -> int {
        return items.values().map(item -> item.qty).sum()
    }
}

actor Auditor {
    store: Store
    var lines: [string] = []

    fn note(event: string) {
        lines.push("{event} ({store.total()} in store)")
    }

    fn report() -> [string] {
        return lines
    }
}

fn main() {
    store := Store.start()
    auditor := Auditor.start(store: store)

    store.put(Item { name: "bolt", qty: 4 })
    auditor.note("put bolts")
    store.put(Item { name: "beam", qty: 1 })
    auditor.note("put a beam")

    for line in auditor.report() {
        print(line)
    }
}
```

```text
put bolts (4 in store)
put a beam (5 in store)
```

There is one way for this to go wrong, and it is worth knowing before you meet it. If, in the middle of a message, actor A asks actor B, and B, to answer, asks A, then A is waiting for B and B for A, and neither will ever finish. Koo does not let such a program hang. An ask that would close a circle of waiting panics, and says what is waiting for what:

```koo
actor Ping {
    var other: Pong? = none

    fn meet(pong: Pong) {
        other = pong
    }

    fn serve() -> int {
        if pong := other {
            return pong.hit()
        }
        return 0
    }

    fn echo() -> int {
        return 1
    }
}

actor Pong {
    ping: Ping

    fn hit() -> int {
        return ping.echo()
    }
}

fn main() {
    ping := Ping.start()
    pong := Pong.start(ping: ping)
    ping.meet(pong)
    print(ping.serve())
}
```

```panic
panic: `Ping` is asked `echo` while it waits for the answer to this very message: it would wait for ever. Use `send` on one side.
```

The remedy is in the message: one of the two asks should be a tell. Actors that only tell each other can be arranged in any pattern at all.

## 9.8 When an Actor Panics

A panic in a method that was asked for is a panic of the asker: the program was waiting for an answer that will never come. But a panic in a message that was *told* has nobody waiting. It ends that message, and not the program. The actor then *starts again* with the state it was started with, and goes on to the next thing it was told.

```koo
actor Divider {
    var done: int = 0

    fn divide(a: int, b: int) {
        done += 1
        print("{a} / {b} = {a / b}")
    }

    fn count() -> int {
        return done
    }
}

fn main() {
    divider := Divider.start()
    divider.send.divide(6, 3)
    divider.send.divide(1, 0)
    divider.send.divide(8, 2)
    print("divisions since the last start: {divider.count()}")
}
```

```text
6 / 3 = 2
8 / 2 = 4
divisions since the last start: 1
```

On the standard error output, the program says what happened:

```log
panic: division by zero
  at main.koo:6
koo: the actor `Divider` panicked, and starts again
```

The second message panicked. The actor started afresh, with `done` at zero, ran the third, and so the count at the end is 1.

Starting again from a known state is the oldest remedy for a program that has got into a state nobody foresaw, and it is right for it to be the default: an actor that has panicked has, by definition, a bug, and its state is under suspicion. But an actor that panics again and again is not being cured by it. An actor that panics more than five times in a minute is left stopped.

## 9.9 A Limit on Mail

A tell does not wait for the actor, so nothing stops a program from telling an actor faster than the actor can listen. Were the messages simply to pile up, a program with a slow actor would use more and more memory until there was none. So the mail of an actor has a limit: 1,024 messages may wait for an actor, and a thread that tells it one more waits itself, until the actor has caught up. The fast sender is slowed to the pace of the slow receiver, which is called *backpressure*, and nobody has to arrange it.

The limit can be set for an actor by writing `@mailbox(n)` before its declaration. And where waiting is not what is wanted, there is `try_send`, used like `send`, which does not wait: it leaves the message if there is room and returns `true`, or returns `false` and leaves nothing.

```koo
// An actor that takes its time, and lets only four messages wait.
@mailbox(4)
actor Printer {
    var printed: int = 0

    fn page(n: int) {
        var work := 0
        for i in 0..3000000 {
            work = (work + i + n) % 1000003
        }
        printed += 1
    }

    fn count() -> int {
        return printed
    }
}

fn main() {
    printer := Printer.start()

    var accepted := 0
    for n in 0..20 {
        if printer.try_send.page(n) {
            accepted += 1
        }
    }
    print("some of the twenty were refused: {accepted < 20}")

    for n in 0..10 {
        printer.send.page(n)
    }
    print("every page that was accepted was printed: {printer.count() == accepted + 10}")
}
```

```text
some of the twenty were refused: true
every page that was accepted was printed: true
```

How many of the twenty are accepted depends on how quickly the printer gets to work, and differs from one run to the next; that some are refused, and that none of the ten sent with `send` is lost, does not.

There is one sender that cannot be made to wait: the actor itself. If an actor tells itself something while its mail is full, the only one who could make room is the one waiting for it. More generally, a thread may wait for room in the mail of an actor that cannot run until that very thread is done. Like a circle of asks, this is a wait that could never end, and it is a panic that says so:

```koo
@mailbox(2)
actor Echo {
    var ticks: int = 0

    fn flood() {
        for i in 0..10 {
            self.send.tick()
        }
    }

    fn tick() {
        ticks += 1
    }

    fn count() -> int {
        return ticks
    }
}

fn main() {
    echo := Echo.start()
    echo.send.flood()
    print(echo.count())
}
```

```text
2
```

```log
panic: `Echo` cannot be told `tick`: its mailbox is full, and what would make room in it is waiting for this message to end. Give the actor a larger `@mailbox`, or send less at a time.
koo: the actor `Echo` panicked, and starts again
```

`self`, in a method of an actor, is the actor's own handle. The first two ticks fitted in the mail; the third did not, the message `flood` panicked, the actor started again, and then did the two ticks that were waiting.

## 9.10 Stopping

An actor lives until it is stopped by `handle.stop()`, or until the end of the program. When the program ends, which is when `main` has returned and all that was told has been done, the actors that remain are stopped.

After an actor has stopped, a message that is told to it is dropped, and an ask of it is a panic.

```koo
actor Greeter {
    greeting: string

    fn greet(name: string) -> string {
        return "{greeting}, {name}!"
    }
}

fn main() {
    greeter := Greeter.start(greeting: "Hello")
    print(greeter.greet("Ada"))
    greeter.stop()
    print(greeter.greet("Bob"))
}
```

```text
Hello, Ada!
```

```panic
panic: `greet` was asked of an actor that is stopped (ActorStopped)
```

## 9.11 What Is Not There

Actors are the newest part of Koo, and the part that will grow the most. It is as well to know the limits of what is built.

A method that runs for a long time keeps the thread it is running on, and one that never returns keeps it for ever. The other actors do not suffer for it: when every thread is busy and actors have been waiting for a hundredth of a second, another thread is started for them, up to a few hundred threads. But nothing takes the thread away from the method, and nothing can make its own actor answer anyone else until it is done. An ask of that actor waits as long as the method runs.

When a told message panics, the storage that the message was using at that moment is not freed.

The specification describes more than this: actors with type parameters, ways to supervise one actor from another, and to reach an actor on another machine. None of that is built.

**Exercise 9-1.** Write an actor `Stock` that holds the quantity in stock of each of a set of items, with methods to add stock, to take stock, failing if there is not enough, and to report. Write a program that takes from it in a loop, and reports what is left.

**Exercise 9-2.** Change the prime-counting program to use sixteen counters, then one. Time the three versions.

**Exercise 9-3.** Write a program in which a first actor reads lines from the input and tells them to a second, which counts the words in each and tells the count to a third, which keeps the total. Have `main` print the total at the end. Why does the total need to be asked for, and not told?
