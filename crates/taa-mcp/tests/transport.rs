use axum::{
    body::{to_bytes, Body},
    http::{Request, StatusCode},
    Router,
};
use serde_json::{json, Value};
use std::sync::Arc;
use taa_mcp::{router, Server, MODERN, STATEFUL};
use taa_platform::MockPlatform;
use taa_tools::config::Config;
use tower::ServiceExt;
fn app() -> Router {
    router(Server::new(
        Config::default(),
        Arc::new(MockPlatform::default()),
    ))
}
async fn request(
    app: &Router,
    method: &str,
    mut body: Value,
    headers: &[(&str, &str)],
) -> (StatusCode, axum::http::HeaderMap, String) {
    if headers
        .iter()
        .any(|(k, v)| *k == "mcp-protocol-version" && *v == MODERN)
        && body.is_object()
    {
        if body.get("params").is_none() {
            body["params"] = json!({});
        }
        if body["params"].is_object() {
            body["params"]["_meta"] = json!({"io.modelcontextprotocol/protocolVersion":MODERN,"io.modelcontextprotocol/clientCapabilities":{}});
        }
    }
    let mut b = Request::builder()
        .method(method)
        .uri("/mcp")
        .header("host", "localhost:8765")
        .header("content-type", "application/json")
        .header("accept", "application/json, text/event-stream");
    if headers
        .iter()
        .any(|(k, v)| *k == "mcp-protocol-version" && *v == MODERN)
    {
        if let Some(m) = body["method"].as_str() {
            b = b.header("mcp-method", m);
        }
        if let Some(n) = body["params"]["name"].as_str() {
            b = b.header("mcp-name", n);
        }
    }
    for (k, v) in headers {
        b = b.header(*k, *v);
    }
    let r = app
        .clone()
        .oneshot(b.body(Body::from(body.to_string())).unwrap())
        .await
        .unwrap();
    let status = r.status();
    let h = r.headers().clone();
    let text =
        String::from_utf8(to_bytes(r.into_body(), usize::MAX).await.unwrap().to_vec()).unwrap();
    (status, h, text)
}
fn call(n: &str, a: Value) -> Value {
    json!({"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":format!("agentaccess___{n}"),"arguments":a}})
}
fn init(v: &str) -> Value {
    json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":v,"capabilities":{},"clientInfo":{"name":"test","version":"1"}}})
}
fn value(s: &str) -> Value {
    serde_json::from_str(s).unwrap()
}
#[tokio::test]
async fn versions_and_lifecycle() {
    for v in STATEFUL.into_iter().chain(["unsupported"]) {
        let a = app();
        let (status, h, b) = request(&a, "POST", init(v), &[]).await;
        assert_eq!(status, 200);
        assert_eq!(
            value(&b)["result"]["protocolVersion"],
            if v == "unsupported" { STATEFUL[2] } else { v }
        );
        let sid = h["mcp-session-id"].to_str().unwrap();
        let hdr = [("mcp-session-id", sid)];
        let (st, _, body) = request(&a, "POST", call("wait", json!({"duration":0.001})), &[]).await;
        assert_eq!(st, 400);
        assert_eq!(body, "Missing Mcp-Session-Id header");
        assert_eq!(
            request(
                &a,
                "POST",
                json!({"jsonrpc":"2.0","method":"notifications/initialized"}),
                &hdr
            )
            .await
            .0,
            202
        );
        assert!(
            value(&request(&a, "POST", init(v), &[]).await.2)["error"]["message"]
                .as_str()
                .unwrap()
                .contains("CONNECTION_LIMIT_REACHED")
        );
        assert_eq!(
            value(
                &request(&a, "POST", call("wait", json!({"duration":0.001})), &hdr)
                    .await
                    .2
            )["result"]["isError"],
            false
        );
        assert_eq!(request(&a, "DELETE", Value::Null, &hdr).await.0, 204);
        assert_eq!(
            request(&a, "POST", call("wait", json!({"duration":0.001})), &hdr)
                .await
                .0,
            404
        );
        assert_eq!(request(&a, "DELETE", Value::Null, &hdr).await.0, 404);
    }
}
#[tokio::test]
async fn modern_lifecycle() {
    let a = app();
    let h = [("mcp-protocol-version", MODERN)];
    let d = request(
        &a,
        "POST",
        json!({"jsonrpc":"2.0","id":1,"method":"server/discover"}),
        &h,
    )
    .await;
    assert!(value(&d.2)["result"]["supportedVersions"]
        .as_array()
        .unwrap()
        .contains(&json!(MODERN)));
    assert!(!d.1.contains_key("mcp-session-id"));
    let r = value(
        &request(
            &a,
            "POST",
            call("get_session_info", json!({"mcpSessionId":"NEW"})),
            &h,
        )
        .await
        .2,
    );
    let sid = r["result"]["mcpSessionId"].as_str().unwrap();
    assert_eq!(
        value(r["result"]["content"][1]["text"].as_str().unwrap())["mcpSessionId"],
        sid
    );
    assert!(value(
        &request(
            &a,
            "POST",
            call("wait", json!({"mcpSessionId":"NEW","duration":0.001})),
            &h
        )
        .await
        .2
    )["result"]["isError"]
        .as_bool()
        .unwrap());
    for n in ["wait", "screenshot", "unknown"] {
        let r = value(
            &request(&a, "POST", call(n, json!({"mcpSessionId":sid})), &h)
                .await
                .2,
        );
        assert_eq!(r["result"]["isError"], true);
        assert_eq!(r["result"]["mcpSessionId"], sid);
    }
    for _ in 0..2 {
        let r = value(
            &request(
                &a,
                "POST",
                call(
                    "close_session",
                    json!({"mcpSessionId":sid,"expire_streaming_session":true}),
                ),
                &h,
            )
            .await
            .2,
        );
        assert_eq!(r["result"]["isError"], false);
        assert_eq!(r["result"]["mcpSessionId"], sid);
    }
    assert_eq!(
        value(
            &request(
                &a,
                "POST",
                call("wait", json!({"mcpSessionId":"unknown","duration":0.001})),
                &h
            )
            .await
            .2
        )["result"]["isError"],
        true
    );
    assert_eq!(request(&a, "DELETE", Value::Null, &h).await.0, 405);
}
#[tokio::test]
async fn interfaces_and_security() {
    let a = app();
    let list = json!({"jsonrpc":"2.0","id":1,"method":"tools/list"});
    for interface in [
        "",
        "OPENAI_COMPUTER_V1",
        "ANTHROPIC_COMPUTER_20250124",
        "ANTHROPIC_COMPUTER_20251124",
        "ANTHROPIC_COMPUTER_TOOLSET_20260801",
        "GOOGLE_COMPUTER_USE_3X_DESKTOP_V1",
        "bad",
    ] {
        let r = value(
            &request(
                &a,
                "POST",
                list.clone(),
                &[
                    ("mcp-protocol-version", MODERN),
                    ("x-amzn-agentaccess-computer-use-interface-id", interface),
                ],
            )
            .await
            .2,
        );
        if interface.is_empty() {
            assert!(r["result"]["tools"].is_array());
        } else {
            assert_eq!(
                r["error"]["code"],
                if interface == "bad" { -32602 } else { -32603 }
            );
        }
    }
    for origin in ["https://evil.example", "null", "http://localhost.evil"] {
        assert_eq!(
            request(&a, "POST", list.clone(), &[("origin", origin)])
                .await
                .0,
            403
        );
    }
    assert_eq!(
        request(&a, "POST", list.clone(), &[("host", "evil.example")])
            .await
            .0,
        403
    );
    assert_eq!(request(&a, "GET", Value::Null, &[]).await.0, 405);
}
#[tokio::test]
async fn rpc_errors() {
    let a = app();
    for (body, code) in [
        (json!([]), -32600),
        (json!({"id":1,"method":"ping"}), -32600),
        (json!({"jsonrpc":"2.0","id":1,"method":"other"}), -32601),
    ] {
        assert_eq!(
            value(
                &request(&a, "POST", body, &[("mcp-protocol-version", MODERN)])
                    .await
                    .2
            )["error"]["code"],
            code
        );
    }
}
#[tokio::test]
async fn idle_expiry() {
    let mut c = Config::default();
    c.sessions.idle_timeout_secs = 1;
    let a = router(Server::new(c, Arc::new(MockPlatform::default())));
    let (_, h, _) = request(&a, "POST", init(STATEFUL[2]), &[]).await;
    tokio::time::sleep(std::time::Duration::from_millis(1010)).await;
    assert_eq!(
        request(
            &a,
            "POST",
            call("wait", json!({"duration":0.001})),
            &[("mcp-session-id", h["mcp-session-id"].to_str().unwrap())]
        )
        .await
        .0,
        404
    );
}

