# Supernova DB

Supernova DB is a custom Rust database project built around a simple durable storage model:

- persistent tables with row storage
- WAL-backed mutation journal
- snapshot checkpoints for fast recovery
- schema metadata and validation
- rich query language with operators like `=`, `!=`, `>`, `<`, `>=`, `<=`, and `~`
- backup and restore support
- custom TCP protocol for clients

## Quick start

```bash
cargo build
cargo run -- create-table users --columns name,role,age
cargo run -- set users alice '{"name":"Alice","role":"admin","age":30}'
cargo run -- set users bob '{"name":"Bob","role":"user","age":27}'
cargo run -- query users role=admin
cargo run -- query users age>=25 --sort-by age --limit 10
cargo run -- describe users
cargo run -- checkpoint
cargo run -- backup ./snapshots/supernova-backup
```

## Query examples

```bash
cargo run -- query users role=admin
cargo run -- query users age>=25
cargo run -- query users role!=user
cargo run -- query users name~ali
```

## Restore

```bash
cargo run -- restore ./snapshots/supernova-backup
```

## Server mode

```bash
cargo run -- serve --host 127.0.0.1 --port 8080 --token demo-token
```

## Design notes

This is still an experimental database system, but it is intentionally structured to be:

- easy to read and teach
- durable with WAL + snapshots
- extendable with custom protocol logic
- useful as a foundation for a real custom DB engine

## Roadmap

Next major milestones can include:

- join support
- transaction isolation levels
- index compaction
- SQL-like parser
- replication/follower mode
- TLS-secured network transport
