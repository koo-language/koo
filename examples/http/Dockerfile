# A Koo server as a container of a few megabytes with nothing in it but the program.
#
#     docker build -f examples/http/Dockerfile -t orders .
#     docker run --rm -p 8080:8080 orders
#     curl localhost:8080/orders/7
#
# Run from the root of the repository. The first stage builds the compiler, the second
# builds the program with everything linked in, and the last holds the program alone.

FROM rust:1-slim-trixie AS koo
RUN apt-get update && apt-get install -y --no-install-recommends clang libc6-dev && rm -rf /var/lib/apt/lists/*
WORKDIR /koo
COPY Cargo.toml Cargo.lock rustfmt.toml ./
COPY compiler compiler
RUN cargo build --release

FROM koo AS build
COPY examples/http/orders.koo /app/orders.koo
RUN /koo/target/release/koo build --release --static /app/orders.koo -o /app/orders

FROM scratch
COPY --from=build /app/orders /orders
EXPOSE 8080
ENTRYPOINT ["/orders"]
