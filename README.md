# Supernova DB

Supernova DB is a custom Rust database designed to be understandable, resilient, and extensible. This stage adds table schemas, richer row queries, sorting, better recovery, and a stronger protocol foundation.

## What is new in v0.5.0?

- schema-aware table creation with required column metadata
- secondary indexes and faster equality lookups
- query operators (`=`, `!=`, `>`, `<`, `>=`, `<=`, `~`)
- row sorting and limit support
- checkpoint + WAL recovery improvements
- richer TCP protocol with table metadata and query payloads

## Quick start

```bash
cargo build
cargo run -- create-table users --columns name:string,role:string,age:number
cargo run -- set users alice '{"name":"Alice","role":"admin","age":30}'
cargo run -- set users bob '{"name":"Bob","role":"user","age":25}'
cargo run -- query users role=admin
cargo run -- list users --sort age --limit 10
cargo run -- checkpoint
```

## Query examples

```bash
cargo run -- query users age>=25
cargo run -- query users name~ali
cargo run -- query users role=admin
```

## Server mode

```bash
cargo run -- serve --host 127.0.0.1 --port 8080 --token demo-token
```

## Protocol

Each request is a framed message using the `SDB2` magic header.

Example payload:

```json
{"op":"query","table":"users","field":"role","query":"role=admin","sort_by":"age","limit":10,"token":"demo-token"}
```

## Design goals

- custom protocol, not just a wrapper around a library
- data persistence with clear recovery semantics
- schema validation for safer row writes
- beginner-friendly structure that is easy to extend

## Build and tests

```bash
cargo test
cargo run -- --help
```