#[tokio::test]
async fn march_batches() {
    let a = app();
    let (_, h, _) = request(&a, "POST", init(STATEFUL[0]), &[]).await;
    let hdr = [("mcp-session-id", h["mcp-session-id"].to_str().unwrap())];
    let r=value(&request(&a,"POST",json!([{"jsonrpc":"2.0","id":2,"method":"ping"},{"jsonrpc":"2.0","method":"notifications/initialized"},call("wait",json!({"duration":0.001}))]),&hdr).await.2);
    assert_eq!(r.as_array().unwrap().len(), 2);
    assert_eq!(r[1]["result"]["isError"], false);
}

#[tokio::test]
async fn protocol_validation() {
    let a = app();
    let (_, h, _) = request(&a, "POST", init(STATEFUL[2]), &[]).await;
    let sid = h["mcp-session-id"].to_str().unwrap();
    assert_eq!(
        request(
            &a,
            "POST",
            call("wait", json!({"duration":0.001})),
            &[("mcp-session-id", sid), ("mcp-protocol-version", "bad")]
        )
        .await
        .0,
        400
    );
    let r = value(
        &request(
            &a,
            "POST",
            json!({"jsonrpc":"2.0","id":1,"method":"server/discover"}),
            &[("mcp-protocol-version", "2099-01-01")],
        )
        .await
        .2,
    );
    assert_eq!(r["error"]["code"], -32022);
    assert_eq!(r["error"]["data"]["supported"], json!([MODERN]));
    assert_eq!(
        request(
            &a,
            "POST",
            json!({"jsonrpc":"2.0","id":1,"method":"ping"}),
            &[
                ("mcp-protocol-version", STATEFUL[0]),
                ("mcp-session-id", sid)
            ]
        )
        .await
        .0,
        400
    );
    let req = Request::builder()
        .method("POST")
        .uri("/mcp")
        .header("host", "localhost")
        .header("content-type", "application/json")
        .header("accept", "application/json")
        .body(Body::from("{"))
        .unwrap();
    let r = a.oneshot(req).await.unwrap();
    assert_eq!(
        value(std::str::from_utf8(&to_bytes(r.into_body(), usize::MAX).await.unwrap()).unwrap())
            ["error"]["code"],
        -32700
    );
}

