# Banter server image (banter-server only). The desktop client is not included.
# Build needs the workspace Cargo.toml because banter-server is a workspace member.
# Podman and Docker both work. Prefer Podman on Black-Birb when Docker Desktop is down.
#
#   podman build -t banter-server:local .
#   docker  build -t banter-server:local .

FROM rust:1.99-bookworm AS build
WORKDIR /src

COPY Cargo.toml Cargo.lock ./
COPY crates/banter/Cargo.toml crates/banter/Cargo.toml
# Workspace member stub: Cargo resolves the desktop crate without compiling GLFW/ImGui.
RUN mkdir -p crates/banter/src \
    && printf 'pub fn desktop_not_in_this_image() {}\n' > crates/banter/src/lib.rs

COPY crates/banter-server crates/banter-server
RUN cargo build -p banter-server --release

FROM debian:bookworm-slim
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates \
    && rm -rf /var/lib/apt/lists/* \
    && useradd --system --uid 10001 --home /data --shell /usr/sbin/nologin banter \
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
