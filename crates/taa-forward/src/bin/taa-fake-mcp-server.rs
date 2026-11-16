//! Portable forwarding fixture, shipped as a binary so integration tests can use
//! CARGO_BIN_EXE and external SDK verification can exercise the identical process.
use serde_json::{json, Value};
use std::{sync::Arc, time::Duration};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
#[tokio::main(flavor = "current_thread")]
async fn main() {
    let args: Vec<_> = std::env::args().skip(1).collect();
    let mode = args
        .first()
        .map(String::as_str)
        .unwrap_or("good")
        .to_owned();
    let delay = args
        .get(1)
        .and_then(|s| s.parse::<f64>().ok())
        .unwrap_or(1.0);
    if let Some(path) = args
        .iter()
        .position(|s| s == "--pid-file")
        .and_then(|i| args.get(i + 1))
    {
        std::fs::write(path, std::process::id().to_string()).unwrap();
    }
    if mode == "stderr-noise" {
        eprintln!("fixture stderr noise");
    }
    if mode == "bad-stdout" {
        println!("not JSON");
    }
    let output = Arc::new(tokio::sync::Mutex::new(tokio::io::stdout()));
    let list_count = std::sync::atomic::AtomicUsize::new(0);
    let mut lines = BufReader::new(tokio::io::stdin()).lines();
    while let Ok(Some(line)) = lines.next_line().await {
        let Ok(v) = serde_json::from_str::<Value>(&line) else {
            continue;
        };
        if v["method"] == "notifications/cancelled" {
            if let Some(path) = args
                .iter()
                .position(|s| s == "--cancel-file")
                .and_then(|i| args.get(i + 1))
            {
                std::fs::write(path, line).unwrap();
            }
            continue;
        }
        if v.get("id").is_none() {
            continue;
        }
        let first_list = v["method"] == "tools/list"
            && list_count.fetch_add(1, std::sync::atomic::Ordering::Relaxed) == 0;
        let warm = args.iter().any(|s| s == "--warm-retry");
        let single = args
            .iter()
            .position(|s| s == "--single-tool")
            .and_then(|i| args.get(i + 1))
            .cloned();
        let mode = mode.clone();
        let output = output.clone();
        tokio::spawn(async move {
            let method = v["method"].as_str().unwrap_or("");
            if (mode == "slow-init" && method == "initialize")
                || (mode == "slow-list" && method == "tools/list" && (!warm || first_list))
                || (mode == "slow-call" && method == "tools/call")
            {
                tokio::time::sleep(Duration::from_secs_f64(delay)).await;
            }
            let mut response = json!({"jsonrpc":"2.0","id":v["id"]});
            response["result"] = match method {
                "initialize" => {
                    json!({"protocolVersion":"2025-06-18","capabilities":{"tools":{}},"serverInfo":{"name":"fixture","version":"1"}})
                }
                "tools/list" => {
                    let tool = |name: &str| json!({"name":name,"title":"Fixture title","description":"Fixture tool","inputSchema":{"type":"object","properties":{"text":{"type":"string"}},"additionalProperties":false},"outputSchema":{"type":"object","properties":{"text":{"type":"string"}}},"annotations":{"readOnlyHint":true},"_meta":{"fixture":true}});
                    if let Some(single) = single {
                        json!({"tools":[tool(&single)]})
                    } else if mode == "empty" {
                        json!({"tools":[]})
                    } else if mode == "paginate" {
                        if v["params"]["cursor"] == "second" {
                            json!({"tools":[tool("ping")]})
                        } else {
                            json!({"tools":[tool("echo")],"nextCursor":"second"})
                        }
                    } else {
                        json!({"tools":[tool("echo"),tool("ping"),tool("odd___name"),tool("annotated")]})
                    }
                }
                "tools/call" => {
                    if mode == "crash-on-call" {
                        std::process::exit(23);
                    }
                    if mode == "error-call" {
                        response.as_object_mut().unwrap().remove("result");
                        response["error"] = json!({"code":if v["params"]["arguments"]["code"] == -32602 { -32602 } else { -32603 },"message":"fixture error"});
                        Value::Null
                    } else {
                        let text = v["params"]["arguments"]["text"].as_str().unwrap_or("pong");
                        json!({"content":[{"type":"text","text":text}],"structuredContent":{"text":text},"isError":mode == "tool-error","_meta":{"fixture":true}})
                    }
                }
                _ => json!({}),
            };
            if response.get("error").is_some() {
                response.as_object_mut().unwrap().remove("result");
            }
            let mut bytes = serde_json::to_vec(&response).unwrap();
            bytes.push(b'\n');
            let mut output = output.lock().await;
            let _ = output.write_all(&bytes).await;
            let _ = output.flush().await;
        });
    }
    if args.iter().any(|s| s == "--linger-on-eof") {
        tokio::time::sleep(Duration::from_secs(60)).await;
    }
}
