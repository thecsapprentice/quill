# syntax=docker/dockerfile:1

# Builds the Quill server. Use with compose.yaml for local testing:
#   docker compose up --build

FROM rust:1.97.0-slim-trixie AS build
# Use the image's toolchain as-is; rust-toolchain.toml would otherwise make
# rustup download rustfmt and clippy, which a release build doesn't need.
ENV RUSTUP_TOOLCHAIN=1.97.0
WORKDIR /src
COPY . .
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/src/target \
    cargo build --release --locked -p quill-server \
    && cp target/release/quill-server /usr/local/bin/quill-server

FROM debian:trixie-slim
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates \
    && rm -rf /var/lib/apt/lists/* \
    && useradd --system --uid 10001 --no-create-home quill
COPY --from=build /usr/local/bin/quill-server /usr/local/bin/quill-server
USER quill
ENV QUILL_BIND=0.0.0.0:8080
EXPOSE 8080
# The server shuts down gracefully on SIGINT.
STOPSIGNAL SIGINT
ENTRYPOINT ["quill-server"]
CMD ["serve"]
