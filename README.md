# Supernova DB

Supernova DB is a custom database project written in Rust. It is designed to be beginner-friendly, secure enough for experimentation, and powerful enough to serve as a platform for a custom protocol-driven storage engine.

## Highlights

- custom protocol with `SDB2` header
- snapshot persistence and WAL durability
- transaction support with commit/rollback
- schema-aware table metadata
- transactional query filtering
- TCP server mode with optional token authentication

## Project goals

Supernova DB focuses on making the internals understandable while still containing advanced data-layer ideas such as:

- durable writes
- explicit storage snapshots
- transaction boundaries
- row-level queries
- table metadata and index-friendly records

## Quick start

```bash
cargo build
cargo run -- create-table users
cargo run -- set users alice '{"name":"Alice","role":"admin"}'
cargo run -- get users alice
cargo run -- list users
cargo run -- query users "role=admin"
cargo run -- begin
cargo run -- set users bob '{"name":"Bob","role":"user"}'
cargo run -- commit
```

## Server mode

```bash
cargo run -- serve --host 127.0.0.1 --port 8080 --token demo-token
```

## Example request payload

```json
{
  "op": "query",
  "table": "users",
  "key": "",
  "value": null,
  "query": "role=admin",
  "token": "demo-token"
}
```

## Advanced features now included

- transactional lifecycle (`begin`, `commit`, `rollback`)
- table metadata with tracked columns
- JSON row query filtering
- WAL-backed persistence model
- optional server-side auth token

## Build and test

```bash
cargo test
cargo run -- --help
```
