use std::io::{BufRead as _, BufReader, Write as _};

fn main() {
    let stdin = std::io::stdin();
    let mut stdout = std::io::stdout();

    for line in BufReader::new(stdin.lock()).lines() {
        let Ok(line) = line else {
            return;
        };
        let Ok(msg) = serde_json::from_str::<serde_json::Value>(&line) else {
            return;
        };
        let id = msg["id"].clone();
        let method = msg["method"].as_str().unwrap_or_default();
        let response = match method {
            "initialize" => serde_json::json!({
                "jsonrpc": "2.0",
                "id": id,
                "result": { "protocolVersion": 1, "agentInfo": { "name": "fake-codex-acp" } }
            }),
            "session/new" => serde_json::json!({
                "jsonrpc": "2.0",
                "id": id,
                "result": { "sessionId": "fake-session" }
            }),
            "session/prompt" => {
                let update = serde_json::json!({
                    "jsonrpc": "2.0",
                    "method": "session/update",
                    "params": {
                        "sessionId": "fake-session",
                        "update": { "content": { "text": "mocked ACP execution complete" } }
                    }
                });
                let _ = serde_json::to_writer(&mut stdout, &update);
                let _ = stdout.write_all(b"\n");
                let _ = stdout.flush();
                serde_json::json!({
                    "jsonrpc": "2.0",
                    "id": id,
                    "result": {}
                })
            }
            _ => serde_json::json!({
                "jsonrpc": "2.0",
                "id": id,
                "error": { "code": -32601, "message": format!("unknown method {method}") }
            }),
        };
        let _ = serde_json::to_writer(&mut stdout, &response);
        let _ = stdout.write_all(b"\n");
        let _ = stdout.flush();
    }
}
