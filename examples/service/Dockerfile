# The orders service as a container.
#
#     docker build -f examples/service/Dockerfile -t orders .
#     docker run --rm -p 8080:8080 -e DATABASE_URL=postgres://... orders
#
# Run from the root of the repository; `compose.yaml` beside this file runs it with a
# database. The first stage builds the compiler, the second builds the service, and the
# last holds the service and the two things it needs of the system: OpenSSL, to reach a
# database or another server over TLS, and the list of authorities to trust.

FROM rust:1-slim-trixie AS koo
RUN apt-get update && apt-get install -y --no-install-recommends clang libc6-dev && rm -rf /var/lib/apt/lists/*
WORKDIR /koo
COPY Cargo.toml Cargo.lock rustfmt.toml ./
COPY compiler compiler
RUN cargo build --release

FROM koo AS build
COPY examples/service/service.koo /app/service.koo
RUN /koo/target/release/koo build --release /app/service.koo -o /app/service

FROM debian:trixie-slim
RUN apt-get update && apt-get install -y --no-install-recommends libssl3t64 ca-certificates && rm -rf /var/lib/apt/lists/*
COPY --from=build /app/service /service
# The service runs as nobody in particular, and is told what it needs by its environment.
USER 65534
ENV PORT=8080 KOO_LOG=json
EXPOSE 8080
ENTRYPOINT ["/service"]
