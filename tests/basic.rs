use clap::{Parser, Subcommand};
use serde_json::Value;
use supernova_db::{protocol, SupernovaDb};
use std::error::Error;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::path::PathBuf;

#[derive(Parser, Debug)]
#[command(name = "supernova-db", about = "Advanced custom Rust database with a custom protocol")]
struct Cli {
    #[arg(long, default_value = "supernova_data")]
    db: PathBuf,

    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand, Debug)]
enum Commands {
    Set {
        table: String,
        key: String,
        value: String,
    },
    Get {
        table: String,
        key: String,
    },
    Delete {
        table: String,
        key: String,
    },
    List {
        table: String,
    },
    Tables,
    CreateTable {
        table: String,
    },
    DeleteTable {
        table: String,
    },
    Serve {
        #[arg(long, default_value = "127.0.0.1")]
        host: String,
        #[arg(long, default_value_t = 8080)]
        port: u16,
        #[arg(long)]
        token: Option<String>,
    },
}

fn main() -> Result<(), Box<dyn Error>> {
    let cli = Cli::parse();

    match cli.command {
        Commands::Set { table, key, value } => {
            let mut db = SupernovaDb::open(cli.db)?;
            let parsed = parse_value(&value);
            db.set(&table, key, parsed)?;
            println!("OK");
        }
        Commands::Get { table, key } => {
            let db = SupernovaDb::open(cli.db)?;
            match db.get(&table, &key) {
                Some(value) => println!("{value}"),
                None => println!("<not found>"),
            }
        }
        Commands::Delete { table, key } => {
            let mut db = SupernovaDb::open(cli.db)?;
            match db.delete(&table, &key)? {
                Some(value) => println!("deleted: {value}"),
                None => println!("<not found>"),
            }
        }
        Commands::List { table } => {
            let db = SupernovaDb::open(cli.db)?;
            let rows = db.list(&table);
            if rows.is_empty() {
                println!("<empty>");
            } else {
                for (key, value) in rows {
                    println!("{key}={value}");
                }
            }
        }
        Commands::Tables => {
            let db = SupernovaDb::open(cli.db)?;
            for name in db.table_names() {
                println!("{name}");
            }
        }
        Commands::CreateTable { table } => {
            let mut db = SupernovaDb::open(cli.db)?;
            db.create_table(&table)?;
            println!("OK");
        }
        Commands::DeleteTable { table } => {
            let mut db = SupernovaDb::open(cli.db)?;
            match db.delete_table(&table)? {
                Some(_) => println!("OK"),
                None => println!("<not found>"),
            }
        }
        Commands::Serve { host, port, token } => {
            serve(&host, port, cli.db, token)?;
        }
    }

    Ok(())
}

fn parse_value(raw: &str) -> Value {
    serde_json::from_str(raw).unwrap_or(Value::String(raw.to_string()))
}

fn serve(host: &str, port: u16, db_path: PathBuf, required_token: Option<String>) -> Result<(), Box<dyn Error>> {
    let address = format!("{host}:{port}");
    let listener = TcpListener::bind(&address)?;
    println!("Supernova DB server started on {address}");

    for stream in listener.incoming() {
        let Ok(mut stream) = stream else {
            continue;
        };

        let mut header = [0u8; 8];
        if stream.read_exact(&mut header).is_err() {
            continue;
        }

        if &header[0..4] != b"SDB2" {
            let _ = stream.write_all(b"INVALID_PROTOCOL");
            continue;
        }

        let len = u32::from_be_bytes([header[4], header[5], header[6], header[7]]) as usize;
        let mut payload = vec![0u8; len];
        if stream.read_exact(&mut payload).is_err() {
            continue;
        }

        let request = match protocol::decode_request(&[header.as_slice(), payload.as_slice()].concat()) {
            Ok(req) => req,
            Err(err) => {
                let response = protocol::Response {
                    ok: false,
                    message: err,
                    value: None,
                    data: None,
                };
                let _ = stream.write_all(&protocol::encode_response(&response));
                continue;
            }
        };

        if let Some(token) = &required_token {
            if request.token.as_deref() != Some(token.as_str()) {
                let response = protocol::Response {
                    ok: false,
                    message: "unauthorized".to_string(),
                    value: None,
                    data: None,
                };
                let _ = stream.write_all(&protocol::encode_response(&response));
                continue;
            }
        }

        let response = match request.op.as_str() {
            "ping" => protocol::Response {
                ok: true,
                message: "pong".to_string(),
                value: None,
                data: None,
            },
            "create_table" => {
                let mut db = SupernovaDb::open(&db_path)?;
                db.create_table(&request.table)?;
                protocol::Response {
                    ok: true,
                    message: "table created".to_string(),
                    value: None,
                    data: Some(serde_json::json!({"table": request.table})),
                }
            }
            "delete_table" => {
                let mut db = SupernovaDb::open(&db_path)?;
                let removed = db.delete_table(&request.table)?;
                protocol::Response {
                    ok: removed.is_some(),
                    message: if removed.is_some() { "table removed".to_string() } else { "table not found".to_string() },
                    value: None,
                    data: None,
                }
            }
            "set" => {
                let mut db = SupernovaDb::open(&db_path)?;
                let value = request.value.unwrap_or(Value::Null);
                db.set(&request.table, request.key.clone(), value.clone())?;
                protocol::Response {
                    ok: true,
                    message: "stored".to_string(),
                    value: Some(value),
                    data: None,
                }
            }
            "get" => {
                let db = SupernovaDb::open(&db_path)?;
                match db.get(&request.table, &request.key) {
                    Some(value) => protocol::Response {
                        ok: true,
                        message: "found".to_string(),
                        value: Some(value.clone()),
                        data: Some(serde_json::json!({"table": request.table, "key": request.key, "value": value})),
                    },
                    None => protocol::Response {
                        ok: false,
                        message: "not found".to_string(),
                        value: None,
                        data: None,
                    },
                }
            }
            "delete" => {
                let mut db = SupernovaDb::open(&db_path)?;
                match db.delete(&request.table, &request.key)? {
                    Some(value) => protocol::Response {
                        ok: true,
                        message: "deleted".to_string(),
                        value: Some(value.clone()),
                        data: Some(serde_json::json!({"table": request.table, "key": request.key, "value": value})),
                    },
                    None => protocol::Response {
                        ok: false,
                        message: "not found".to_string(),
                        value: None,
                        data: None,
                    },
                }
            }
            "list" => {
                let db = SupernovaDb::open(&db_path)?;
                let rows = db.list(&request.table);
                protocol::Response {
                    ok: true,
                    message: "ok".to_string(),
                    value: None,
                    data: Some(serde_json::json!({"table": request.table, "rows": rows })),
                }
            }
            "tables" => {
                let db = SupernovaDb::open(&db_path)?;
                protocol::Response {
                    ok: true,
                    message: "ok".to_string(),
                    value: None,
                    data: Some(serde_json::json!(db.table_names())),
                }
            }
            _ => protocol::Response {
                ok: false,
                message: format!("unsupported operation: {}", request.op),
                value: None,
                data: None,
            },
        };

        let encoded = protocol::encode_response(&response);
        let _ = stream.write_all(&encoded);
    }

    Ok(())
}
