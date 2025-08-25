FROM --platform=$BUILDPLATFORM rust:1.89-slim-bookworm AS chef
RUN rustup target add x86_64-unknown-linux-gnu
RUN cargo install cargo-chef --locked
RUN apt-get update && \
    apt-get install -y gcc-x86-64-linux-gnu && \
    rm -rf /var/lib/apt/lists/*
# set correct linker
ENV RUSTFLAGS='-C linker=x86_64-linux-gnu-gcc'
WORKDIR /usr/src/swissrpg-app-test

FROM chef AS planner
COPY app ./app
COPY command_macro ./command_macro
COPY lib ./lib
COPY ui ./ui
COPY Cargo.lock Cargo.toml ./
RUN cargo chef prepare --recipe-path recipe.json

FROM chef AS builder
COPY --from=planner /usr/src/swissrpg-app-test/recipe.json recipe.json
# COPY .cargo ./.cargo
# Build dependencies - this is the caching Docker layer!
RUN cargo chef cook --features "bottest" --release --target x86_64-unknown-linux-gnu --recipe-path recipe.json
# Build application
COPY app ./app
COPY command_macro ./command_macro
COPY lib ./lib
COPY ui ./ui
COPY .sqlx ./.sqlx
COPY .env Cargo.lock Cargo.toml ./
# The next step will fail if Git LFS backed files haven't been downloaded
RUN ! sed -n '/^version/p;q' ui/src/web/html/static/SwissRPG-logo-128.png | grep git-lfs
RUN cargo build --features "bottest" --release --target x86_64-unknown-linux-gnu --bin swissrpg-app

FROM debian:bookworm-slim AS runtime
WORKDIR /usr/src/swissrpg-app-test
RUN apt update && apt install -y ca-certificates && rm -rf /var/lib/apt/lists/*
COPY --from=builder /usr/src/swissrpg-app-test/target/x86_64-unknown-linux-gnu/release/swissrpg-app /usr/local/bin/swissrpg-app-test
RUN chmod a=rx /usr/local/bin/swissrpg-app-test
COPY ui/src/web/html/static /usr/local/share/swissrpg-app-test/www
RUN find /usr/local/share/swissrpg-app-test/www -type d -exec chmod a=rx {} \;
RUN find /usr/local/share/swissrpg-app-test/www -type f -exec chmod a=r {} \;
EXPOSE 3001
CMD ["/usr/local/bin/swissrpg-app-test"]
