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
#[command(name = "supernova-db", about = "Supernova DB: custom Rust storage engine and protocol")]
struct Cli {
    #[arg(long, global = true, default_value = "supernova_data")]
    db: PathBuf,
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    CreateTable { table: String },
    DeleteTable { table: String },
    Tables,
    Set { table: String, key: String, value: String },
    Get { table: String, key: String },
    Delete { table: String, key: String },
    List { table: String },
    Query { table: String, field: String, value: String },
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
        Commands::CreateTable { table } => SupernovaDb::open(cli.db)?.create_table(&table)?,
        Commands::DeleteTable { table } => println!("{}", SupernovaDb::open(cli.db)?.delete_table(&table)?),
        Commands::Tables => for table in SupernovaDb::open(cli.db)?.table_names() { println!("{table}"); },
        Commands::Set { table, key, value } => {
            SupernovaDb::open(cli.db)?.set(&table, key, parse_value(&value))?;
            println!("OK");
        }
        Commands::Get { table, key } => match SupernovaDb::open(cli.db)?.get(&table, &key) {
            Some(value) => println!("{value}"), None => println!("<not found>"),
        },
        Commands::Delete { table, key } => match SupernovaDb::open(cli.db)?.delete(&table, &key)? {
            Some(value) => println!("{value}"), None => println!("<not found>"),
        },
        Commands::List { table } => for (key, value) in SupernovaDb::open(cli.db)?.list(&table) { println!("{key}={value}"); },
        Commands::Query { table, field, value } => {
            for (key, row) in SupernovaDb::open(cli.db)?.query_eq(&table, &field, &parse_value(&value)) { println!("{key}={row}"); }
        }
        Commands::CreateIndex { table, field } => SupernovaDb::open(cli.db)?.create_index(&table, &field)?,
        Commands::Indexes { table } => for field in SupernovaDb::open(cli.db)?.indexes(&table) { println!("{field}"); },
        Commands::Checkpoint => SupernovaDb::open(cli.db)?.checkpoint()?,
        Commands::Serve { host, port, token } => serve(&host, port, cli.db, token)?,
    }
    Ok(())
}

fn parse_value(input: &str) -> Value {
    serde_json::from_str(input).unwrap_or_else(|_| Value::String(input.to_owned()))
}

fn serve(host: &str, port: u16, path: PathBuf, token: Option<String>) -> Result<(), Box<dyn Error>> {
    let listener = TcpListener::bind((host, port))?;
    let address = listener.local_addr()?;
    let db = Arc::new(Mutex::new(SupernovaDb::open(path)?));
    println!("Supernova DB listening on {address}");
    for incoming in listener.incoming() {
        match incoming {
            Ok(stream) => {
                let db = Arc::clone(&db);
                let token = token.clone();
                thread::spawn(move || {
                    if let Err(error) = handle_client(stream, db, token) {
                        eprintln!("client error: {error}");
                    }
                });
            }
            Err(error) => eprintln!("accept error: {error}"),
        }
    }
    Ok(())
}

fn handle_client(mut stream: TcpStream, db: Arc<Mutex<SupernovaDb>>, token: Option<String>) -> io::Result<()> {
    stream.set_read_timeout(Some(Duration::from_secs(10)))?;
    stream.set_write_timeout(Some(Duration::from_secs(10)))?;
    let request = match protocol::read_request(&mut stream) {
        Ok(request) => request,
        Err(error) => {
            return protocol::write_response(&mut stream, &protocol::Response { ok: false, message: error.to_string(), value: None, data: None });
        }
    };
    if token.as_ref().is_some_and(|expected| request.token.as_ref() != Some(expected)) {
        return protocol::write_response(&mut stream, &protocol::Response { ok: false, message: "unauthorized".into(), value: None, data: None });
    }
    let result = execute_request(&request, &db);
    let response = match result {
        Ok((message, value, data)) => protocol::Response { ok: true, message, value, data },
        Err(error) => protocol::Response { ok: false, message: error, value: None, data: None },
    };
    protocol::write_response(&mut stream, &response)
}

fn execute_request(request: &protocol::Request, db: &Arc<Mutex<SupernovaDb>>) -> Result<(String, Option<Value>, Option<Value>), String> {
    let mut db = db.lock().map_err(|_| "database lock poisoned".to_string())?;
    let result = match request.op.as_str() {
        "ping" => ("pong".into(), None, None),
        "tables" => ("ok".into(), None, Some(json!(db.table_names()))),
        "create_table" => { db.create_table(&request.table).map_err(|e| e.to_string())?; ("table created".into(), None, None) }
        "delete_table" => (if db.delete_table(&request.table).map_err(|e| e.to_string())? { "table deleted" } else { "table not found" }.into(), None, None),
        "set" => { let value = request.value.clone().unwrap_or(Value::Null); db.set(&request.table, request.key.clone(), value.clone()).map_err(|e| e.to_string())?; ("stored".into(), Some(value), None) }
        "get" => match db.get(&request.table, &request.key) { Some(value) => ("found".into(), Some(value), None), None => ("not found".into(), None, None) },
        "delete" => match db.delete(&request.table, &request.key).map_err(|e| e.to_string())? { Some(value) => ("deleted".into(), Some(value), None), None => ("not found".into(), None, None) },
        "list" => ("ok".into(), None, Some(json!(db.list(&request.table)))),
        "query" => {
            let field = request.field.as_deref().ok_or("query requires field")?;
            let expected = request.value.as_ref().ok_or("query requires value")?;
            ("ok".into(), None, Some(json!(db.query_eq(&request.table, field, expected))))
        }
        "create_index" => { let field = request.field.as_deref().ok_or("create_index requires field")?; db.create_index(&request.table, field).map_err(|e| e.to_string())?; ("index created".into(), None, None) }
        "indexes" => ("ok".into(), None, Some(json!(db.indexes(&request.table)))),
        "checkpoint" => { db.checkpoint().map_err(|e| e.to_string())?; ("checkpoint complete".into(), None, None) }
        _ => return Err(format!("unsupported operation: {}", request.op)),
    };
    Ok(result)
}
