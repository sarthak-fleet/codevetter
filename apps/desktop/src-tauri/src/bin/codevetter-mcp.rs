use codevetter_desktop::mcp::{sanitize::sanitize_error_message, server::CodeVetterMcpServer};
use rmcp::ServiceExt;
use std::path::PathBuf;

#[tokio::main]
async fn main() {
    if let Err(error) = run().await {
        eprintln!("codevetter-mcp: {}", sanitize_error_message(&error, ""));
        std::process::exit(1);
    }
}

async fn run() -> Result<(), String> {
    let arguments = parse_arguments(std::env::args().skip(1))?;
    let server = CodeVetterMcpServer::new(arguments.database, arguments.repo_id)?;
    let service = server
        .serve(rmcp::transport::stdio())
        .await
        .map_err(|error| format!("Start stdio transport: {error}"))?;
    service
        .waiting()
        .await
        .map_err(|error| format!("Serve stdio transport: {error}"))?;
    Ok(())
}

#[derive(Debug, PartialEq, Eq)]
struct Arguments {
    database: PathBuf,
    repo_id: String,
}

fn parse_arguments(arguments: impl IntoIterator<Item = String>) -> Result<Arguments, String> {
    let mut database = None;
    let mut repo_id = None;
    let mut arguments = arguments.into_iter();
    while let Some(argument) = arguments.next() {
        match argument.as_str() {
            "--database" => {
                database = Some(PathBuf::from(
                    arguments
                        .next()
                        .ok_or_else(|| "--database requires a path".to_string())?,
                ));
            }
            "--repo-id" => {
                repo_id = Some(
                    arguments
                        .next()
                        .ok_or_else(|| "--repo-id requires an opaque identity".to_string())?,
                );
            }
            "--help" | "-h" => {
                return Err(
                    "usage: codevetter-mcp --database <codevetter.db> --repo-id <opaque-id>"
                        .to_string(),
                );
            }
            _ => return Err("Unknown codevetter-mcp argument".to_string()),
        }
    }
    Ok(Arguments {
        database: database.ok_or_else(|| "--database is required".to_string())?,
        repo_id: repo_id.ok_or_else(|| "--repo-id is required".to_string())?,
    })
}
