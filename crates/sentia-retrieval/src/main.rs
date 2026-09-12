use std::collections::VecDeque;
use std::path::PathBuf;

use sentia_retrieval::{
    error_to_envelope, Indexer, QueryEngine, QueryRequest, RetrievalConfig, RetrievalError,
};

fn main() {
    match run(std::env::args().skip(1).collect()) {
        Ok(Some(output)) => println!("{output}"),
        Ok(None) => {}
        Err(err) => {
            let envelope = error_to_envelope(&err);
            eprintln!(
                "{}",
                serde_json::to_string_pretty(&envelope).expect("serialize error")
            );
            std::process::exit(1);
        }
    }
}

fn run(raw_args: Vec<String>) -> Result<Option<String>, RetrievalError> {
    let command = parse_command(raw_args)?;

    match command {
        Command::Help => {
            print_usage();
            Ok(None)
        }
        Command::Index(args) => {
            let config = RetrievalConfig::load(&args.config)?;
            let report = Indexer::new(args.index).rebuild(&config)?;
            Ok(Some(serialize_json(&report, args.pretty)))
        }
        Command::Query(args) => {
            let request = QueryRequest {
                query: args.query,
                limit: args.limit.min(64),
                max_content_chars: args.max_content_chars.min(2_000),
            };

            let engine = QueryEngine::new(args.index);
            if args.router_context {
                let response = engine.query_router_context(request)?;
                Ok(Some(serialize_json(&response, args.pretty)))
            } else {
                let response = engine.query(request)?;
                Ok(Some(serialize_json(&response, args.pretty)))
            }
        }
    }
}

#[derive(Debug)]
enum Command {
    Help,
    Index(IndexArgs),
    Query(QueryArgs),
}

#[derive(Debug)]
struct IndexArgs {
    config: PathBuf,
    index: PathBuf,
    pretty: bool,
}

#[derive(Debug)]
struct QueryArgs {
    index: PathBuf,
    query: String,
    limit: usize,
    max_content_chars: usize,
    router_context: bool,
    pretty: bool,
}

fn parse_command(args: Vec<String>) -> Result<Command, RetrievalError> {
    let mut queue = VecDeque::from(args);
    let Some(command) = queue.pop_front() else {
        return Ok(Command::Help);
    };

    if command == "--help" || command == "-h" || command == "help" {
        return Ok(Command::Help);
    }

    match command.as_str() {
        "index" => parse_index(queue).map(Command::Index),
        "query" => parse_query(queue).map(Command::Query),
        other => Err(RetrievalError::InvalidConfig(format!(
            "unknown subcommand {other}; expected index or query"
        ))),
    }
}

fn parse_index(mut args: VecDeque<String>) -> Result<IndexArgs, RetrievalError> {
    let mut config = PathBuf::from("/etc/sentia/retrieval/sources.toml");
    let mut index = PathBuf::from("/var/lib/sentia/retrieval/index.sqlite");
    let mut pretty = false;

    while let Some(flag) = args.pop_front() {
        match flag.as_str() {
            "--config" => {
                config = PathBuf::from(next_value("--config", &mut args)?);
            }
            "--index" => {
                index = PathBuf::from(next_value("--index", &mut args)?);
            }
            "--pretty" => {
                pretty = true;
            }
            "--help" | "-h" => {
                print_usage();
                std::process::exit(0);
            }
            other => {
                return Err(RetrievalError::InvalidConfig(format!(
                    "unknown index flag {other}"
                )));
            }
        }
    }

    Ok(IndexArgs {
        config,
        index,
        pretty,
    })
}

fn parse_query(mut args: VecDeque<String>) -> Result<QueryArgs, RetrievalError> {
    let mut index = PathBuf::from("/var/lib/sentia/retrieval/index.sqlite");
    let mut query = None;
    let mut limit = 6;
    let mut max_content_chars = 320;
    let mut router_context = false;
    let mut pretty = false;

    while let Some(flag) = args.pop_front() {
        match flag.as_str() {
            "--index" => {
                index = PathBuf::from(next_value("--index", &mut args)?);
            }
            "--query" => {
                query = Some(next_value("--query", &mut args)?);
            }
            "--limit" => {
                let raw = next_value("--limit", &mut args)?;
                limit = raw.parse::<usize>().map_err(|_| {
                    RetrievalError::InvalidConfig("--limit must be an integer".to_string())
                })?;
            }
            "--max-content-chars" => {
                let raw = next_value("--max-content-chars", &mut args)?;
                max_content_chars = raw.parse::<usize>().map_err(|_| {
                    RetrievalError::InvalidConfig(
                        "--max-content-chars must be an integer".to_string(),
                    )
                })?;
            }
            "--router-context" => {
                router_context = true;
            }
            "--pretty" => {
                pretty = true;
            }
            "--help" | "-h" => {
                print_usage();
                std::process::exit(0);
            }
            other => {
                return Err(RetrievalError::InvalidConfig(format!(
                    "unknown query flag {other}"
                )));
            }
        }
    }

    let query = query.ok_or_else(|| {
        RetrievalError::InvalidConfig("query requires --query <text>".to_string())
    })?;

    Ok(QueryArgs {
        index,
        query,
        limit,
        max_content_chars,
        router_context,
        pretty,
    })
}

fn next_value(flag: &str, args: &mut VecDeque<String>) -> Result<String, RetrievalError> {
    args.pop_front()
        .ok_or_else(|| RetrievalError::InvalidConfig(format!("missing value for {flag}")))
}

fn serialize_json<T: serde::Serialize>(value: &T, pretty: bool) -> String {
    if pretty {
        serde_json::to_string_pretty(value).expect("serialize")
    } else {
        serde_json::to_string(value).expect("serialize")
    }
}

fn print_usage() {
    eprintln!(
        "sentia-retrieval\n\n\
         Usage:\n\
           sentia-retrieval index [--config PATH] [--index PATH] [--pretty]\n\
           sentia-retrieval query --query TEXT [--index PATH] [--limit N] [--max-content-chars N] [--router-context] [--pretty]\n"
    );
}
