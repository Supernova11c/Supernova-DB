# Supernova DB

Supernova DB is a learning-focused Rust database with its own storage format and `SDB2` TCP framing. This release adds atomic WAL batches, crash recovery, secondary equality indexes, and a usable CLI/server implementation.

> This is an experimental embedded database, not yet a production replacement for SQLite, PostgreSQL, or other mature systems. The optional token is sent in plaintext: only expose the server on trusted networks or behind TLS.

## Quick start

```sh
cargo test
cargo run -- create-table users
cargo run -- set users alice '{"name":"Alice","role":"admin"}'
cargo run -- create-index users role
cargo run -- query users role admin
cargo run -- get users alice
cargo run -- checkpoint
```

Data lives in `./supernova_data` by default. Change it with the global `--db <directory>` option, which must appear before the command, e.g. `cargo run -- --db ./data set users alice '"Alice"'`.

## Commands

- `create-table <table>` / `delete-table <table>`
- `set <table> <key> <json-or-string>` / `get <table> <key>` / `delete <table> <key>`
- `list <table>` / `query <table> <field> <json-or-string>`
- `create-index <table> <field>` / `indexes <table>`
- `tables` / `checkpoint`
- `serve --host 127.0.0.1 --port 8080 [--token <secret>]`

Query is equality matching. For example, `query users role admin` matches rows whose JSON field `role` equals the string `admin`. Numeric and boolean values can be queried by supplying valid JSON literals.

## Storage and recovery

Mutations are appended as a single JSON record to `wal.log` and synced before becoming visible. A committed transaction is represented by one WAL record containing its complete mutation batch, so replay applies all its operations together. `checkpoint` writes a temporary snapshot, renames it into place, then truncates the WAL. Indexes are rebuilt from row data on open; they are derived data, not a second source of truth.

## Custom network protocol

Each TCP request and response is one frame:

```
4 bytes  magic: SDB2
4 bytes  payload length, unsigned big-endian
N bytes  UTF-8 JSON payload (maximum 4 MiB)
```

Example request:

```json
{"op":"set","table":"users","key":"alice","value":{"name":"Alice"},"token":null}
```

Supported operations: `ping`, `tables`, `create_table`, `delete_table`, `get`, `set`, `delete`, `list`, `query`, `create_index`, `indexes`, and `checkpoint`. Each connection handles one request. The server binds to loopback by default; use an explicit host to expose it. Configure `--token` to require a shared token, but remember this protocol does not encrypt traffic.
