# Supernova DB

Supernova DB is an advanced custom database project written in Rust. It is designed to be beginner-friendly, secure enough for experimentation, and powerful enough to serve as a foundation for a custom protocol-driven storage engine.

## Highlights

- Custom binary protocol with a protocol magic header
- Snapshot persistence for fast recovery
- Write-ahead log (WAL) journaling for durability
- Multi-table data model
- CLI for local operations
- TCP server mode for custom client connections
- JSON-first request/response schema for easy learning

## Project goals

Supernova DB is designed with a clear philosophy:

- keep the protocol custom and understandable
- keep persistence transparent and explicit
- make the code easy to read and teach
- build a solid base for custom database features without a huge framework

## Quick start

```bash
cargo build
cargo run -- tables
cargo run -- create-table users
cargo run -- set users alice '{"name":"Alice","role":"admin"}'
cargo run -- get users alice
cargo run -- list users
```

## Server mode

```bash
cargo run -- serve --host 127.0.0.1 --port 8080 --token demo-token
```

Clients can send framed JSON payloads with the `SDB2` protocol header. A request example looks like:

```json
{
  "op": "set",
  "table": "users",
  "key": "alice",
  "value": {"name":"Alice","role":"admin"},
  "token": "demo-token"
}
```

## Structure

- `src/lib.rs` — library exports
- `src/storage.rs` — advanced persistent storage layer with WAL + snapshots
- `src/protocol.rs` — custom client/server protocol
- `src/main.rs` — CLI and TCP server handling
- `tests/basic.rs` — integration tests

## Current capabilities

- create and delete tables
- set, get, and delete key/value entries
- list rows in a table
- list all tables
- durable writes using WAL
- data snapshot recovery
- custom network protocol over TCP

## Roadmap

The next milestone can include:

- indexed lookups
- transactions
- replication hooks
- binary typed values
- query parser
- authentication + roles
- scavenger / compaction for WAL
- SQL-like subset support

## Build

```bash
cargo test
cargo run -- --help
```
