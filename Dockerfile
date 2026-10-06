FROM rust:1.98-bookworm AS build

WORKDIR /build
COPY Cargo.toml Cargo.lock* ./
COPY src ./src
COPY app/ui/static ./app/ui/static
COPY config ./config
RUN cargo build --release --bin factory-intelligence

FROM debian:bookworm-slim

RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates \
    && rm -rf /var/lib/apt/lists/*
WORKDIR /app
COPY --from=build /build/target/release/factory-intelligence /usr/local/bin/factory-intelligence
COPY app/ui/static ./app/ui/static
COPY config ./config
COPY data/synthetic_factory_incidents.jsonl ./data/synthetic_factory_incidents.jsonl
COPY data/eval_dataset.jsonl ./data/eval_dataset.jsonl

ENV BIND_ADDRESS=0.0.0.0:8000
EXPOSE 8000
CMD ["factory-intelligence"]
