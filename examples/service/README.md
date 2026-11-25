# A service

`service.koo` is a small orders service, and a pattern for a service of your own: about a hundred lines, of which the part that is not about orders is ten. It is written the way the [twelve factors](https://12factor.net) ask, and most of what they ask is what Koo does when nothing else is said.

```sh
export DATABASE_URL=postgres://postgres:secret@localhost:5432/postgres
koo run examples/service/service.koo

curl -d '{"customer": "ada", "sku": "hammer", "qty": 2}' localhost:8080/orders
curl localhost:8080/orders/1
curl localhost:8080/healthz
```

With a database of its own, in containers:

```sh
docker compose -f examples/service/compose.yaml up --build
```

The tests run this service against a real database and check what it answers (`the_example_service_serves`).

## The twelve factors

| | Factor | Here |
| --- | --- | --- |
| I | One codebase | A directory of `.koo` files is a program. |
| II | Dependencies declared | The program is one executable with the support library inside it. The system gives it only OpenSSL, for TLS. |
| III | Config in the environment | `Settings.from_env()` reads a struct from the environment, each field from the variable of its name, and fails at once, naming every variable that is missing. |
| IV | Backing services by address | `postgres.connect(settings.database_url)`: the database is an address, `sslmode` and all. Another server is `http.get(url)`. |
| V | Build, release, run | `koo build --release` makes the executable; the `Dockerfile` makes an image of it; the environment makes a release of the image. |
| VI | Stateless processes | A handler keeps nothing between requests; what must be kept is in the database. |
| VII | Port binding | `app.serve()` listens where `PORT` says. |
| VIII | Concurrency by processes | `WEB_CONCURRENCY` workers, one for each processor unless it is said; a worker answers many requests at a time, since a handler that waits for the database does not hold it up. |
| IX | Disposability | The service starts in a few milliseconds. Told to stop, with `SIGTERM`, it answers the requests it has in hand and ends with status 0. A handler that panics fails its own request and nothing else. |
| X | Parity of development and production | The same executable, and the same PostgreSQL, on a laptop and in a container. |
| XI | Logs as a stream | `log.info("order {order.id} placed")` writes a line to the standard error output, and with `KOO_LOG=json` every line the service writes is a JSON object, with `order.id` as a field. Nothing is written to a file. |
| XII | Admin tasks as one-off processes | A task is another program run against the same database: `koo run migrate.koo`. |

## What to set

| Variable | |
| --- | --- |
| `DATABASE_URL` | the database: `postgres://user:password@host:5432/name?sslmode=verify-full` |
| `PORT` | where to listen; 8080 if it is not set |
| `WEB_CONCURRENCY` | how many workers; one for each processor if it is not set |
| `POOL` | the most connections to the database that one worker holds; 10 if it is not set |
| `LOG_REQUESTS` | `false` to leave the line for each request out of the log |
| `KOO_LOG` | `json` for a log that a program reads |
| `KOO_LOG_LEVEL` | `debug`, `info`, `warn`, `error` or `off` |
