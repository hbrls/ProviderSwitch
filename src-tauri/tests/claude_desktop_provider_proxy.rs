//! Exercise Provider proxies through the real Claude Desktop Routing gateway.

use std::sync::Arc;
use std::time::Duration;

use axum::{body::Bytes, http::StatusCode, Json, Router};
use cc_switch_lib::{
    live::engine::DeviceStore,
    mode::state::{self, Mode},
    Database, Provider, ProxyService,
};
use serde_json::{json, Value};
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
    net::{TcpListener, TcpStream},
    sync::{Barrier, Mutex, Notify},
};

const GATEWAY_TOKEN: &str = "desktop-proxy-integration-token";

fn route_to(id: &str) {
    state::update(&DeviceStore::for_device(), |live| {
        let desktop = live.apps.entry("claude-desktop".into()).or_default();
        desktop.mode = Some(Mode::Proxy);
        desktop.attached = true;
        desktop.proxy_route = Some(id.to_string());
    })
    .unwrap();
}

fn provider(id: &str, upstream: &str, proxy: Option<&str>) -> Provider {
    let mut row = Provider::with_id(
        id.into(),
        id.into(),
        json!({"env": {
            "ANTHROPIC_BASE_URL": upstream,
            "ANTHROPIC_AUTH_TOKEN": "upstream-secret"
        }}),
        None,
    );
    row.meta = Some(
        serde_json::from_value(json!({
            "apiFormat": "anthropic",
            "claudeDesktopMode": "proxy",
            "claudeDesktopProxyUrl": proxy,
            "claudeDesktopModelRoutes": {
                "claude-sonnet-5": {"model": id}
            }
        }))
        .unwrap(),
    );
    row
}

fn message() -> Value {
    json!({
        "model": "claude-sonnet-5",
        "max_tokens": 128,
        "messages": [{"role": "user", "content": "hello"}]
    })
}

fn completion() -> Value {
    json!({
        "id": "msg_proxy_test",
        "type": "message",
        "role": "assistant",
        "model": "claude-sonnet-5",
        "content": [{"type": "text", "text": "hello"}],
        "stop_reason": "end_turn",
        "usage": {"input_tokens": 1, "output_tokens": 1}
    })
}

fn request(client: &reqwest::Client, gateway: &str) -> reqwest::RequestBuilder {
    client
        .post(format!("{gateway}/claude-desktop/v1/messages"))
        .header("x-api-key", GATEWAY_TOKEN)
        .header("anthropic-version", "2023-06-01")
        .json(&message())
}

/// A real HTTP CONNECT tunnel that records the unmodified outbound request.
async fn connect_proxy(
    reject: bool,
    accepted: Arc<Notify>,
    both_connected: Option<Arc<Barrier>>,
) -> (String, tokio::task::JoinHandle<String>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let task = tokio::spawn(async move {
        let (socket, _) = listener.accept().await.unwrap();
        let mut socket = BufReader::new(socket);
        let mut line = String::new();
        socket.read_line(&mut line).await.unwrap();
        assert!(line.starts_with("CONNECT "), "{line}");
        let target = line.split_whitespace().nth(1).unwrap().to_string();
        loop {
            let mut header = String::new();
            assert!(socket.read_line(&mut header).await.unwrap() > 0);
            if header == "\r\n" {
                break;
            }
        }
        accepted.notify_one();
        if let Some(barrier) = both_connected {
            barrier.wait().await;
        }
        if reject {
            socket
                .get_mut()
                .write_all(b"HTTP/1.1 502 Bad Gateway\r\nContent-Length: 0\r\n\r\n")
                .await
                .unwrap();
            return target;
        }
        let mut upstream = TcpStream::connect(&target).await.unwrap();
        socket
            .get_mut()
            .write_all(b"HTTP/1.1 200 Connection Established\r\n\r\n")
            .await
            .unwrap();
        let _ = tokio::io::copy_bidirectional(socket.get_mut(), &mut upstream).await;
        target
    });
    (url, task)
}

