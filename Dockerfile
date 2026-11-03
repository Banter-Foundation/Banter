# Banter server. The desktop app is not in this image.
# Build needs the workspace manifest because banter-server is a workspace member.

FROM rust:1.99-bookworm AS build
WORKDIR /src
COPY Cargo.toml Cargo.lock ./
COPY crates/banter/Cargo.toml crates/banter/Cargo.toml
# The desktop crate is a workspace member. Cargo only needs its manifest to
# resolve the workspace, so the image does not copy or compile the app.
RUN mkdir -p crates/banter/src && printf 'pub fn desktop_not_in_this_image() {}\n' > crates/banter/src/lib.rs
COPY crates/banter-server crates/banter-server
RUN cargo build -p banter-server --release

FROM debian:bookworm-slim
RUN useradd --system --uid 10001 --home /data --shell /usr/sbin/nologin banter \
    && mkdir -p /data \
    && chown banter:banter /data
COPY --from=build /src/target/release/banter-server /usr/local/bin/banter-server
USER banter
ENV BANTER_DATA=/data \
    BANTER_BIND=0.0.0.0 \
    BANTER_PORT=3970
VOLUME ["/data"]
EXPOSE 3970
ENTRYPOINT ["/usr/local/bin/banter-server"]
