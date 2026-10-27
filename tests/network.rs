use axum::{Json, Router, extract::State, http::HeaderMap, routing::post};
use grokbot_orb::{config::Config, daemon::Daemon, speak, webhook};
use serde_json::{Value, json};
use std::{
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::sync::mpsc;
fn config() -> Config {
    Config {
        dry_run: true,
        callback_host: "127.0.0.1".into(),
        callback_port: 0,
        tts_fallback: "none".into(),
        halo_backend: "log".into(),
        public_base: "http://orb.local:8765".into(),
        ..Config::default()
    }
}
async fn serve(app: Router) -> (String, tokio::task::JoinHandle<()>) {
    let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", l.local_addr().unwrap());
    let t = tokio::spawn(async move { axum::serve(l, app).await.unwrap() });
    (base, t)
}
async fn body(r: reqwest::Response) -> Value {
    r.json().await.unwrap()
}
#[tokio::test]
async fn test_health_is_open_and_speak_checks_the_token() {
    let (tx, mut rx) = mpsc::channel(8);
    let (base, t) = serve(speak::router(
        tx,
        || json!({"status":"ok","halo":"idle"}),
        "orb-token".into(),
    ))
    .await;
    let c = reqwest::Client::new();
    assert_eq!(
        body(c.get(format!("{base}/health")).send().await.unwrap()).await["status"],
        "ok"
    );
    let url = format!("{base}/speak");
    assert_eq!(
        c.post(&url)
            .json(&json!({"spoken_text":"Hi","status":"done"}))
            .send()
            .await
            .unwrap()
            .status(),
        401
    );
    assert_eq!(
        c.post(&url)
            .bearer_auth("orb-token")
            .json(&json!({"status":"done"}))
            .send()
            .await
            .unwrap()
            .status(),
        400
    );
    let r = c
        .post(&url)
        .bearer_auth("orb-token")
        .json(&json!({"spoken_text":"The milk is in.","status":"done","bot_name":"omabot"}))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 200);
    assert_eq!(body(r).await["ok"], true);
    assert_eq!(
        c.post(&url)
            .header("X-Orb-Token", "orb-token")
            .json(&json!({"status":"working","bot_name":"writer"}))
            .send()
            .await
            .unwrap()
            .status(),
        200
    );
    assert_eq!(rx.recv().await.unwrap().spoken_text, "The milk is in.");
    assert_eq!(rx.recv().await.unwrap().bot_name.as_deref(), Some("writer"));
    t.abort();
}
#[tokio::test]
async fn test_full_queue_returns_503() {
    let (tx, _rx) = mpsc::channel(1);
    tx.try_send(speak::parse_request(&json!({"status":"working"})).unwrap())
        .unwrap();
    let (base, t) = serve(speak::router(tx, || json!({"status":"ok"}), String::new())).await;
    let r = reqwest::Client::new()
        .post(format!("{base}/speak"))
        .json(&json!({"spoken_text":"Hi","status":"done"}))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 503);
    t.abort();
}
#[tokio::test]
async fn test_idle_callback_is_spoken_and_health_stays_up() {
    let d = Daemon::new(Config {
        speak_token: "orb-token".into(),
        ..config()
    })
    .unwrap();
    let r = d.start().await.unwrap();
    let base = format!("http://{}", r.addr);
    let c = reqwest::Client::new();
    let h = body(c.get(format!("{base}/health")).send().await.unwrap()).await;
    assert_eq!(h["status"], "ok");
    assert_eq!(h["service"], "hey-grokbot-orb");
    assert_eq!(h["mode"], "dry-run");
    assert_eq!(h["halo"], "idle");
    assert!(h.get("webhook").is_none());
    assert_eq!(
        c.post(format!("{base}/speak"))
            .json(&json!({"spoken_text":"no","status":"done"}))
            .send()
            .await
            .unwrap()
            .status(),
        401
    );
    assert_eq!(
        c.post(format!("{base}/speak"))
            .bearer_auth("orb-token")
            .json(&json!({"spoken_text":"Soup is ready.","status":"done","bot_name":"bob"}))
            .send()
            .await
            .unwrap()
            .status(),
        200
    );
    for _ in 0..50 {
        if d.health()["spoken"] == 1 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert_eq!(d.health()["spoken"], 1);
    assert_eq!(d.health()["halo"], "idle");
    r.stop().await;
}
type Seen = Arc<Mutex<Vec<(HeaderMap, Value)>>>;
async fn hook(State(s): State<Seen>, h: HeaderMap, Json(v): Json<Value>) -> Json<Value> {
    s.lock().unwrap().push((h, v));
    Json(json!({"accepted":true}))
}
#[tokio::test]
async fn test_dispatch_posts_bearer_and_automation_key() {
    let seen = Seen::default();
    let (base, t) = serve(
        Router::new()
            .route("/hook", post(hook))
            .with_state(seen.clone()),
    )
    .await;
    let c = Config {
        webhook_url: format!("{base}/hook"),
        webhook_bearer: "hook-bearer".into(),
        automation_key: "auto-key".into(),
        speak_token: "speak-secret".into(),
        ..config()
    };
    let r = webhook::dispatch(
        &c,
        "What's on tomorrow",
        "Bob",
        "http://orb.local:8765/speak",
    )
    .await;
    assert!(r.ok);
    assert_eq!(r.status, Some(200));
    assert_eq!(r.bot_name, "bob");
    let seen = seen.lock().unwrap();
    let (h, b) = &seen[0];
    assert_eq!(h["authorization"], "Bearer hook-bearer");
    assert_eq!(h["x-automation-key"], "auto-key");
    assert_eq!(b["task"], "What's on tomorrow");
    assert_eq!(b["bot_name"], "bob");
    assert!(!b.to_string().contains("auto-key") && !b.to_string().contains("hook-bearer"));
    t.abort();
}
#[tokio::test]
async fn test_simulated_wake_posts_the_webhook() {
    let seen = Seen::default();
    let (base, t) = serve(
        Router::new()
            .route("/hook", post(hook))
            .with_state(seen.clone()),
    )
    .await;
    let d = Daemon::new(Config {
        simulate_wake: true,
        simulate_text: "What's on the kitchen calendar tomorrow?".into(),
        simulate_bot: "Bob".into(),
        webhook_url: format!("{base}/hook"),
        webhook_bearer: "hook-bearer".into(),
        automation_key: "auto-key".into(),
        speak_token: "speak-secret".into(),
        ..config()
    })
    .unwrap();
    let r = d.start().await.unwrap();
    for _ in 0..100 {
        if !seen.lock().unwrap().is_empty() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    {
        let seen = seen.lock().unwrap();
        let (h, b) = &seen[0];
        assert_eq!(h["authorization"], "Bearer hook-bearer");
        assert_eq!(h["x-automation-key"], "auto-key");
        assert_eq!(b["task"], "What's on the kitchen calendar tomorrow?");
        assert_eq!(b["callback"]["url"], "http://orb.local:8765/speak");
        assert_eq!(
            b["callback"]["headers"]["Authorization"],
            "Bearer speak-secret"
        );
        assert_eq!(b["source"], "hey-grokbot-orb");
    }
    assert_eq!(d.health()["halo"], "working:bob");
    r.stop().await;
    t.abort();
}
#[tokio::test]
async fn test_speak_during_a_session_injects_force_message() {
    use futures_util::{SinkExt, StreamExt};
    use tokio_tungstenite::{accept_async, tungstenite::Message};
    let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("ws://{}", l.local_addr().unwrap());
    let (tx, mut rx) = mpsc::channel(20);
    let server = tokio::spawn(async move {
        let (s, _) = l.accept().await.unwrap();
        let mut ws = accept_async(s).await.unwrap();
        while let Some(Ok(m)) = ws.next().await {
            if let Message::Text(t) = m {
                tx.send(serde_json::from_str::<Value>(&t).unwrap())
                    .await
                    .unwrap();
            } else if let Message::Ping(p) = m {
                ws.send(Message::Pong(p)).await.unwrap();
            }
        }
    });
    let d = Daemon::new(Config {
        xai_api_key: "fake-test-key".into(),
        xai_realtime_url: url,
        simulate_wake: true,
        simulate_text: "hello".into(),
        ..config()
    })
    .unwrap();
    let r = d.start().await.unwrap();
    let first = tokio::time::timeout(Duration::from_secs(3), rx.recv())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(first["type"], "session.update");
    for _ in 0..100 {
        if d.health()["session_active"] == true {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    d.handle_speak(
        speak::parse_request(
            &json!({"spoken_text":"Kettle's boiled.","status":"done","bot_name":"Bob"}),
        )
        .unwrap(),
    )
    .await
    .unwrap();
    let found = tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            let e = rx.recv().await.unwrap();
            if e["item"]["content"][0]["text"]
                .as_str()
                .is_some_and(|s| s.starts_with(grokbot_orb::protocol::FORCE_PREAMBLE))
            {
                break e;
            }
        }
    })
    .await
    .unwrap();
    assert!(found.to_string().contains("Kettle's boiled."));
    assert_eq!(d.health()["spoken"], 1);
    assert_eq!(d.health()["halo"], "working:bob");
    d.handle_speak(speak::parse_request(&json!({"status":"working","bot_name":"writer"})).unwrap())
        .await
        .unwrap();
    assert_eq!(d.health()["halo"], "working:writer");
    assert_eq!(d.health()["spoken"], 1);
    r.stop().await;
    server.abort();
}
#[tokio::test]
async fn oversized_callback_is_rejected() {
    let (tx, _rx) = mpsc::channel(8);
    let (base, t) = serve(speak::router(tx, || json!({}), String::new())).await;
    let r = reqwest::Client::new()
        .post(format!("{base}/speak"))
        .body("x".repeat(32769))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 413);
    t.abort();
}
#[tokio::test]
async fn gateway_failure_retries_once() {
    let count = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let c = count.clone();
    let (base, t) = serve(Router::new().route(
        "/hook",
        post(move || {
            let c = c.clone();
            async move {
                let n = c.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                if n == 0 {
                    axum::http::StatusCode::BAD_GATEWAY
                } else {
                    axum::http::StatusCode::OK
                }
            }
        }),
    ))
    .await;
    let r = webhook::dispatch(
        &Config {
            webhook_url: format!("{base}/hook"),
            ..config()
        },
        "task",
        "bob",
        "http://orb.local/speak",
    )
    .await;
    assert!(r.ok);
    assert_eq!(count.load(std::sync::atomic::Ordering::SeqCst), 2);
    t.abort();
}

