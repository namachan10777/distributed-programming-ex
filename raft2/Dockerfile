FROM rust:latest as base
WORKDIR /work
RUN cargo install cargo-chef

FROM base as planner
COPY src src
COPY Cargo.* .
RUN cargo chef prepare

FROM base as builder
COPY --from=planner /work/recipe.json .
RUN cargo chef cook --release
COPY src src
COPY Cargo.* .

FROM debian:bullseye-slim
COPY --from=builder /work/target/release/raft /usr/local/bin

ENTRYPOINT [ "/usr/local/bin/raft" ]
CMD [ "--servers", "/etc/raft/servers.json" ]