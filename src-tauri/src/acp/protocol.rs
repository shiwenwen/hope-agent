//! NDJSON + JSON-RPC 2.0 transport layer over stdio

use std::io::{self, BufRead, Write};

use anyhow::Result;
use serde_json::Value;

use crate::acp::types::{JsonRpcMessage, JsonRpcResponse, JsonRpcNotification, ERROR_PARSE};

/// Newline-Delimited JSON transport over stdin/stdout.
///
/// Each message is a single JSON object terminated by `\n`.
/// This is the standard ACP transport per the protocol spec.
pub struct NdJsonTransport {
    stdin: io::BufReader<io::Stdin>,
    stdout: io::Stdout,
}

impl NdJsonTransport {
    pub fn new() -> Self {
        Self {
            stdin: io::BufReader::new(io::stdin()),
            stdout: io::stdout(),
        }
    }

    /// Read the next JSON-RPC message from stdin. Returns None on EOF.
    pub fn read_message(&mut self) -> Result<Option<JsonRpcMessage>> {
        let mut line = String::new();
        loop {
            line.clear();
            let bytes_read = self.stdin.read_line(&mut line)?;
            if bytes_read == 0 {
                return Ok(None); // EOF
            }
            let trimmed = line.trim();
            if trimmed.is_empty() {
                continue; // skip blank lines
            }
            match serde_json::from_str::<JsonRpcMessage>(trimmed) {
                Ok(msg) => return Ok(Some(msg)),
                Err(e) => {
                    // Send parse error and continue
                    let err_resp = JsonRpcResponse::error(
                        Value::Null,
                        ERROR_PARSE,
                        format!("Parse error: {}", e),
                    );
                    self.write_response(&err_resp)?;
                    continue;
                }
            }
        }
    }

    /// Write a JSON-RPC response to stdout
    pub fn write_response(&mut self, response: &JsonRpcResponse) -> Result<()> {
        let json = serde_json::to_string(response)?;
        writeln!(self.stdout, "{}", json)?;
        self.stdout.flush()?;
        Ok(())
    }

    /// Write a JSON-RPC notification to stdout
    pub fn write_notification(&mut self, notification: &JsonRpcNotification) -> Result<()> {
        let json = serde_json::to_string(notification)?;
        writeln!(self.stdout, "{}", json)?;
        self.stdout.flush()?;
        Ok(())
    }
}
