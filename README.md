# Supernova DB

Supernova DB is an experimental, beginner-friendly **multi-model Rust database**. One collection can hold JSON documents and expose them through several access patterns:

- **Document / user data:** store and retrieve JSON by collection and ID.
- **Keyword search:** build field-specific inverted indexes and rank documents by matching terms.
- **Vector search:** store numeric embedding arrays and find nearest vectors by cosine similarity (exact scan).
- **SQL-like queries:** run a small, deliberately limited `SELECT` syntax over document fields.
- **Custom client protocol:** length-bounded `SDB3` JSON frames over TCP.

This is a functional learning and prototyping foundation, **not yet a replacement for SQLite, PostgreSQL, or mature vector/search databases**. It has not been independently benchmarked or audited. Current storage is a single-process JSON snapshot; vector search is exact scan; TCP token authentication is plaintext and does not provide TLS. Use backups and do not expose the server to untrusted networks.

## Quick start

```sh
cargo test
cargo run -- create-collection articles
cargo run -- put articles doc-1 '{"title":"Rust database","body":"Fast searchable storage","embedding":[0.1,0.2,0.3]}'
cargo run -- create-text-index articles body
cargo run -- search-text articles body searchable
cargo run -- create-vector-index articles embedding 3
cargo run -- vector-search articles embedding '[0.1,0.2,0.25]' --limit 5
cargo run -- sql 'SELECT * FROM articles WHERE title = "Rust database" LIMIT 10'
```

The database directory defaults to `supernova_data`; choose another with global `--db` before the command (`cargo run -- --db ./data ...`). Documents are JSON and IDs are unique inside each collection.

## SQL subset

Supported form:

```sql
SELECT * FROM collection [WHERE field (= | != | > | < | >= | <= | ~) literal] [LIMIT n]
```

`~` performs case-insensitive substring matching on strings. Literals may be JSON values or unquoted strings. This is intentionally a small query language, not full SQL: joins, DDL, transactions, aggregation, and SQL wire compatibility are not implemented.

## Indexing

Create a keyword index explicitly with `create-text-index <collection> <field>`. Search tokenizes Unicode alphanumeric terms, lowercases them, and ranks by term frequency. Vector fields are declared with `create-vector-index <collection> <field> <dimensions>`; writes validate the dimension and searches rank by cosine similarity. These indexes are rebuilt from persisted documents when the database opens; vector similarity currently scans all documents.

## Network protocol

Run locally with `cargo run -- serve --host 127.0.0.1 --port 8080`. Each connection handles one request. A frame is `SDB3` (4 bytes), a big-endian unsigned 32-bit JSON payload length, then JSON payload (maximum 1 MiB). Optionally set `--token`; it is a shared plaintext token, **not encryption**. Bind only to a trusted interface or place behind a properly configured TLS proxy.

## Roadmap

Before production use, Supernova DB needs extensive correctness and crash testing, benchmarks, file locking, robust WAL/checkpoint design, TLS and stronger auth, bounded worker/backpressure handling, migrations, and scalable on-disk indexes. The current implementation keeps those limitations explicit while providing a clear base to learn and extend.
