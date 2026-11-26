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

They were written to be run from a scratch directory, and say so in their paths:
`KOO_ROOT` stands for the repository, and `PASSWORD` for the password of a throwaway
test database. A script names the files it needs beside it; the commit messages of
the fixes say which script showed which defect.