#[tokio::test]
async fn tts_requests_pcm_with_the_configured_voice() {
    let seen = Seen::default();
    async fn tts_hook(State(s): State<Seen>, h: HeaderMap, Json(v): Json<Value>) -> Vec<u8> {
        s.lock().unwrap().push((h, v));
        vec![1, 0, 255, 255]
    }
    let (base, t) = serve(
        Router::new()
            .route("/tts", post(tts_hook))
            .with_state(seen.clone()),
    )
    .await;
    let c = Config {
        xai_api_key: "fake-tts-key".into(),
        xai_tts_url: format!("{base}/tts"),
        xai_voice: "ara".into(),
        ..config()
    };
    let (pcm, rate) = grokbot_orb::tts::synthesize(&c, "Soup is ready.")
        .await
        .unwrap();
    assert_eq!(pcm, vec![1, 0, 255, 255]);
    assert_eq!(rate, 24000);
    let seen = seen.lock().unwrap();
    let (h, b) = &seen[0];
    assert_eq!(h["authorization"], "Bearer fake-tts-key");
    assert_eq!(b["text"], "Soup is ready.");
    assert_eq!(b["voice_id"], "ara");
    assert_eq!(
        b["output_format"],
        json!({"codec":"pcm","sample_rate":24000})
    );
    t.abort();
}
#[tokio::test]
async fn realtime_tool_call_returns_webhook_result() {
    use futures_util::{SinkExt, StreamExt};
    use tokio_tungstenite::{accept_async, tungstenite::Message};
    let seen = Seen::default();
    let (base, http) = serve(
        Router::new()
            .route("/hook", post(hook))
            .with_state(seen.clone()),
    )
    .await;
    let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("ws://{}", l.local_addr().unwrap());
    let (tx, rx) = tokio::sync::oneshot::channel();
    let ws = tokio::spawn(async move {
        let (s, _) = l.accept().await.unwrap();
        let mut ws = accept_async(s).await.unwrap();
        ws.next().await.unwrap().unwrap();
        ws.send(Message::Text(json!({"type":"response.function_call_arguments.done","name":"delegate_to_grok_bot","call_id":"test-call","arguments":"{\"task\":\"Check the milk\",\"bot_name\":\"Oma Bot\"}"}).to_string().into())).await.unwrap();
        while let Some(Ok(Message::Text(t))) = ws.next().await {
            let e: Value = serde_json::from_str(&t).unwrap();
            if e["item"]["type"] == "function_call_output" {
                tx.send(e).unwrap();
                break;
            }
        }
    });
    let d = Daemon::new(Config {
        simulate_wake: true,
        xai_api_key: "fake-key".into(),
        xai_realtime_url: url,
        webhook_url: format!("{base}/hook"),
        ..config()
    })
    .unwrap();
    let running = d.start().await.unwrap();
    let e = tokio::time::timeout(Duration::from_secs(3), rx)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(e["item"]["call_id"], "test-call");
    let out: Value = serde_json::from_str(e["item"]["output"].as_str().unwrap()).unwrap();
    assert_eq!(out["ok"], true);
    assert_eq!(out["bot_name"], "omabot");
    assert_eq!(seen.lock().unwrap()[0].1["task"], "Check the milk");
    running.stop().await;
    ws.abort();
    http.abort();
}
