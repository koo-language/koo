# A server under load, and what found the defects

`check.py` is run by the tests on every change. It builds `check.koo`, a server whose
handlers ask and tell 256 actors, and makes a series of checks against it, each of
which passes or fails: hundreds of asks at once answered exactly once each, long
messages whole, workers killed outright, clients and local peers that say half of
something, a table of connections with no room left, and afterwards nothing spinning,
nothing left open, and a prompt end when the server is told to stop.

    python3 tests/stress/check.py target/release/koo
    python3 tests/stress/check.py target/release/koo --target linux-x86-64

It takes about two minutes, and needs Python 3 and a limit on open files of 8192 or
more (`ulimit -n`).

`races.sh` looks for races inside the support library, on Linux. A program built with
`--sanitize=thread` is checked by ThreadSanitizer, but the library it is linked with is
not, so the sanitizer sees nothing of the tables the library keeps. `races.sh` builds the
library again so that it does (that takes a nightly Rust compiler), has `koo` link it by
naming it in `KOO_RUNTIME`, and runs every test program that has actors or threads.

    sh tests/stress/races.sh target/release/koo

In a container the sanitizer must be let turn off the shuffling of addresses, which
Docker forbids unless it is told `--security-opt seccomp=unconfined`: without that,
every program ends at once with `CHECK failed: tsan_platform_linux.cpp`, and the
script says that all of them have races.

`fuzz.py` throws what is nearly right at everything the support library reads from
outside: requests of HTTP, bodies of JSON, and the answers of another server of HTTP,
of a PostgreSQL and of a Redis, which three are peers of its own that spoil a right
answer in one way or another. The server it aims at (`fuzz/main.koo`) must go on
whatever it is sent, keep its workers, write no panic of its own, and stop with status
0. A run prints its seed, and the same seed throws the same again. `fuzz.sh` does it
with the library built so that AddressSanitizer sees inside it, on Linux with a nightly
Rust compiler, as `races.sh` does for races. A server built so has no stacks to switch
between, and each of its workers answers one request at a time: so there the clients
of the three peers' readers wait for each answer before they ask again (`--in-turn`),
a handler has three tenths of a second and not five seconds, and fewer things are thrown in the same time. A
read beyond what was allocated, planted in the reader of JSON, is reported by it at
once, which is the check that it sees what it is there to see.

    python3 tests/stress/fuzz.py target/release/koo --seconds 15
    sh tests/stress/fuzz.sh target/release/koo

`limits.sh` runs the same server in a container that is allowed forty processes, where
a thread counts as one, and checks that the process which watches the workers leaves
room under the limit to start a worker again, and does when one is killed: both where
the server sees its own control group as the top one, and where it sees the groups of
its whole machine and its own is further in. It needs Docker, and an image with a Rust
toolchain in it.

    sh tests/stress/limits.sh

## What found the defects

These are the programs and scripts with which several independent searches for
defects found what they found: hangs of a server under load, lookups that did not
keep to their time, crashes from hostile peers. They are kept so that the same
searches can be made again. The test suite does not run these: most need a Linux
container with limits set on it, a name server that misbehaves on purpose, or
minutes of load.

Each directory is one search, as it left its files:

| Directory | What it tried |
| --- | --- |
| `actors` | Handlers of a server asking actors, on macOS: hundreds at once, long messages, workers killed, a stack that runs out |
| `actors-linux` | The same on Linux, in containers with limits on processors, memory, files and processes (`m.koo` is the one server; `run.sh` runs a script in a container) |
| `cross` | Programs built with `--target`, and name servers that are slow, silent or cut their answers short |
| `client-database` | The HTTP client and the PostgreSQL driver against peers that stall, lie or go silent |
| `lookup-client-database` | A name server that answers wrongly on purpose (`dns.py`), a database that does (`fakepg.py`), redirections to odd places |
| `review` | Small programs that confirm what a reading of the code suspected |
| `review3` | The same for the last reading: connections a worker keeps, a full table, a busy supervising process, told messages with no thread to run them |
| `hunt3` | The last search, on Linux: one worker taking every connection, answers cut short by a stop, limits on processes and files, a long answer to a busy worker, a flood at the resolver, local peers that forge messages (`two.sh` runs a server alone in one container and a script beside it) |
| `review4` | A reading of the fixes for all of that: long answers to workers that are busy, a told method that runs out of stack |

They were written to be run from a scratch directory, and say so in their paths:
`KOO_ROOT` stands for the repository, and `PASSWORD` for the password of a throwaway
test database. A script names the files it needs beside it; the commit messages of
the fixes say which script showed which defect.
