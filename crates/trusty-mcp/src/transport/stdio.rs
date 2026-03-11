//! Stdio transport: newline-delimited JSON-RPC over stdin/stdout.
//!
//! All tracing output goes to stderr to keep stdout clean for the JSON-RPC stream.

use std::sync::Arc;

use anyhow::Result;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tracing::{debug, warn};

use crate::protocol::JsonRpcRequest;
use crate::server::McpServer;

pub async fn run_stdio(server: Arc<McpServer>) -> Result<()> {
    let stdin = tokio::io::stdin();
    let stdout = tokio::io::stdout();
    let mut reader = BufReader::new(stdin).lines();
    let mut writer = tokio::io::BufWriter::new(stdout);

    while let Some(line) = reader.next_line().await? {
        let line = line.trim().to_string();
        if line.is_empty() {
            continue;
        }
        debug!(bytes = line.len(), "stdio recv");

        let resp = match serde_json::from_str::<JsonRpcRequest>(&line) {
            Ok(req) => server.handle(req).await,
            Err(e) => {
                warn!(error = %e, "failed to parse JSON-RPC request");
                crate::protocol::JsonRpcResponse::parse_error()
            }
        };

        let bytes = serde_json::to_vec(&resp)?;
        writer.write_all(&bytes).await?;
        writer.write_all(b"\n").await?;
        writer.flush().await?;
    }

    Ok(())
}
