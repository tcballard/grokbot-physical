//! Bidirectional WebSocket transport with separate capture, playback and tool workers.
use crate::{daemon::Daemon, halo::State, protocol};
use anyhow::Result;
use base64::{Engine, engine::general_purpose::STANDARD};
use futures_util::{SinkExt, StreamExt};
use serde_json::{Value, json};
use std::{
    sync::Arc,
    time::{Duration, Instant},
};
use tokio::sync::mpsc;
use tokio_tungstenite::{
    connect_async_with_config,
    tungstenite::{Message, client::IntoClientRequest, protocol::WebSocketConfig},
};
#[derive(Debug)]
pub(crate) struct Outgoing {
    events: Vec<Value>,
    done: tokio::sync::oneshot::Sender<Result<(), String>>,
}
pub(crate) async fn send(tx: &mpsc::Sender<Outgoing>, events: Vec<Value>) -> Result<()> {
    let (done, ack) = tokio::sync::oneshot::channel();
    tokio::time::timeout(Duration::from_secs(10), async {
        tx.send(Outgoing { events, done })
            .await
            .map_err(|_| anyhow::anyhow!("realtime session closed"))?;
        ack.await
            .map_err(|_| anyhow::anyhow!("realtime writer closed"))?
            .map_err(anyhow::Error::msg)
    })
    .await?
}
struct SessionGuard(Arc<Daemon>);
impl Drop for SessionGuard {
    fn drop(&mut self) {
        {
            let mut s = self.0.state.lock().expect("runtime state poisoned");
            s.session = None;
            s.opening = false;
            s.speaking = false;
        }
        self.0.restore();
    }
}
pub async fn run(
    d: Arc<Daemon>,
    preroll: Vec<u8>,
    text: Option<String>,
    mut mic: Option<tokio::sync::broadcast::Receiver<Vec<u8>>>,
) -> Result<()> {
    let _guard = SessionGuard(d.clone());
    if d.config.xai_api_key.is_empty() {
        d.set_halo(State::Error);
        tokio::time::sleep(Duration::from_millis(800)).await;
        return Ok(());
    }
    d.set_halo(State::Listening);
    let mut request = d.config.realtime_url().into_client_request()?;
    let mut auth = format!("Bearer {}", d.config.xai_api_key)
        .parse::<tokio_tungstenite::tungstenite::http::HeaderValue>()?;
    auth.set_sensitive(true);
    request.headers_mut().insert("authorization", auth);
    let config = WebSocketConfig::default().max_message_size(Some(4_000_000));
    let (mut ws, _) = tokio::time::timeout(
        Duration::from_secs(10),
        connect_async_with_config(request, Some(config), false),
    )
    .await??;
    ws.send(Message::Text(
        protocol::session_update(&d.config).to_string().into(),
    ))
    .await?;
    if !preroll.is_empty() {
        let p = crate::audio::resample(&preroll, 16000, 24000)?;
        ws.send(Message::Text(
            json!({"type":"input_audio_buffer.append","audio":STANDARD.encode(p)})
                .to_string()
                .into(),
        ))
        .await?;
    }
    if let Some(t) = text {
        for e in protocol::user_text(&t) {
            ws.send(Message::Text(e.to_string().into())).await?;
        }
    }
    let (tx, mut outgoing) = mpsc::channel::<Outgoing>(64);
    {
        let mut s = d.state.lock().expect("runtime state poisoned");
        s.session = Some(tx.clone());
        s.opening = false;
        s.responses_done = 0;
        s.speaking = false;
        s.last_activity = Instant::now();
    }
    let (audio_tx, mut audio_rx) = mpsc::channel::<Vec<u8>>(128);
    let player = d.clone();
    let mut workers = tokio::task::JoinSet::new();
    workers.spawn(async move {
        while let Some(p) = audio_rx.recv().await {
            if let Err(e) = player.player.lock().await.write(&p, 24000).await {
                tracing::warn!(error=%e,"speaker write failed");
            }
        }
    });
    let mut stop = d.shutdown.subscribe();
    let mut tick = tokio::time::interval(Duration::from_millis(200));
    let mut last_ping = Instant::now();
    let result = async {
        loop {
            tokio::select! {
                _ = async { let _ = stop.wait_for(|s| *s).await; } => break,
                _ = tick.tick() => {
                    let expired = {
                        let s = d.state.lock().expect("runtime state poisoned");
                        let limit = if d.config.dry_run && s.responses_done > 0 {
                            d.config.session_idle_s.min(3.0)
                        } else {
                            d.config.session_idle_s
                        };
                        s.last_activity.elapsed().as_secs_f64() > limit
                    };
                    if expired { break; }
                    if last_ping.elapsed() > Duration::from_secs(20) {
                        ws.send(Message::Ping(Vec::new().into())).await?;
                        last_ping = Instant::now();
                    }
                }
                Some(batch) = outgoing.recv() => {
                    let result = async {
                        for event in batch.events {
                            ws.send(Message::Text(event.to_string().into())).await?;
                        }
                        Ok::<(), anyhow::Error>(())
                    }.await;
                    let _ = batch.done.send(result.as_ref().map(|_| ()).map_err(ToString::to_string));
                    result?;
                }
                block = async {
                    match &mut mic {
                        Some(rx) => loop {
                            match rx.recv().await {
                                Ok(pcm) => break Some(pcm),
                                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                                Err(tokio::sync::broadcast::error::RecvError::Closed) => break None,
                            }
                        },
                        None => std::future::pending().await,
                    }
                } => {
                    if let Some(pcm) = block {
                        let event = json!({"type":"input_audio_buffer.append","audio":STANDARD.encode(pcm)});
                        ws.send(Message::Text(event.to_string().into())).await?;
                    } else {
                        mic = None;
                    }
                }
                raw = ws.next() => {
                    let Some(raw) = raw else { break; };
                    let mut audio = None;
                    match raw? {
                        Message::Binary(bytes) => audio = Some(bytes.to_vec()),
                        Message::Close(_) => break,
                        Message::Ping(ping) => ws.send(Message::Pong(ping)).await?,
                        Message::Text(raw) => {
                            let Ok(event) = serde_json::from_str::<Value>(&raw) else { continue; };
                            let kind = event["type"].as_str().unwrap_or("");
                            match kind {
                                "response.output_audio.delta" | "response.audio.delta" => {
                                    let data = event["delta"].as_str().filter(|s| !s.is_empty())
                                        .or_else(|| event["audio"].as_str()).unwrap_or("");
                                    if !data.is_empty() { audio = STANDARD.decode(data).ok(); }
                                }
                                "response.function_call_arguments.done" => {
                                    d.state.lock().expect("runtime state poisoned").last_activity = Instant::now();
                                    let d = d.clone();
                                    let tx = tx.clone();
                                    workers.spawn(async move {
                                        let result = d.on_tool(&event).await;
                                        let call_id = event["call_id"].as_str().unwrap_or("");
                                        let _ = send(&tx, protocol::tool_result(call_id, result)).await;
                                    });
                                }
                                "response.done" | "response.output_audio.done" |
                                "input_audio_buffer.speech_started" | "input_audio_buffer.speech_stopped" | "error" => {
                                    let mut state = d.state.lock().expect("runtime state poisoned");
                                    state.last_activity = Instant::now();
                                    match kind {
                                        "response.done" | "response.output_audio.done" => {
                                            state.responses_done += 1;
                                            state.speaking = false;
                                        }
                                        "input_audio_buffer.speech_started" if state.held.is_none() =>
                                            state.halo.set_state(State::Listening),
                                        "error" => state.halo.set_state(State::Error),
                                        _ => {},
                                    }
                                }
                                _ => {},
                            }
                        }
                        _ => {},
                    }
                    if let Some(pcm) = audio {
                        {
                            let mut state = d.state.lock().expect("runtime state poisoned");
                            state.last_activity = Instant::now();
                            state.speaking = true;
                            if !matches!(state.halo.state, State::Working(_) | State::Error) {
                                state.halo.set_state(State::Speaking);
                            }
                        }
                        if audio_tx.try_send(pcm).is_err() {
                            tracing::warn!("speaker queue overflow; dropping audio");
                        }
                    }
                }
                Some(result) = workers.join_next() => {
                    if let Err(error) = result { tracing::warn!(%error, "session worker failed"); }
                }
            }
        }
        Ok::<(), anyhow::Error>(())
    }.await;
    let _ = tokio::time::timeout(Duration::from_secs(1), ws.close(None)).await;
    drop(audio_tx);
    workers.abort_all();
    while workers.join_next().await.is_some() {}
    result
}
