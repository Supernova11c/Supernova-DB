use clap::{Parser, Subcommand};
use serde_json::{json, Value};
use std::error::Error;
use std::io;
use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use supernova_db::{protocol, SupernovaDb};

#[derive(Parser)]
#[command(name = "supernova-db", about = "Supernova DB command line and server")]
struct Cli {
    #[arg(long, global = true, default_value = "supernova_data")]
    db: PathBuf,

    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    CreateTable {
        table: String,
        #[arg(long, value_delimiter = ',')]
        columns: Vec<String>,
    },
    DeleteTable { table: String },
    Tables,
    Set { table: String, key: String, value: String },
    Get { table: String, key: String },
    Delete { table: String, key: String },
    List {
        table: String,
        #[arg(long)] sort_by: Option<String>,
        #[arg(long)] limit: Option<usize>,
    },
    Query { table: String, predicate: String },
    CreateIndex { table: String, field: String },
    Indexes { table: String },
    Checkpoint,
    Serve {
        #[arg(long, default_value = "127.0.0.1")] host: String,
        #[arg(long, default_value_t = 8080)] port: u16,
        #[arg(long)] token: Option<String>,
    },
}

fn main() -> Result<(), Box<dyn Error>> {
    let cli = Cli::parse();

    match cli.command {
        Commands::CreateTable { table, columns } => {
            let mut db = SupernovaDb::open(cli.db)?;
            if columns.is_empty() {
                db.create_table(&table)?;
            } else {
                db.create_table_with_schema(&table, &columns)?;
            }
            println!("OK");
        }
        Commands::DeleteTable { table } => {
            let mut db = SupernovaDb::open(cli.db)?;
            println!("{}", db.delete_table(&table)?);
        }
        Commands::Tables => {
            let db = SupernovaDb::open(cli.db)?;
            for name in db.table_names() {
                println!("{name}");
            }
        }
        Commands::Set { table, key, value } => {
            let mut db = SupernovaDb::open(cli.db)?;
            db.set(&table, key, parse_value(&value))?;
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
                Some(value) => println!("{value}"),
                None => println!("<not found>"),
            }
        }
        Commands::List { table, sort_by, limit } => {
            let db = SupernovaDb::open(cli.db)?;
            for (key, row) in db.list(&table, sort_by.as_deref(), limit) {
                println!("{key}={row}");
            }
        }
        Commands::Query { table, predicate } => {
            let db = SupernovaDb::open(cli.db)?;
            let rows = db.query(&table, &predicate);
            for (key, value) in rows {
                println!("{key}={value}");
            }
        }
        Commands::CreateIndex { table, field } => {
            let mut db = SupernovaDb::open(cli.db)?;
            db.create_index(&table, &field)?;
            println!("OK");
        }
        Commands::Indexes { table } => {
            let db = SupernovaDb::open(cli.db)?;
            for field in db.indexes(&table) {
                println!("{field}");
            }
        }
        Commands::Checkpoint => {
            let mut db = SupernovaDb::open(cli.db)?;
            db.checkpoint()?;
            println!("OK");
        }
        Commands::Serve { host, port, token } => {
            serve(&host, port, cli.db, token)?;
        }
    }

    Ok(())
}

fn parse_value(raw: &str) -> Value {
    serde_json::from_str(raw).unwrap_or(Value::String(raw.to_owned()))
}

fn serve(host: &str, port: u16, path: PathBuf, token: Option<String>) -> Result<(), Box<dyn Error>> {
    let listener = TcpListener::bind((host, port))?;
    let mut addr = listener.local_addr()?;
    println!("Supernova DB listening on {}:{}", addr.ip(), addr.port());
    let db = Arc::new(Mutex::new(SupernovaDb::open(path)?));

    for stream in listener.incoming() {
        let Ok(stream) = stream else {
            continue;
        };
        let db = Arc::clone(&db);
        let token = token.clone();
        thread::spawn(move || {
            if let Err(error) = handle_client(stream, db, token) {
                eprintln!("client error: {error}");
            }
        });
    }

    Ok(())
}

