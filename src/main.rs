mod config;
mod tools;

use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

#[tokio::main(flavor = "current_thread")]
async fn main() {
    // Config errors are reported per tool call, so the client can show them to the user.
    let ctx = config::load().map(tools::Ctx::new);
    let mut lines = BufReader::new(tokio::io::stdin()).lines();
    let mut out = tokio::io::stdout();

    while let Ok(Some(line)) = lines.next_line().await {
        let Ok(req) = serde_json::from_str::<Value>(&line) else { continue };
        let Some(id) = req.get("id").cloned() else { continue }; // notification
        let params = req.get("params").cloned().unwrap_or(Value::Null);

        let reply = match req["method"].as_str().unwrap_or("") {
            "initialize" => Ok(json!({
                "protocolVersion": params["protocolVersion"].as_str().unwrap_or("2025-06-18"),
                "capabilities": { "tools": {} },
                "serverInfo": { "name": "discord-mcp", "version": env!("CARGO_PKG_VERSION") }
            })),
            "ping" => Ok(json!({})),
            "tools/list" => Ok(json!({ "tools": tools::list() })),
            "tools/call" => Ok({
                let name = params["name"].as_str().unwrap_or("");
                let args = &params["arguments"];
                let res = match &ctx {
                    Ok(c) => c.call(name, args).await,
                    Err(e) => Err(e.clone()),
                };
                let (text, is_error) = match res {
                    Ok(v) => (v.to_string(), false),
                    Err(e) => (e, true),
                };
                json!({ "content": [{ "type": "text", "text": text }], "isError": is_error })
            }),
            m => Err(json!({ "code": -32601, "message": format!("method not found: {m}") })),
        };

        let msg = match reply {
            Ok(result) => json!({ "jsonrpc": "2.0", "id": id, "result": result }),
            Err(error) => json!({ "jsonrpc": "2.0", "id": id, "error": error }),
        };
        let mut buf = msg.to_string();
        buf.push('\n');
        if out.write_all(buf.as_bytes()).await.is_err() || out.flush().await.is_err() {
            break;
        }
    }
}
