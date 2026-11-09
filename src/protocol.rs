use clap::{Parser, Subcommand};
use supernova_db::{protocol, SupernovaDb};
use std::error::Error;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::path::PathBuf;

#[derive(Parser, Debug)]
#[command(name = "supernova-db", about = "Custom Rust database with a beginner-friendly protocol")]
struct Cli {
    #[arg(long, default_value = "supernova.db")]
    db: PathBuf,

    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand, Debug)]
enum Commands {
    Set { key: String, value: String },
    Get { key: String },
    Delete { key: String },
    List,
    Serve {
        #[arg(long, default_value = "127.0.0.1")]
        host: String,
        #[arg(long, default_value_t = 8080)]
        port: u16,
    },
}

fn main() -> Result<(), Box<dyn Error>> {
    let cli = Cli::parse();

    match cli.command {
        Commands::Set { key, value } => {
            let mut db = SupernovaDb::open(cli.db)?;
            db.set(key, value)?;
            println!("OK");
        }
        Commands::Get { key } => {
            let db = SupernovaDb::open(cli.db)?;
            match db.get(&key) {
                Some(value) => println!("{value}"),
                None => println!("<not found>"),
            }
        }
        Commands::Delete { key } => {
            let mut db = SupernovaDb::open(cli.db)?;
            match db.delete(&key)? {
                Some(value) => println!("deleted: {value}"),
                None => println!("<not found>"),
            }
        }
        Commands::List => {
            let db = SupernovaDb::open(cli.db)?;
            for (key, value) in db.list() {
                println!("{key}={value}");
            }
        }
        Commands::Serve { host, port } => {
            serve(&host, port, cli.db)?;
        }
    }

    Ok(())
}

fn serve(host: &str, port: u16, db_path: PathBuf) -> Result<(), Box<dyn Error>> {
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

        let expected_magic = b"SDB1";
        if &header[0..4] != expected_magic {
            let _ = stream.write_all(b"INVALID_PROTOCOL");
            continue;
        }

        let len = u32::from_be_bytes([header[4], header[5], header[6], header[7]]) as usize;
        let mut payload = vec![0u8; len];
        if stream.read_exact(&mut payload).is_err() {
            continue;
        }

        let mut frame = Vec::with_capacity(8 + payload.len());
        frame.extend_from_slice(&header);
        frame.extend_from_slice(&payload);

        let request = match protocol::decode_request(&frame) {
            Ok(req) => req,
            Err(err) => {
                let response = protocol::Response {
                    ok: false,
                    message: err,
                    value: None,
                };
                let encoded = protocol::encode_response(&response);
                let _ = stream.write_all(&encoded);
                continue;
            }
        };

        let response = match request.op.to_ascii_lowercase().as_str() {
            "set" => {
                let mut db = SupernovaDb::open(&db_path)?;
                let value = request.value.unwrap_or_default();
                db.set(request.key.clone(), value.clone())?;
                protocol::Response {
                    ok: true,
                    message: "stored".into(),
                    value: Some(value),
                }
            }
            "get" => {
                let db = SupernovaDb::open(&db_path)?;
                match db.get(&request.key) {
                    Some(value) => protocol::Response {
                        ok: true,
                        message: "found".into(),
                        value: Some(value),
                    },
                    None => protocol::Response {
                        ok: false,
                        message: "not found".into(),
                        value: None,
                    },
                }
            }
            "delete" => {
                let mut db = SupernovaDb::open(&db_path)?;
                match db.delete(&request.key)? {
                    Some(value) => protocol::Response {
                        ok: true,
                        message: "deleted".into(),
                        value: Some(value),
                    },
                    None => protocol::Response {
                        ok: false,
                        message: "not found".into(),
                        value: None,
                    },
                }
            }
            "ping" => protocol::Response {
                ok: true,
                message: "pong".into(),
                value: None,
            },
            _ => protocol::Response {
                ok: false,
                message: format!("unsupported operation: {}", request.op),
                value: None,
            },
        };

        let encoded = protocol::encode_response(&response);
        let _ = stream.write_all(&encoded);
    }

    Ok(())
}
