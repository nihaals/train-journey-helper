FROM rust:alpine AS builder
RUN apk add --no-cache musl-dev ca-certificates
WORKDIR /src
COPY Cargo.toml Cargo.lock ./
COPY ./src/ ./src/
RUN cargo build --release

FROM scratch
COPY --from=builder /etc/ssl/certs/ca-certificates.crt /etc/ssl/certs/
COPY --from=builder /src/target/release/train-journey-helper /app
ENTRYPOINT ["/app"]
