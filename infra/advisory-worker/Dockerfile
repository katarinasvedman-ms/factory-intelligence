FROM rust:bookworm AS build
WORKDIR /src
COPY Cargo.toml Cargo.lock ./
COPY src ./src
COPY config ./config
COPY app ./app
COPY data ./data
RUN cargo build --release --bin factory-advisory-worker

FROM debian:bookworm-slim
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates \
    && rm -rf /var/lib/apt/lists/* \
    && useradd --create-home --uid 10001 worker
WORKDIR /app
COPY --from=build /src/target/release/factory-advisory-worker /app/factory-advisory-worker
COPY --from=build /src/config /app/config
COPY --from=build /src/data /app/data
RUN mkdir -p /app/state && chown -R worker:worker /app
USER worker
ENV DEMO_CONFIG=config/demo.aks-agentic.yaml
ENTRYPOINT ["/app/factory-advisory-worker"]