fn handle_client(mut stream: TcpStream, db: Arc<Mutex<SupernovaDb>>, token: Option<String>) -> io::Result<()> {
    stream.set_read_timeout(Some(Duration::from_secs(10)))?;
    stream.set_write_timeout(Some(Duration::from_secs(10)))?;

    let request = match protocol::read_request(&mut stream) {
        Ok(req) => req,
        Err(error) => {
            return protocol::write_response(&mut stream, &protocol::Response {
                ok: false,
                message: error.to_string(),
                value: None,
                data: None,
            });
        }
    };

    if token.as_ref().is_some_and(|expected| request.token.as_deref() != Some(expected)) {
        return protocol::write_response(&mut stream, &protocol::Response {
            ok: false,
            message: "unauthorized".to_string(),
            value: None,
            data: None,
        });
    }

    let response = match execute_request(&request, &db) {
        Ok((message, value, data)) => protocol::Response { ok: true, message, value, data },
        Err(error) => protocol::Response { ok: false, message: error, value: None, data: None },
    };

    protocol::write_response(&mut stream, &response)
}

fn execute_request(request: &protocol::Request, db: &Arc<Mutex<SupernovaDb>>) -> Result<(String, Option<Value>, Option<Value>), String> {
    let mut db = db.lock().map_err(|_| "database lock poisoned".to_string())?;

    match request.op.as_str() {
        "ping" => Ok(("pong".to_string(), None, None)),
        "tables" => Ok(("ok".to_string(), None, Some(json!(db.table_names())))),
        "create_table" => {
            if request.columns.as_ref().is_some_and(|v| !v.is_empty()) {
                db.create_table_with_schema(&request.table, &request.columns.clone().unwrap_or_default())
                    .map_err(|e| e.to_string())?;
            } else {
                db.create_table(&request.table).map_err(|e| e.to_string())?;
            }
            Ok(("table created".to_string(), None, None))
        }
        "delete_table" => {
            let removed = db.delete_table(&request.table).map_err(|e| e.to_string())?;
            Ok((if removed { "table deleted" } else { "table not found" }.to_string(), None, None))
        }
        "set" => {
            let value = request.value.clone().unwrap_or(Value::Null);
            db.set(&request.table, request.key.clone(), value.clone()).map_err(|e| e.to_string())?;
            Ok(("stored".to_string(), Some(value), None))
        }
        "get" => match db.get(&request.table, &request.key) {
            Some(value) => Ok(("found".to_string(), Some(value), None)),
            None => Ok(("not found".to_string(), None, None)),
        },
        "delete" => match db.delete(&request.table, &request.key).map_err(|e| e.to_string())? {
            Some(value) => Ok(("deleted".to_string(), Some(value), None)),
            None => Ok(("not found".to_string(), None, None)),
        },
        "list" => {
            let rows = db.list(&request.table, request.sort_by.as_deref(), request.limit);
            Ok(("ok".to_string(), None, Some(json!(rows))))
        }
        "query" => {
            let predicate = request.query.clone().unwrap_or_else(|| request.field.clone().unwrap_or_default());
            let rows = db.query(&request.table, &predicate);
            Ok(("ok".to_string(), None, Some(json!(rows))))
        }
        "create_index" => {
            let field = request.field.as_deref().ok_or_else(|| "create_index requires field".to_string())?;
            db.create_index(&request.table, field).map_err(|e| e.to_string())?;
            Ok(("index created".to_string(), None, None))
        }
        "indexes" => Ok(("ok".to_string(), None, Some(json!(db.indexes(&request.table))))),
        "checkpoint" => {
            db.checkpoint().map_err(|e| e.to_string())?;
            Ok(("checkpoint complete".to_string(), None, None))
        }
        _ => Err(format!("unsupported operation: {}", request.op)),
    }
}
