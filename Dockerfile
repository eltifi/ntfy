FROM rust:alpine as builder

RUN apk add --no-cache musl-dev sqlite-dev gcc openssl-dev

WORKDIR /app
COPY . .

# Build release binary
RUN DATABASE_URL=sqlite:test.db cargo build --release

FROM alpine

LABEL org.opencontainers.image.source="https://github.com/eltifi/ntfy"
LABEL org.opencontainers.image.title="ntfy"

RUN apk add --no-cache tzdata ca-certificates

COPY --from=builder /app/target/release/ntfy /usr/bin/ntfy

EXPOSE 80/tcp
ENTRYPOINT ["ntfy"]