#[tokio::test]
async fn gated_calls_are_unknown() {
    let mut c = Config::default();
    c.agent_access.computer_input = false;
    c.agent_access.computer_vision = false;
    let a = router(Server::new(c, Arc::new(MockPlatform::recording())));
    let (_, h, _) = request(&a, "POST", init(STATEFUL[2]), &[]).await;
    for n in ["screenshot", "key", "execute_sequence", "close_session"] {
        let r = value(
            &request(
                &a,
                "POST",
                call(n, json!({})),
                &[("mcp-session-id", h["mcp-session-id"].to_str().unwrap())],
            )
            .await
            .2,
        );
        assert_eq!(
            r["result"]["content"][0]["text"],
            "CLIENT_ERROR: Unknown tool"
        );
    }
}

#[tokio::test]
async fn rpc_errors_preserve_desktop_id() {
    let a = app();
    let h = [("mcp-protocol-version", MODERN)];
    let r = value(
        &request(
            &a,
            "POST",
            call("get_session_info", json!({"mcpSessionId":"NEW"})),
            &h,
        )
        .await
        .2,
    );
    let sid = r["result"]["mcpSessionId"].as_str().unwrap();
    let r = value(
        &request(
            &a,
            "POST",
            call("wait", json!({"mcpSessionId":sid,"duration":0.001})),
            &[
                ("mcp-protocol-version", MODERN),
                ("x-amzn-agentaccess-computer-use-interface-id", "bad"),
            ],
        )
        .await
        .2,
    );
    assert_eq!(r["error"]["code"], -32602);
    assert_eq!(r["error"]["data"]["mcpSessionId"], sid);
}

// Raw requests deliberately bypass the happy-path helper's metadata/header injection.
async fn raw(a: &Router, body: impl Into<Body>, headers: &[(&str, &str)]) -> (StatusCode, Value) {
    let mut b = Request::builder().method("POST").uri("/mcp");
    for (k, v) in headers {
        b = b.header(*k, *v);
    }
    let r = a
        .clone()
        .oneshot(b.body(body.into()).unwrap())
        .await
        .unwrap();
    let status = r.status();
    let bytes = to_bytes(r.into_body(), usize::MAX).await.unwrap();
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    )
}
const BASE: &[(&str, &str)] = &[
    ("host", "localhost"),
    ("content-type", "application/json"),
    ("accept", "application/json"),
];

