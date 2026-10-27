//! HTTP callback validation and bounded queue admission.
use anyhow::{Result, bail, ensure};
use axum::{
    Json, Router,
    body::Bytes,
    extract::{DefaultBodyLimit, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use serde::Serialize;
use serde_json::{Value, json};
use std::sync::Arc;
use tokio::sync::mpsc;
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Status {
    Done,
    Error,
    Working,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpeakRequest {
    pub spoken_text: String,
    pub status: Status,
    pub bot_name: Option<String>,
}
pub fn parse_request(data: &Value) -> Result<SpeakRequest> {
    ensure!(data.is_object(), "body must be a JSON object");
    let status = match data
        .get("status")
        .and_then(Value::as_str)
        .unwrap_or(if data.get("status").is_none() {
            "done"
        } else {
            ""
        })
        .trim()
        .to_lowercase()
        .as_str()
    {
        "done" => Status::Done,
        "working" => Status::Working,
        "error" => Status::Error,
        _ => bail!("status must be done, error, or working"),
    };
    let text = match data.get("spoken_text") {
        None | Some(Value::Null) => "",
        Some(Value::String(s)) => s,
        _ => bail!("spoken_text must be a string"),
    }
    .trim();
    ensure!(
        status == Status::Working || !text.is_empty(),
        "spoken_text is required"
    );
    ensure!(
        text.chars().count() <= 2000,
        "spoken_text is longer than 2000 characters"
    );
    let bot = match data.get("bot_name") {
        None | Some(Value::Null) => None,
        Some(Value::String(s)) => {
            if s.trim().is_empty() {
                None
            } else {
                Some(s.trim().to_owned())
            }
        }
        _ => bail!("bot_name must be a string"),
    };
    Ok(SpeakRequest {
        spoken_text: text.into(),
        status,
        bot_name: bot,
    })
}
#[derive(Clone)]
struct HttpState {
    queue: mpsc::Sender<SpeakRequest>,
    health: Arc<dyn Fn() -> Value + Send + Sync>,
    token: String,
}
pub fn router(
    queue: mpsc::Sender<SpeakRequest>,
    health: impl Fn() -> Value + Send + Sync + 'static,
    token: String,
) -> Router {
    Router::new()
        .route("/health", get(health_route))
        .route("/speak", post(speak_route))
        .layer(DefaultBodyLimit::max(32768))
        .with_state(HttpState {
            queue,
            health: Arc::new(health),
            token,
        })
}
async fn health_route(State(s): State<HttpState>) -> Json<Value> {
    Json((s.health)())
}
fn error(code: StatusCode, message: &str) -> Response {
    (code, Json(json!({"ok":false,"error":message}))).into_response()
}
async fn speak_route(
    State(s): State<HttpState>,
    headers: HeaderMap,
    body: Result<Bytes, axum::extract::rejection::BytesRejection>,
) -> Response {
    let bearer = format!("Bearer {}", s.token);
    if !s.token.is_empty()
        && headers.get("authorization").and_then(|v| v.to_str().ok()) != Some(bearer.as_str())
        && headers.get("x-orb-token").and_then(|v| v.to_str().ok()) != Some(s.token.as_str())
    {
        return error(StatusCode::UNAUTHORIZED, "unauthorized");
    }
    let body = match body {
        Ok(b) => b,
        Err(_) => return error(StatusCode::PAYLOAD_TOO_LARGE, "body is too large"),
    };
    let data: Value = match serde_json::from_slice(&body) {
        Ok(d) => d,
        Err(_) => return error(StatusCode::BAD_REQUEST, "body must be JSON"),
    };
    let request = match parse_request(&data) {
        Ok(r) => r,
        Err(e) => return error(StatusCode::BAD_REQUEST, &e.to_string()),
    };
    let status = request.status;
    if s.queue.try_send(request).is_err() {
        return error(StatusCode::SERVICE_UNAVAILABLE, "speak queue is full");
    }
    Json(json!({"ok":true,"status":status})).into_response()
}
