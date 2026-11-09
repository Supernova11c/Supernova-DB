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
#[command(name = "supernova-db", about = "Beginner-friendly multi-model Rust database")]
struct Cli {
    #[arg(long, global = true, default_value = "supernova_data")]
    db: PathBuf,
    #[command(subcommand)] command: Commands,
}
#[derive(Subcommand)]
enum Commands {
    CreateCollection { name: String },
    Collections,
    Put { collection: String, id: String, document: String },
    Get { collection: String, id: String },
    Delete { collection: String, id: String },
    List { collection: String },
    CreateTextIndex { collection: String, field: String },
    SearchText { collection: String, field: String, query: String, #[arg(long, default_value_t=10)] limit: usize },
    CreateVectorIndex { collection: String, field: String, dimensions: usize },
    VectorSearch { collection: String, field: String, vector: String, #[arg(long, default_value_t=10)] limit: usize },
    Sql { statement: String },
    Serve { #[arg(long, default_value="127.0.0.1")] host: String, #[arg(long, default_value_t=8080)] port: u16, #[arg(long)] token: Option<String> },
}
fn main() -> Result<(), Box<dyn Error>> {
    let cli = Cli::parse();
    match cli.command {
        Commands::CreateCollection { name } => SupernovaDb::open(cli.db)?.create_collection(&name).map_err(other)?,
        Commands::Collections => for item in SupernovaDb::open(cli.db)?.collections() { println!("{} documents={} text={:?} vectors={:?}", item.name, item.document_count, item.text_fields, item.vector_fields); },
        Commands::Put { collection, id, document } => { let value = serde_json::from_str(&document)?; SupernovaDb::open(cli.db)?.put(&collection, &id, value).map_err(other)?; println!("OK"); }
        Commands::Get { collection, id } => match SupernovaDb::open(cli.db)?.get(&collection, &id) { Some(value) => println!("{value}"), None => println!("<not found>") },
        Commands::Delete { collection, id } => match SupernovaDb::open(cli.db)?.delete(&collection, &id).map_err(other)? { Some(value) => println!("{value}"), None => println!("<not found>") },
        Commands::List { collection } => for (id, value) in SupernovaDb::open(cli.db)?.list(&collection) { println!("{id}={value}"); },
        Commands::CreateTextIndex { collection, field } => SupernovaDb::open(cli.db)?.create_text_index(&collection, &field).map_err(other)?,
        Commands::SearchText { collection, field, query, limit } => for (id, doc, score) in SupernovaDb::open(cli.db)?.search_text(&collection, &field, &query, limit).map_err(other)? { println!("{id} score={score} {doc}"); },
        Commands::CreateVectorIndex { collection, field, dimensions } => SupernovaDb::open(cli.db)?.create_vector_index(&collection, &field, dimensions).map_err(other)?,
        Commands::VectorSearch { collection, field, vector, limit } => { let vector: Vec<f64> = serde_json::from_str(&vector)?; for (id, doc, score) in SupernovaDb::open(cli.db)?.search_vector(&collection, &field, &vector, limit).map_err(other)? { println!("{id} cosine={score:.6} {doc}"); } }
        Commands::Sql { statement } => for (id, value) in SupernovaDb::open(cli.db)?.sql(&statement).map_err(other)? { println!("{id}={value}"); },
        Commands::Serve { host, port, token } => serve(&host, port, cli.db, token)?,
    }
    Ok(())
}
fn other(message: String) -> io::Error { io::Error::new(io::ErrorKind::Other, message) }
fn serve(host: &str, port: u16, path: PathBuf, token: Option<String>) -> Result<(), Box<dyn Error>> {
    let listener = TcpListener::bind((host, port))?;
    let database = Arc::new(Mutex::new(SupernovaDb::open(path).map_err(other)?));
    println!("Supernova DB listening at {host}:{port}");
    for connection in listener.incoming() {
        if let Ok(stream) = connection { let db = Arc::clone(&database); let token = token.clone(); thread::spawn(move || { if let Err(e) = handle_client(stream, db, token) { eprintln!("client: {e}"); } }); }
    }
    Ok(())
}
fn handle_client(mut stream: TcpStream, db: Arc<Mutex<SupernovaDb>>, token: Option<String>) -> io::Result<()> {
    stream.set_read_timeout(Some(Duration::from_secs(10)))?;
    stream.set_write_timeout(Some(Duration::from_secs(10)))?;
    let request = match protocol::read_request(&mut stream) { Ok(req) => req, Err(e) => return protocol::write_response(&mut stream, &protocol::Response { ok:false, message:e.to_string(), data:None }) };
    if token.as_ref().is_some_and(|expected| request.token.as_ref() != Some(expected)) { return protocol::write_response(&mut stream, &protocol::Response { ok:false, message:"unauthorized".into(), data:None }); }
    let response = match execute(&request, &db) { Ok(data) => protocol::Response { ok:true, message:"ok".into(), data }, Err(message) => protocol::Response { ok:false, message, data:None } };
    protocol::write_response(&mut stream, &response)
}
fn execute(request: &protocol::Request, db: &Arc<Mutex<SupernovaDb>>) -> Result<Option<Value>, String> {
    let mut db = db.lock().map_err(|_| "database lock poisoned".to_string())?;
    match request.op.as_str() {
        "ping" => Ok(Some(json!("pong"))),
        "collections" => Ok(Some(json!(db.collections()))),
        "create_collection" => { db.create_collection(&request.collection)?; Ok(None) }
        "put" => { db.put(&request.collection, &request.id, request.document.clone().ok_or("document is required")?)?; Ok(None) }
        "get" => Ok(db.get(&request.collection, &request.id)),
        "delete" => Ok(db.delete(&request.collection, &request.id)?),
        "list" => Ok(Some(json!(db.list(&request.collection)))),
        "create_text_index" => { db.create_text_index(&request.collection, request.field.as_deref().ok_or("field is required")?)?; Ok(None) }
        "search_text" => Ok(Some(json!(db.search_text(&request.collection, request.field.as_deref().ok_or("field is required")?, request.query.as_deref().ok_or("query is required")?, request.limit.unwrap_or(10))?))),
        "create_vector_index" => { db.create_vector_index(&request.collection, request.field.as_deref().ok_or("field is required")?, request.dimensions.ok_or("dimensions is required")?)?; Ok(None) }
        "vector_search" => Ok(Some(json!(db.search_vector(&request.collection, request.field.as_deref().ok_or("field is required")?, request.vector.as_deref().ok_or("vector is required")?, request.limit.unwrap_or(10))?))),
        "sql" => Ok(Some(json!(db.sql(request.query.as_deref().ok_or("query is required")?)?))),
        _ => Err(format!("unknown operation: {}", request.op)),
    }
}