#[tokio::test]
async fn modern_statuses_and_envelopes() {
    let a = app();
    let mut h = BASE.to_vec();
    h.push(("mcp-protocol-version", MODERN));
    for (body, code) in [
        ("{", -32700),
        (r#"{"jsonrpc":"2.0","id":{},"method":"ping"}"#, -32600),
        (
            r#"{"jsonrpc":"2.0","id":1,"method":"ping","params":[]}"#,
            -32602,
        ),
        (r#"{"jsonrpc":"2.0","id":1,"method":"ping"}"#, -32602),
    ] {
        let (st, v) = raw(&a, body, &h).await;
        assert_eq!(st, 400);
        assert_eq!(v["error"]["code"], code);
        if code == -32600 {
            assert!(v["id"].is_null());
        }
    }
    let (st, _, b) = request(
        &a,
        "POST",
        json!({"jsonrpc":"2.0","id":1,"method":"unknown"}),
        &[("mcp-protocol-version", MODERN)],
    )
    .await;
    assert_eq!(st, 404);
    assert_eq!(value(&b)["error"]["code"], -32601);
    for method in ["ping", "tools/call", "tools/list", "server/discover"] {
        let (st, _, b) = request(
            &a,
            "POST",
            json!({"jsonrpc":"2.0","id":1,"method":method}),
            &[("mcp-protocol-version", "2099-01-01")],
        )
        .await;
        assert_eq!(st, 400);
        assert_eq!(value(&b)["error"]["code"], -32022);
        assert_eq!(value(&b)["error"]["data"]["requested"], "2099-01-01");
    }
    let (st, v) = raw(
        &a,
        r#"{"jsonrpc":"2.0","method":"notifications/cancelled"}"#,
        &h,
    )
    .await;
    assert_eq!(st, 202);
    assert!(v.is_null());
    assert_eq!(
        request(
            &a,
            "POST",
            json!({"jsonrpc":"2.0","id":1,"method":"ping"}),
            &[("mcp-protocol-version", MODERN)]
        )
        .await
        .0,
        200
    );
}

#[tokio::test]
async fn bounded_bodies_and_batches() {
    let a = app();
    assert_eq!(
        raw(&a, " ".repeat(taa_mcp::MAX_BODY_BYTES + 1), BASE)
            .await
            .0,
        413
    );
    let nested = format!("{}{}", "[".repeat(200000), "]".repeat(200000));
    let (st, v) = raw(&a, nested, BASE).await;
    assert_eq!(st, 400);
    assert_eq!(v["error"]["code"], -32700);
    let (_, h, _) = request(&a, "POST", init(STATEFUL[0]), &[]).await;
    let (_,_,b) = request(&a,"POST",json!([[{"jsonrpc":"2.0","id":1,"method":"ping"}],42,{"jsonrpc":"2.0","id":2,"method":"ping"}]),&[("mcp-session-id",h["mcp-session-id"].to_str().unwrap())]).await;
    let v = value(&b);
    assert_eq!(v[0]["error"]["code"], -32600);
    assert_eq!(v[1]["error"]["code"], -32600);
    assert_eq!(v[2]["result"], json!({}));
}

#[tokio::test]
async fn media_and_untrusted_headers() {
    let a = app();
    let body = init(STATEFUL[2]).to_string();
    for (key, val, status) in [
        ("host", "evil.example", 403),
        ("origin", "http://localhost", 403),
        ("content-type", "text/plain", 400),
        ("mcp-method", "ping", 400),
    ] {
        let mut h = BASE.to_vec();
        if key == "origin" {
            h.push((key, val));
            h.push((key, val));
        } else {
            h.push((key, val));
            if key == "mcp-method" {
                h.push((key, val));
            }
        }
        assert_eq!(raw(&a, body.clone(), &h).await.0, status);
    }
    for (h, st) in [
        (
            vec![("host", "localhost"), ("accept", "application/json")],
            415,
        ),
        (
            vec![
                ("host", "localhost"),
                ("content-type", "text/plain"),
                ("accept", "application/json"),
            ],
            415,
        ),
        (
            vec![("host", "localhost"), ("content-type", "application/json")],
            406,
        ),
        (
            vec![
                ("host", "localhost"),
                ("content-type", "application/json"),
                ("accept", "text/event-stream"),
            ],
            406,
        ),
    ] {
        assert_eq!(raw(&a, body.clone(), &h).await.0, st);
    }
    let req = Request::builder()
        .method("POST")
        .uri("/mcp")
        .header("host", "localhost")
        .header("content-type", "application/json")
        .header("accept", "*/*")
        .header(
            "mcp-method",
            axum::http::HeaderValue::from_bytes(&[0xff]).unwrap(),
        )
        .body(Body::from(body))
        .unwrap();
    assert_eq!(a.oneshot(req).await.unwrap().status(), 400);
}

#[tokio::test]
async fn concrete_bind_security() {
    let c = Config {
        bind: "192.0.2.10:8765".into(),
        ..Config::default()
    };
    c.validate().unwrap();
    let a = router(Server::new(c, Arc::new(MockPlatform::default())));
    let h = [
        ("host", "192.0.2.10:8765"),
        ("content-type", "application/json"),
        ("accept", "application/json"),
    ];
    assert_eq!(raw(&a, init(STATEFUL[2]).to_string(), &h).await.0, 200);
    let h = [
        ("host", "192.0.2.11:8765"),
        ("content-type", "application/json"),
        ("accept", "application/json"),
    ];
    assert_eq!(raw(&a, init(STATEFUL[2]).to_string(), &h).await.0, 403);
    for bind in ["0.0.0.0:8765", "[::]:8765"] {
        let c = Config {
            bind: bind.into(),
            ..Config::default()
        };
        assert!(c.validate().unwrap_err().contains("NO AUTHENTICATION"));
    }
}

#[tokio::test]
async fn request_ids_params_and_legacy_handler_status() {
    let a = app();
    let mut h = BASE.to_vec();
    h.push(("mcp-protocol-version", MODERN));
    for id in [Value::Null, json!(true), json!(1.5), json!([]), json!({})] {
        let (st, v) = raw(
            &a,
            json!({"jsonrpc":"2.0","id":id,"method":"ping"}).to_string(),
            &h,
        )
        .await;
        assert_eq!(st, 400);
        assert_eq!(v["error"]["code"], -32600);
        assert!(v["id"].is_null());
    }
    for params in [Value::Null, json!(42), json!("bad"), json!([])] {
        let (st, v) = raw(
            &a,
            json!({"jsonrpc":"2.0","id":1,"method":"ping","params":params}).to_string(),
            &h,
        )
        .await;
        assert_eq!(st, 400);
        assert_eq!(v["error"]["code"], -32602);
    }
    let (_, headers, _) = request(&a, "POST", init(STATEFUL[2]), &[]).await;
    let (st, _, b) = request(
        &a,
        "POST",
        json!({"jsonrpc":"2.0","id":1,"method":"unknown"}),
        &[(
            "mcp-session-id",
            headers["mcp-session-id"].to_str().unwrap(),
        )],
    )
    .await;
    assert_eq!(st, 200);
    assert_eq!(value(&b)["error"]["code"], -32601);
    h.push(("mcp-method", "ping"));
    h.push(("mcp-method", "ping"));
    let (st, v) = raw(
        &a,
        json!({"jsonrpc":"2.0","id":1,"method":"ping"}).to_string(),
        &h,
    )
    .await;
    assert_eq!(st, 400);
    assert_eq!(v["error"]["code"], -32020);
}

#[tokio::test]
async fn initialize_notifications_never_allocate_or_reply() {
    for version in STATEFUL.into_iter().chain([MODERN, "unsupported"]) {
        let a = app();
        for params in [init(version)["params"].clone(), json!({}), json!([])] {
            let notification = json!({"jsonrpc":"2.0","method":"initialize","params":params});
            for headers in [vec![], vec![("mcp-protocol-version", version)]] {
                let (status, headers, body) =
                    request(&a, "POST", notification.clone(), &headers).await;
                assert_eq!(status, StatusCode::ACCEPTED);
                assert!(!headers.contains_key("mcp-session-id"));
                assert!(body.is_empty());
            }
        }
        let (status, headers, body) = request(&a, "POST", init(STATEFUL[2]), &[]).await;
        assert_eq!(status, StatusCode::OK);
        assert!(headers.contains_key("mcp-session-id"));
        assert!(value(&body).get("result").is_some());
    }
}

#[tokio::test]
async fn delete_rejects_version_mismatch_without_closing_session() {
    for negotiated in STATEFUL {
        let a = app();
        let (_, headers, _) = request(&a, "POST", init(negotiated), &[]).await;
        let sid = headers["mcp-session-id"].to_str().unwrap();
        for other in STATEFUL.into_iter().filter(|v| *v != negotiated) {
            let (status, _, _) = request(
                &a,
                "DELETE",
                Value::Null,
                &[("mcp-session-id", sid), ("mcp-protocol-version", other)],
            )
            .await;
            assert_eq!(status, StatusCode::BAD_REQUEST);
            let (status, _, body) = request(
                &a,
                "POST",
                json!({"jsonrpc":"2.0","id":2,"method":"ping"}),
                &[
                    ("mcp-session-id", sid),
                    ("mcp-protocol-version", negotiated),
                ],
            )
            .await;
            assert_eq!(status, StatusCode::OK);
            assert_eq!(value(&body)["result"], json!({}));
        }
        assert_eq!(
            request(
                &a,
                "DELETE",
                Value::Null,
                &[
                    ("mcp-session-id", sid),
                    ("mcp-protocol-version", negotiated),
                ]
            )
            .await
            .0,
            StatusCode::NO_CONTENT
        );
        assert_eq!(
            request(&a, "DELETE", Value::Null, &[("mcp-session-id", sid)])
                .await
                .0,
            StatusCode::NOT_FOUND
        );
    }
}

#[tokio::test]
async fn modern_sequence_results_and_close_release() {
    for available in [true, false] {
        let p = Arc::new(if available {
            MockPlatform::recording()
        } else {
            MockPlatform::default()
        });
        let a = router(Server::new(Config::default(), p.clone()));
        let h = [("mcp-protocol-version", MODERN)];
        let r = value(
            &request(
                &a,
                "POST",
                call("get_session_info", json!({"mcpSessionId":"NEW"})),
                &h,
            )
            .await
            .2,
        );
        let sid = r["result"]["mcpSessionId"].as_str().unwrap();
        for (actions, error) in [
            (
                json!([{"type":"key","keys":"a"},{"type":"wait","duration":0.001}]),
                !available,
            ),
            (json!([{"type":"wait","duration":0}]), true),
        ] {
            let r = value(
                &request(
                    &a,
                    "POST",
                    call(
                        "execute_sequence",
                        json!({"mcpSessionId":sid,"actions":actions}),
                    ),
                    &h,
                )
                .await
                .2,
            );
            assert_eq!(r["result"]["isError"], error);
            let blocks = r["result"]["content"].as_array().unwrap();
            assert_eq!(
                value(blocks.last().unwrap()["text"].as_str().unwrap()),
                json!({"mcpSessionId":sid})
            );
        }
        if available {
            let r = value(
                &request(
                    &a,
                    "POST",
                    call("left_mouse_down", json!({"mcpSessionId":sid,"x":10,"y":20})),
                    &h,
                )
                .await
                .2,
            );
            assert_eq!(r["result"]["isError"], false);
        }
        for _ in 0..2 {
            let r = value(
                &request(
                    &a,
                    "POST",
                    call("close_session", json!({"mcpSessionId":sid})),
                    &h,
                )
                .await
                .2,
            );
            assert_eq!(r["result"]["isError"], false);
            assert_eq!(
                value(
                    r["result"]["content"].as_array().unwrap().last().unwrap()["text"]
                        .as_str()
                        .unwrap()
                ),
                json!({"mcpSessionId":sid})
            );
        }
        if available {
            assert!(matches!(
                p.operations.lock().unwrap().last().unwrap(),
                taa_platform::Operation::ReleaseLeft
            ));
        }
        let r = value(
            &request(
                &a,
                "POST",
                call("wait", json!({"mcpSessionId":sid,"duration":0.001})),
                &h,
            )
            .await
            .2,
        );
        assert_eq!(r["result"]["isError"], true);
    }
}

#[tokio::test]
async fn transport_saves_standalone_and_terminal_images() {
    let tmp = tempfile::tempdir().unwrap();
    let mut c = Config::default();
    c.screenshots.save_dir = tmp.path().canonicalize().unwrap().to_str().unwrap().into();
    let a = router(Server::new(c, Arc::new(MockPlatform::recording())));
    let h = [("mcp-protocol-version", MODERN)];
    let r = value(
        &request(
            &a,
            "POST",
            call("screenshot", json!({"mcpSessionId":"NEW"})),
            &h,
        )
        .await
        .2,
    );
    assert_eq!(r["result"]["isError"], false);
    let sid = r["result"]["mcpSessionId"].as_str().unwrap();
    let r = value(
        &request(
            &a,
            "POST",
            call(
                "execute_sequence",
                json!({"mcpSessionId":sid,"actions":[{"type":"wait","duration":0.001}]}),
            ),
            &h,
        )
        .await
        .2,
    );
    assert_eq!(r["result"]["isError"], false);
    fn count(path: &std::path::Path) -> usize {
        std::fs::read_dir(path)
            .unwrap()
            .map(|e| {
                let p = e.unwrap().path();
                if p.is_dir() {
                    count(&p)
                } else {
                    1
                }
            })
            .sum()
    }
    assert_eq!(count(tmp.path()), 2);
}

#[tokio::test]
async fn forwarding_transport_modes() {
    for modern in [false, true] {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("redirect.json");
        let pid_path = dir.path().join("pid");
        std::fs::write(&path,json!({"mcpServers":{"fixture":{"command":env!("CARGO_BIN_EXE_taa-mcp-forward-fixture"),"args":["good","--pid-file",pid_path]}}}).to_string()).unwrap();
        let mut config = Config::default();
        config.agent_access.computer_vision = false;
        config.agent_access.computer_input = false;
        config.agent_access.forward_mcp_tools = true;
        config.forwarding.config_path = path.to_str().unwrap().into();
        config.validate().unwrap();
        let server = Server::new(config, Arc::new(MockPlatform::default()));
        let a = router(server.clone());
        let sid = if modern {
            String::new()
        } else {
            request(&a, "POST", init(STATEFUL[1]), &[]).await.1["mcp-session-id"]
                .to_str()
                .unwrap()
                .to_owned()
        };
        let headers = if modern {
            vec![("mcp-protocol-version", MODERN)]
        } else {
            vec![("mcp-session-id", sid.as_str())]
        };
        let (_, _, body) = request(
            &a,
            "POST",
            json!({"jsonrpc":"2.0","id":1,"method":"tools/list"}),
            &headers,
        )
        .await;
        let listed = value(&body);
        let tools = listed["result"]["tools"].as_array().unwrap();
        assert!(!tools
            .iter()
            .any(|t| t["name"] == "agentaccess___screenshot"));
        let echo = tools
            .iter()
            .find(|t| t["name"] == "forwarded___fixture___echo")
            .unwrap();
        assert_eq!(echo["annotations"]["readOnlyHint"], true);
        assert_eq!(echo["outputSchema"]["type"], "object");
        assert_eq!(
            echo["inputSchema"]["properties"]["mcpSessionId"].is_object(),
            modern
        );
        let args = if modern {
            json!({"mcpSessionId":"NEW","text":"relay"})
        } else {
            json!({"text":"relay"})
        };
        let (_,_,body) = request(&a,"POST",json!({"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"forwarded___fixture___echo","arguments":args}}),&headers).await;
        let result = value(&body);
        assert_eq!(result["result"]["isError"], false);
        assert_eq!(result["result"]["content"][0]["text"], "relay");
        assert_eq!(result["result"]["_meta"]["fixture"], true);
        let session = if modern {
            result["result"]["mcpSessionId"]
                .as_str()
                .unwrap()
                .to_owned()
        } else {
            sid.clone()
        };
        let pid: u32 = std::fs::read_to_string(&pid_path).unwrap().parse().unwrap();
        if modern {
            let (_, _, body) = request(
                &a,
                "POST",
                call("close_session", json!({"mcpSessionId":session})),
                &headers,
            )
            .await;
            assert_eq!(value(&body)["result"]["isError"], false);
        } else {
            assert_eq!(request(&a, "DELETE", Value::Null, &headers).await.0, 204);
        }
        wait_for_process_exit(pid).await;
        server.shutdown().await;
    }
}
#[tokio::test]
async fn forwarding_disabled_is_unknown() {
    let a = app();
    let (_, h, _) = request(&a, "POST", init(STATEFUL[1]), &[]).await;
    let headers = [("mcp-session-id", h["mcp-session-id"].to_str().unwrap())];
    let (_,_,body) = request(&a,"POST",json!({"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"forwarded___fixture___echo"}}),&headers).await;
    assert_eq!(
        value(&body)["result"]["content"][0]["text"],
        "CLIENT_ERROR: Unknown tool"
    );
    let (_, _, body) = request(
        &a,
        "POST",
        json!({"jsonrpc":"2.0","id":1,"method":"tools/list"}),
        &headers,
    )
    .await;
    assert!(!value(&body)["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .any(|t| t["name"].as_str().unwrap().starts_with("forwarded___")));
}

fn forwarding_app(mode: &str) -> (tempfile::TempDir, Arc<Server>, Router) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("forward.json");
    std::fs::write(&path,json!({"mcpServers":{"fixture":{"command":env!("CARGO_BIN_EXE_taa-mcp-forward-fixture"),"args":[mode,"--pid-file",dir.path().join("pid")]}}}).to_string()).unwrap();
    let mut config = Config::default();
    config.agent_access.forward_mcp_tools = true;
    config.sessions.idle_timeout_secs = 1;
    config.forwarding.config_path = path.to_str().unwrap().into();
    let server = Server::new(config, Arc::new(MockPlatform::default()));
    let app = router(server.clone());
    (dir, server, app)
}
#[tokio::test]
async fn forwarding_errors_and_interface_independence() {
    for modern in [false, true] {
        for (mode, code, expected) in [
            ("error-call", -32602, "CLIENT_ERROR: fixture error"),
            ("error-call", -32603, "SERVER_ERROR: fixture error"),
            (
                "crash-on-call",
                0,
                "SERVER_ERROR: forwarded server 'fixture' is unavailable",
            ),
            ("tool-error", 0, "untouched"),
        ] {
            let (_dir, server, a) = forwarding_app(mode);
            let sid = if modern {
                String::new()
            } else {
                request(&a, "POST", init(STATEFUL[1]), &[]).await.1["mcp-session-id"]
                    .to_str()
                    .unwrap()
                    .into()
            };
            let mut headers = if modern {
                vec![("mcp-protocol-version", MODERN)]
            } else {
                vec![("mcp-session-id", sid.as_str())]
            };
            headers.push((
                "x-amzn-agentaccess-computer-use-interface-id",
                "ANTHROPIC_COMPUTER_20250124",
            ));
            let (_, _, body) = request(
                &a,
                "POST",
                json!({"jsonrpc":"2.0","id":1,"method":"tools/list"}),
                &headers,
            )
            .await;
            assert!(value(&body)["result"]["tools"]
                .as_array()
                .unwrap()
                .iter()
                .any(|t| t["name"] == "forwarded___fixture___echo"));
            let args = if modern {
                json!({"mcpSessionId":"NEW","text":"untouched","code":code})
            } else {
                json!({"text":"untouched","code":code})
            };
            let (_,_,body) = request(&a,"POST",json!({"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"forwarded___fixture___echo","arguments":args}}),&headers).await;
            let reply = value(&body);
            assert_eq!(reply["result"]["isError"], true);
            assert_eq!(reply["result"]["content"][0]["text"], expected);
            if modern {
                assert!(reply["result"]["mcpSessionId"].is_string());
                assert_eq!(reply["result"]["content"].as_array().unwrap().len(), 2);
            }
            server.shutdown().await;
        }
    }
}
#[tokio::test]
async fn forwarding_does_not_wait_for_desktop() {
    let (_dir, server, a) = forwarding_app("good");
    let (_, h, _) = request(&a, "POST", init(STATEFUL[1]), &[]).await;
    let sid = h["mcp-session-id"].to_str().unwrap().to_owned();
    let waiting = {
        let a = a.clone();
        let sid = sid.clone();
        tokio::spawn(async move {
            request(
                &a,
                "POST",
                call("wait", json!({"duration":1})),
                &[("mcp-session-id", &sid)],
            )
            .await
        })
    };
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    let (_,_,body) = tokio::time::timeout(std::time::Duration::from_millis(500),request(&a,"POST",json!({"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"forwarded___fixture___ping"}}),&[("mcp-session-id",&sid)])).await.unwrap();
    assert_eq!(value(&body)["result"]["isError"], false);
    assert!(!waiting.is_finished());
    waiting.await.unwrap();
    server.shutdown().await;
}
#[test]
fn forwarding_config_gating() {
    assert!(Config::parse(
        "[agent_access]\ncomputer_vision=false\ncomputer_input=false\nforward_mcp_tools=true"
    )
    .is_ok());
    assert!(Config::parse(
        "[agent_access]\ncomputer_vision=false\ncomputer_input=false\nforward_mcp_tools=false"
    )
    .is_err());
    assert!(Config::parse(
        "[agent_access]\ncomputer_vision=false\ncomputer_input=true\nforward_mcp_tools=true"
    )
    .is_err());
    assert!(Config::parse("[forwarding]\nconfig_path='relative.json'").is_err());
}

fn process_is_running(pid: u32) -> bool {
    #[cfg(unix)]
    {
        std::process::Command::new("kill")
            .args(["-0", &pid.to_string()])
            .stderr(std::process::Stdio::null())
            .status()
            .unwrap()
            .success()
    }
    #[cfg(windows)]
    {
        String::from_utf8_lossy(
            &std::process::Command::new("tasklist")
                .args(["/FI", &format!("PID eq {pid}"), "/FO", "CSV", "/NH"])
                .output()
                .unwrap()
                .stdout,
        )
        .lines()
        .any(|line| {
            line.split(',')
                .nth(1)
                .is_some_and(|field| field.trim_matches('"') == pid.to_string())
        })
    }
}
async fn wait_for_process_exit(pid: u32) {
    tokio::time::timeout(std::time::Duration::from_secs(4), async {
        loop {
            if !process_is_running(pid) {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
    })
    .await
    .unwrap();
}
#[tokio::test]
async fn forwarding_idle_expiry_and_shutdown_reap_children() {
    for expire in [false, true] {
        let (dir, server, a) = forwarding_app("good");
        let (_, h, _) = request(&a, "POST", init(STATEFUL[1]), &[]).await;
        let headers = [("mcp-session-id", h["mcp-session-id"].to_str().unwrap())];
        request(
            &a,
            "POST",
            json!({"jsonrpc":"2.0","id":1,"method":"tools/list"}),
            &headers,
        )
        .await;
        let pid: u32 = std::fs::read_to_string(dir.path().join("pid"))
            .unwrap()
            .parse()
            .unwrap();
        assert!(process_is_running(pid));
        if !expire {
            server.shutdown().await;
        }
        wait_for_process_exit(pid).await;
        server.shutdown().await;
    }
}

#[tokio::test]
async fn disabled_forwarding_validates_interface_for_forwarded_name() {
    let (_, _, body) = request(
        &app(),
        "POST",
        json!({"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"forwarded___x___y"}}),
        &[
            ("mcp-protocol-version", MODERN),
            ("x-amzn-agentaccess-computer-use-interface-id", "bad"),
        ],
    )
    .await;
    assert_eq!(value(&body)["error"]["code"], -32602);
}
struct GrownDisplay {
    mock: MockPlatform,
    refreshes: std::sync::atomic::AtomicUsize,
}
impl taa_platform::Platform for GrownDisplay {
    fn name(&self) -> &str {
        "grown-display"
    }
    fn screen_size(&self) -> (u32, u32) {
        (1280, 720)
    }
    fn capture_geometry(&self) -> Result<(u32, u32), String> {
        self.refreshes
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        Ok((1920, 1080))
    }
    fn capture(&self) -> Result<taa_platform::Frame, String> {
        panic!("click must not capture a screenshot")
    }
    fn perform(&self, op: taa_platform::Operation) -> Result<(), String> {
        taa_platform::Platform::perform(&self.mock, op)
    }
}

#[tokio::test]
async fn native_http_click_validates_after_geometry_refresh() {
    for (args, error, refreshes) in [
        (json!({"x":1500,"y":500}), None, 1),
        (
            json!({"x":1920,"y":500}),
            Some("x outside screen bounds"),
            1,
        ),
        (
            json!({"x":1500,"y":1080}),
            Some("y outside screen bounds"),
            1,
        ),
        (json!({"x":"1500","y":500}), Some("invalid x"), 0),
    ] {
        let mut config = Config::default();
        config.agent_access.screen_resolution = "native".into();
        let platform = Arc::new(GrownDisplay {
            mock: MockPlatform::recording(),
            refreshes: std::sync::atomic::AtomicUsize::new(0),
        });
        let app = router(Server::new(config, platform.clone()));
        let (status, headers, _) = request(&app, "POST", init(STATEFUL[2]), &[]).await;
        assert_eq!(status, StatusCode::OK);
        let sid = headers["mcp-session-id"].to_str().unwrap();
        let (status, _, body) = request(
            &app,
            "POST",
            call("left_click", args),
            &[("mcp-session-id", sid)],
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        let result = &value(&body)["result"];
        assert_eq!(result["isError"], error.is_some(), "{body}");
        assert_eq!(
            platform.refreshes.load(std::sync::atomic::Ordering::SeqCst),
            refreshes
        );
        let operations = platform.mock.operations.lock().unwrap();
        if let Some(error) = error {
            assert!(
                result["content"][0]["text"]
                    .as_str()
                    .unwrap()
                    .contains(error),
                "{body}"
            );
            assert!(operations.is_empty());
        } else {
            assert_eq!(
                operations.as_slice(),
                &[taa_platform::Operation::Click(
                    taa_platform::Point { x: 1500, y: 500 },
                    taa_platform::Button::Left,
                    1,
                    vec![],
                )]
            );
        }
    }
}
