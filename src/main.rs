use clap::{Parser, Subcommand};
use serde_json::{json, Value};
use std::error::Error;
use std::io::{self, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;
use std::time::Duration;
use supernova_db::{protocol, SupernovaDb};

#[derive(Parser)]
#[command(name="supernova-db", about="Beginner-friendly multi-model Rust database starter")]
struct Cli { #[arg(long, global=true, default_value="supernova_data")] db: PathBuf, #[command(subcommand)] command: Command }
#[derive(Subcommand)]
enum Command {
    CreateCollection { name:String }, Collections,
    Put { collection:String, id:String, document:String }, Get { collection:String, id:String }, Delete { collection:String, id:String }, List { collection:String },
    CreateTextIndex { collection:String, field:String }, SearchText { collection:String, field:String, query:String, #[arg(long,default_value_t=10)] limit:usize },
    CreateVectorIndex { collection:String, field:String, dimensions:usize }, VectorSearch { collection:String, field:String, vector:String, #[arg(long,default_value_t=10)] limit:usize },
    Sql { statement:String }, Serve { #[arg(long,default_value="127.0.0.1")] host:String, #[arg(long,default_value_t=8080)] port:u16, #[arg(long)] token:Option<String> },
}
fn main() -> Result<(), Box<dyn Error>> {
    let cli=Cli::parse();
    match cli.command {
        Command::CreateCollection{name}=>SupernovaDb::open(cli.db)?.create_collection(&name).map_err(other)?,
        Command::Collections=>for c in SupernovaDb::open(cli.db)?.collections(){println!("{} documents={} text={:?} vectors={:?}",c.name,c.document_count,c.text_fields,c.vector_fields)},
        Command::Put{collection,id,document}=>{let v=serde_json::from_str(&document)?;SupernovaDb::open(cli.db)?.put(&collection,&id,v).map_err(other)?;println!("OK")},
        Command::Get{collection,id}=>match SupernovaDb::open(cli.db)?.get(&collection,&id){Some(v)=>println!("{v}"),None=>println!("<not found>")},
        Command::Delete{collection,id}=>match SupernovaDb::open(cli.db)?.delete(&collection,&id).map_err(other)?{Some(v)=>println!("{v}"),None=>println!("<not found>")},
        Command::List{collection}=>for(id,v)in SupernovaDb::open(cli.db)?.list(&collection){println!("{id}={v}")},
        Command::CreateTextIndex{collection,field}=>SupernovaDb::open(cli.db)?.create_text_index(&collection,&field).map_err(other)?,
        Command::SearchText{collection,field,query,limit}=>for(id,v,score)in SupernovaDb::open(cli.db)?.search_text(&collection,&field,&query,limit).map_err(other)?{println!("{id} score={score} {v}")},
        Command::CreateVectorIndex{collection,field,dimensions}=>SupernovaDb::open(cli.db)?.create_vector_index(&collection,&field,dimensions).map_err(other)?,
        Command::VectorSearch{collection,field,vector,limit}=>{let q:Vec<f64>=serde_json::from_str(&vector)?;for(id,v,score)in SupernovaDb::open(cli.db)?.search_vector(&collection,&field,&q,limit).map_err(other)?{println!("{id} cosine={score:.6} {v}")}},
        Command::Sql{statement}=>for(id,v)in SupernovaDb::open(cli.db)?.sql(&statement).map_err(other)?{println!("{id}={v}")},
        Command::Serve{host,port,token}=>serve(&host,port,cli.db,token)?,
    } Ok(())
}
fn other(s:String)->io::Error{io::Error::new(io::ErrorKind::Other,s)}
fn serve(host:&str,port:u16,path:PathBuf,token:Option<String>)->Result<(),Box<dyn Error>>{
    let listener=TcpListener::bind((host,port))?; let mut db=SupernovaDb::open(path).map_err(other)?; println!("Supernova DB listening on {host}:{port}");
    for incoming in listener.incoming(){let Ok(mut stream)=incoming else{continue};if let Err(e)=handle(&mut stream,&mut db,token.as_deref()){eprintln!("client error: {e}")}}
    Ok(())
}
fn handle(stream:&mut TcpStream,db:&mut SupernovaDb,token:Option<&str>)->io::Result<()>{
    stream.set_read_timeout(Some(Duration::from_secs(10)))?;stream.set_write_timeout(Some(Duration::from_secs(10)))?;
    let request=match protocol::read_request(stream){Ok(r)=>r,Err(e)=>return protocol::write_response(stream,&protocol::Response{ok:false,message:e.to_string(),data:None})};
    let result=if token.is_some_and(|expected|request.token.as_deref()!=Some(expected)){Err("unauthorized".to_string())}else{execute(&request,db)};
    let response=match result{Ok(data)=>protocol::Response{ok:true,message:"ok".into(),data},Err(message)=>protocol::Response{ok:false,message,data:None}};
    protocol::write_response(stream,&response)
}
fn execute(r:&protocol::Request,db:&mut SupernovaDb)->Result<Option<Value>,String>{match r.op.as_str(){
    "ping"=>Ok(Some(json!("pong"))),"collections"=>Ok(Some(json!(db.collections()))),
    "create_collection"=>{db.create_collection(&r.collection)?;Ok(None)},"put"=>{db.put(&r.collection,&r.id,r.document.clone().ok_or("document missing")?)?;Ok(None)},
    "get"=>Ok(db.get(&r.collection,&r.id)),"delete"=>Ok(db.delete(&r.collection,&r.id)?),"list"=>Ok(Some(json!(db.list(&r.collection)))),
    "create_text_index"=>{db.create_text_index(&r.collection,r.field.as_deref().ok_or("field missing")?)?;Ok(None)},
    "search_text"=>Ok(Some(json!(db.search_text(&r.collection,r.field.as_deref().ok_or("field missing")?,r.query.as_deref().ok_or("query missing")?,r.limit.unwrap_or(10))?))),
    "create_vector_index"=>{db.create_vector_index(&r.collection,r.field.as_deref().ok_or("field missing")?,r.dimensions.ok_or("dimensions missing")?)?;Ok(None)},
    "vector_search"=>Ok(Some(json!(db.search_vector(&r.collection,r.field.as_deref().ok_or("field missing")?,r.vector.as_deref().ok_or("vector missing")?,r.limit.unwrap_or(10))?))),
    "sql"=>Ok(Some(json!(db.sql(r.query.as_deref().ok_or("query missing")?)?))),_=>Err(format!("unsupported operation: {}",r.op))}}
