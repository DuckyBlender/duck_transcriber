# Stage 1: Chef - prepare recipe
FROM rust:1-slim AS chef
RUN cargo install cargo-chef && cargo install sqlx-cli --no-default-features --features sqlite
WORKDIR /app

# Stage 2: Planner - create recipe.json
FROM chef AS planner
COPY . .
RUN cargo chef prepare --recipe-path recipe.json

# Stage 3: Builder - build dependencies then app
FROM chef AS builder
COPY --from=planner /app/recipe.json recipe.json
# Build dependencies (cached layer)
RUN cargo chef cook --release --recipe-path recipe.json
# Build application
COPY . .
# Use SQLx offline mode for compile-time query checking
ENV SQLX_OFFLINE=true
RUN cargo build --release

# Stage 4: Runtime - minimal final image
# Use rust:1-slim base to ensure GLIBC compatibility
FROM rust:1-slim AS runtime
RUN apt-get update && apt-get install -y ca-certificates && rm -rf /var/lib/apt/lists/*
WORKDIR /app

# Create data directory for SQLite database
RUN mkdir -p /app/data

COPY --from=builder /app/target/release/duck_transcriber .

# Default database location
ENV DATABASE_URL=sqlite:/app/data/duck_transcriber.db

# Run with: docker run --env-file .env duck_transcriber
CMD ["./duck_transcriber"]
