# Supernova DB

Supernova DB is a custom Rust database project built to be fast, secure, beginner-friendly, and easy to extend.

It includes:
- a custom binary protocol
- persistent key-value storage
- a simple server mode
- a CLI for quick developer use
- a clean Rust codebase suitable for learning and experimentation

## What makes it special

Supernova DB is designed around a simple philosophy:
- custom protocol, not just a wrapper around a library
- custom storage format, not just a black-box ODM
- easy to read, easy to teach, easy to extend
- suitable for learning Rust and database design

## Quick start

```bash
cargo build
cargo run -- set user alice
cargo run -- get user
cargo run -- list
```

## Server mode

```bash
cargo run -- serve --host 127.0.0.1 --port 8080
```

Then connect using the custom protocol with any client that sends JSON frames using the `SDB1` magic header.

## Structure

- `src/lib.rs` — library exports
- `src/storage.rs` — persistent storage engine
- `src/protocol.rs` — custom protocol encoding and decoding
- `src/main.rs` — CLI and server entry point

## Example payloads

Set a value:

```json
{"op":"set","key":"name","value":"supernova"}
```

Get a value:

```json
{"op":"get","key":"name"}
```
