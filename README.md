# Supernova DB

Supernova DB is a beginner-friendly Rust database starter demonstrating a **multi-model API** over JSON documents:

- collections of JSON documents keyed by ID
- keyword search with a rebuildable inverted index
- vector similarity search using exact cosine similarity
- a small, deliberately limited `SELECT` query subset
- an `SDB3` length-prefixed custom TCP protocol
- atomic snapshot replacement and in-process transactions

This is an educational starter—not a replacement for SQLite, PostgreSQL, or mature search/vector databases. It stores a JSON snapshot and is intended for small, single-process experiments. The TCP token is plaintext; bind to loopback or use a trusted network. It has not been independently benchmarked or audited.

## Run it

```sh
cargo test
cargo run --example quickstart
cargo run -- create-collection articles
cargo run -- put articles item-1 '{"title":"Rust DB","body":"A small searchable database","embedding":[0.1,0.2,0.3]}'
cargo run -- create-text-index articles body
cargo run -- search-text articles body searchable
cargo run -- create-vector-index articles embedding 3
cargo run -- vector-search articles embedding '[0.1,0.2,0.25]' --limit 5
cargo run -- sql 'SELECT * FROM articles WHERE title = "Rust DB" LIMIT 10'
```

Data is stored in `supernova_data` by default. Choose a directory with `--db` before the command: `cargo run -- --db ./my-data collections`.

## SQL subset

Supported syntax is intentionally narrow:

```sql
SELECT * FROM collection [WHERE field = JSON_VALUE] [LIMIT N]
```

This supports a single equality predicate and a non-negative limit, not full SQL. It does not support joins, SQL DDL, aggregations, prepared statements, or a SQL wire protocol.

## Network protocol

Start the local server with `cargo run -- serve --host 127.0.0.1 --port 8080`. Each connection carries one request. Frames contain four bytes `SDB3`, a four-byte unsigned big-endian payload length, then JSON payload (maximum 1 MiB). An optional `--token` is a shared plaintext token, not encryption.

The library API and `examples/quickstart.rs` are the easiest places to learn and extend the implementation. For production use, it needs independent correctness and crash testing, benchmarks, file locking, a write-ahead log, authenticated encrypted transport, and scalable on-disk indexes.