#[tokio::test]
async fn desktop_routing_uses_each_providers_proxy() {
    // This binary has one test; isolate all device state from the user's configuration.
    let task_home = tempfile::tempdir().unwrap();
    std::env::set_var("CC_SWITCH_TEST_HOME", task_home.path());
    let _ = rustls::crypto::ring::default_provider().install_default();

    let seen = Arc::new(Mutex::new(Vec::<Value>::new()));
    let captured = seen.clone();
    let app = Router::new().fallback(move |body: Bytes| {
        let captured = captured.clone();
        async move {
            captured
                .lock()
                .await
                .push(serde_json::from_slice(&body).unwrap());
            Json(completion())
        }
    });
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let target = listener.local_addr().unwrap().to_string();
    let upstream_url = format!("http://{target}");
    let upstream_task = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });

    let db = Arc::new(Database::memory().unwrap());
    db.set_setting("claude_desktop_gateway_token", GATEWAY_TOKEN)
        .unwrap();
    let mut config = db.get_global_proxy_config().await.unwrap();
    config.listen_address = "127.0.0.1".into();
    config.listen_port = 0;
    db.update_global_proxy_config(config).await.unwrap();
    let service = ProxyService::new(db.clone());
    let gateway_info = service.start().await.unwrap();
    let gateway = format!("http://127.0.0.1:{}", gateway_info.port);
    let client = reqwest::Client::builder()
        .no_proxy()
        .timeout(Duration::from_secs(10))
        .build()
        .unwrap();

    // Hold A's request in its tunnel while switching the routing Provider to B.
    let a_accepted = Arc::new(Notify::new());
    let both_connected = Arc::new(Barrier::new(2));
    let (proxy_a, tunnel_a) =
        connect_proxy(false, a_accepted.clone(), Some(both_connected.clone())).await;
    let (proxy_b, tunnel_b) =
        connect_proxy(false, Arc::new(Notify::new()), Some(both_connected)).await;
    for (id, proxy) in [("a", &proxy_a), ("b", &proxy_b)] {
        db.save_provider("claude-desktop", &provider(id, &upstream_url, Some(proxy)))
            .unwrap();
        let saved = db
            .get_provider_by_id(id, "claude-desktop")
            .unwrap()
            .unwrap();
        assert_eq!(
            saved.meta.unwrap().claude_desktop_proxy_url.as_deref(),
            Some(proxy.as_str())
        );
    }
    route_to("a");
    let first = tokio::spawn(request(&client, &gateway).send());
    tokio::time::timeout(Duration::from_secs(5), a_accepted.notified())
        .await
        .expect("A must reach its own proxy before the route switches");
    route_to("b");
    let second = request(&client, &gateway).send();
    let (a, b) = tokio::join!(first, second);
    for response in [a.unwrap().unwrap(), b.unwrap()] {
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(response.json::<Value>().await.unwrap(), completion());
    }
    for tunnel in [tunnel_a, tunnel_b] {
        assert_eq!(
            tokio::time::timeout(Duration::from_secs(5), tunnel)
                .await
                .unwrap()
                .unwrap(),
            target
        );
    }
    {
        let requests = seen.lock().await;
        assert_eq!(requests.len(), 2);
        for id in ["a", "b"] {
            let mut expected = message();
            expected["model"] = json!(id);
            assert!(
                requests.contains(&expected),
                "unchanged request except existing model mapping: {requests:?}"
            );
        }
    }

    for proxy in [None, Some(""), Some(" \t\n ")] {
        db.save_provider("claude-desktop", &provider("direct", &upstream_url, proxy))
            .unwrap();
        route_to("direct");
        let response = request(&client, &gateway).send().await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(response.json::<Value>().await.unwrap(), completion());
    }
    assert_eq!(seen.lock().await.len(), 5);

    let (proxy, rejected) = connect_proxy(true, Arc::new(Notify::new()), None).await;
    let unused_port = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let unreachable_proxy = format!("http://{}", unused_port.local_addr().unwrap());
    drop(unused_port);
    for proxy in [&proxy, &unreachable_proxy] {
        db.save_provider(
            "claude-desktop",
            &provider("failure", &upstream_url, Some(proxy)),
        )
        .unwrap();
        route_to("failure");
        let response = request(&client, &gateway).send().await.unwrap();
        assert!(
            response.status().is_server_error(),
            "proxy failure must reach Desktop"
        );
        assert_eq!(seen.lock().await.len(), 5, "must not fall back to direct");
    }
    tokio::time::timeout(Duration::from_secs(5), rejected)
        .await
        .unwrap()
        .unwrap();

    service.stop().await.unwrap();
    upstream_task.abort();
}
