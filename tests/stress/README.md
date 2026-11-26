# What found the defects

These are the programs and scripts with which several independent searches for
defects found what they found: hangs of a server under load, lookups that did not
keep to their time, crashes from hostile peers. They are kept so that the same
searches can be made again. The test suite does not run them: most need a Linux
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

They were written to be run from a scratch directory, and say so in their paths:
`KOO_ROOT` stands for the repository, and `PASSWORD` for the password of a throwaway
test database. A script names the files it needs beside it; the commit messages of
the fixes say which script showed which defect.
