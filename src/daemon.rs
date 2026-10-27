//! Owns runtime state, callback processing, capture, and orderly shutdown.
use crate::{
    audio::Player,
    config::Config,
    halo::{Halo, State},
    protocol,
    speak::{SpeakRequest, Status},
};
use anyhow::Result;
use serde_json::{Value, json};
use std::{
    collections::VecDeque,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};
use tokio::{
    sync::{mpsc, watch},
    task::JoinHandle,
};
#[derive(Debug)]
pub(crate) struct RuntimeState {
    pub halo: Halo,
    pub held: Option<State>,
    pub spoken: VecDeque<String>,
    pub session: Option<mpsc::Sender<crate::realtime::Outgoing>>,
    pub opening: bool,
    pub speaking: bool,
    pub responses_done: u32,
    pub last_activity: Instant,
    pub cooldown_until: Instant,
    pub wake_name: String,
}
#[derive(Debug)]
pub struct Daemon {
    pub config: Config,
    pub(crate) state: Mutex<RuntimeState>,
    pub(crate) player: tokio::sync::Mutex<Player>,
    pub(crate) shutdown: watch::Sender<bool>,
    port: std::sync::atomic::AtomicU16,
}
impl Daemon {
    pub fn new(config: Config) -> Result<Arc<Self>> {
        config.validate()?;
        let halo = Halo::new(config.halo_count, config.halo_brightness)?;
        let wake_name =
            crate::wakeword::describe_model(&config.wake_model_path, &config.wake_fallback);
        let player = Player::new(&config);
        let now = Instant::now();
        let port = std::sync::atomic::AtomicU16::new(config.callback_port);
        Ok(Arc::new(Self {
            config,
            state: Mutex::new(RuntimeState {
                halo,
                held: None,
                spoken: VecDeque::new(),
                session: None,
                opening: false,
                speaking: false,
                responses_done: 0,
                last_activity: now,
                cooldown_until: now,
                wake_name,
            }),
            player: tokio::sync::Mutex::new(player),
            shutdown: watch::channel(false).0,
            port,
        }))
    }
    pub fn health(&self) -> Value {
        let s = self.state.lock().expect("runtime state poisoned");
        let active = s.session.is_some();
        json!({"status":"ok","service":"hey-grokbot-orb","version":env!("CARGO_PKG_VERSION"),"mode":if self.config.dry_run{"dry-run"}else if active{"session"}else{"listening"},"halo":s.halo.state.to_string(),"session_active":active,"wake_model":s.wake_name,"spoken":s.spoken.len()})
    }
    pub fn callback_url(&self) -> String {
        let b = self.config.public_base.trim().trim_end_matches('/');
        if !b.is_empty() {
            return format!("{}/speak", b.strip_suffix("/speak").unwrap_or(b));
        }
        let h = match self.config.callback_host.as_str() {
            "0.0.0.0" | "::" => "127.0.0.1",
            h => h,
        };
        let host = if h.contains(':') {
            format!("[{h}]")
        } else {
            h.to_owned()
        };
        format!(
            "http://{host}:{}/speak",
            self.port.load(std::sync::atomic::Ordering::Relaxed)
        )
    }
    pub fn set_halo(&self, state: State) {
        self.state
            .lock()
            .expect("runtime state poisoned")
            .halo
            .set_state(state);
    }
    pub(crate) fn restore(&self) {
        let mut s = self.state.lock().expect("runtime state poisoned");
        let held = s.held.clone().unwrap_or(State::Idle);
        s.halo.set_state(held);
        s.cooldown_until = Instant::now() + Duration::from_secs_f64(self.config.wake_cooldown_s);
    }
    fn remember(&self, text: &str) {
        let mut s = self.state.lock().expect("runtime state poisoned");
        s.spoken.push_back(text.into());
        while s.spoken.len() > 50 {
            s.spoken.pop_front();
        }
        s.last_activity = Instant::now();
    }
    pub async fn handle_speak(&self, r: SpeakRequest) -> Result<()> {
        let bot = r
            .bot_name
            .as_deref()
            .map(crate::bots::normalise)
            .transpose()?;
        let session = {
            let mut s = self.state.lock().expect("runtime state poisoned");
            if r.status == Status::Working {
                let h = State::Working(bot);
                s.held = Some(h.clone());
                s.halo.set_state(h);
                return Ok(());
            }
            if r.status == Status::Error && s.session.is_none() {
                s.halo.set_state(State::Error)
            } else if let Some(b) = bot {
                s.halo.set_state(State::Working(Some(b)))
            } else if s.session.is_none() {
                s.halo.set_state(State::Speaking)
            }
            s.session.clone()
        };
        if let Some(session) = session
            && !r.spoken_text.is_empty()
            && crate::realtime::send(&session, protocol::force_message(&r.spoken_text))
                .await
                .is_ok()
        {
            self.remember(&r.spoken_text);
            self.state.lock().expect("runtime state poisoned").held = None;
            return Ok(());
        }
        if !r.spoken_text.is_empty() {
            self.remember(&r.spoken_text);
            let (pcm, rate) = crate::tts::synthesize(&self.config, &r.spoken_text).await?;
            self.player.lock().await.write(&pcm, rate).await?;
        }
        {
            let mut s = self.state.lock().expect("runtime state poisoned");
            s.held = None;
            if s.session.is_none() {
                s.halo.set_state(State::Idle);
                s.cooldown_until =
                    Instant::now() + Duration::from_secs_f64(self.config.wake_cooldown_s);
            }
        }
        Ok(())
    }
    pub async fn dispatch(&self, task: &str, bot: &str) -> (bool, String) {
        let bot = match crate::bots::normalise(bot) {
            Ok(b) => b,
            Err(e) => {
                self.set_halo(State::Error);
                return (false, e.to_string());
            }
        };
        {
            let mut s = self.state.lock().expect("runtime state poisoned");
            let h = State::Working(Some(bot.clone()));
            s.held = Some(h.clone());
            s.halo.set_state(h);
        }
        let r = crate::webhook::dispatch(&self.config, task, &bot, &self.callback_url()).await;
        if r.ok {
            (
                true,
                "Task accepted. Say one short acknowledgement and nothing else.".into(),
            )
        } else {
            let mut s = self.state.lock().expect("runtime state poisoned");
            s.held = None;
            s.halo.set_state(State::Error);
            (false, r.body)
        }
    }
    pub(crate) async fn on_tool(&self, e: &Value) -> Value {
        let name = e["name"].as_str().unwrap_or("");
        if name != "delegate_to_grok_bot" {
            return json!({"ok":false,"detail":format!("unknown tool {name}")});
        }
        let parsed = (|| -> Result<(String, String)> {
            let a = protocol::parse_tool_arguments(&e["arguments"])?;
            let bot = crate::bots::normalise(
                a["bot_name"]
                    .as_str()
                    .filter(|s| !s.is_empty())
                    .unwrap_or("bob"),
            )?;
            let task = a["task"].as_str().unwrap_or("").trim().to_owned();
            anyhow::ensure!(!task.is_empty(), "task is empty");
            Ok((task, bot))
        })();
        match parsed {
            Ok((task, bot)) => {
                let (ok, detail) = self.dispatch(&task, &bot).await;
                json!({"ok":ok,"bot_name":bot,"detail":detail})
            }
            Err(e) => json!({"ok":false,"detail":e.to_string()}),
        }
    }
    pub async fn start(self: &Arc<Self>) -> Result<Running> {
        let listener = tokio::net::TcpListener::bind((
            self.config.callback_host.as_str(),
            self.config.callback_port,
        ))
        .await?;
        let addr = listener.local_addr()?;
        self.port
            .store(addr.port(), std::sync::atomic::Ordering::Relaxed);
        let (tx, mut rx) = mpsc::channel(8);
        let d = self.clone();
        let app = crate::speak::router(tx, move || d.health(), self.config.speak_token.clone());
        let mut tasks = Vec::new();
        let mut stop = self.shutdown.subscribe();
        tasks.push(tokio::spawn(async move {
            if let Err(e) = axum::serve(listener, app)
                .with_graceful_shutdown(async move {
                    let _ = stop.wait_for(|s| *s).await;
                })
                .await
            {
                tracing::error!(error=%e,"callback server stopped");
            }
        }));
        let d = self.clone();
        let mut stop = self.shutdown.subscribe();
        tasks.push(tokio::spawn(async move {
            loop {
                let request = tokio::select! {
                    biased;
                    _ = async { let _ = stop.wait_for(|s| *s).await; } => break,
                    request = rx.recv() => match request { Some(r) => r, None => break },
                };
                tokio::select! {
                    _ = async { let _ = stop.wait_for(|s| *s).await; } => break,
                    result = d.handle_speak(request) => {
                        if let Err(error) = result {
                            tracing::error!(%error, "speak callback failed");
                            d.set_halo(State::Error);
                        }
                    }
                }
            }
        }));
        let (frames, receiver) = watch::channel(vec![[0; 3]; self.config.halo_count]);
        let driver = if self.config.halo_backend != "log"
            && (!self.config.dry_run || self.config.halo_backend == "ws281x")
        {
            let c = self.config.clone();
            Some(std::thread::spawn(move || {
                crate::halo::run_driver(c, receiver)
            }))
        } else {
            None
        };
        let d = self.clone();
        let mut stop = self.shutdown.subscribe();
        tasks.push(tokio::spawn(async move {
            let start = Instant::now();
            let mut tick = tokio::time::interval(Duration::from_millis(33));
            loop {
                tokio::select! {
                    _ = async { let _ = stop.wait_for(|s| *s).await; } => break,
                    _ = tick.tick() => {
                        let frame = d.state.lock().expect("runtime state poisoned")
                            .halo.render(start.elapsed().as_secs_f64());
                        frames.send_replace(frame);
                    }
                }
            }
        }));
        let d = self.clone();
        let mut stop = self.shutdown.subscribe();
        tasks.push(tokio::spawn(async move {
            let work = async {
                if d.config.simulate_wake {
                    d.set_halo(State::Listening);
                    if d.config.xai_api_key.is_empty() {
                        let text = d.config.simulate_text.trim();
                        if text.is_empty() {
                            d.set_halo(State::Idle);
                        } else {
                            d.dispatch(text, &d.config.simulate_bot).await;
                        }
                        Ok(())
                    } else {
                        let text = if d.config.simulate_text.trim().is_empty() {
                            "Say hello in one short sentence."
                        } else {
                            d.config.simulate_text.trim()
                        };
                        crate::realtime::run(d.clone(), Vec::new(), Some(text.to_owned()), None)
                            .await
                    }
                } else if !d.config.dry_run {
                    crate::wakeword::capture(d.clone()).await
                } else {
                    std::future::pending::<Result<()>>().await
                }
            };
            tokio::select! {
                _ = async { let _ = stop.wait_for(|s| *s).await; } => {},
                result = work => {
                    if let Err(error) = result {
                        tracing::error!(%error, "voice runtime failed");
                        d.set_halo(State::Error);
                    }
                }
            }
        }));
        if self.config.xai_api_key.is_empty() {
            tracing::warn!("XAI_API_KEY is empty; voice sessions and xAI TTS are disabled");
        }
        if self.config.webhook_url.is_empty() {
            tracing::warn!("GROKBOT_WEBHOOK_URL is empty");
        }
        if let Some(w) = reachability_warning(&self.callback_url()) {
            tracing::warn!("{w}");
        }
        tracing::info!(%addr,"callback server listening");
        Ok(Running {
            daemon: self.clone(),
            addr,
            tasks,
            driver,
        })
    }
}
#[derive(Debug)]
pub struct Running {
    pub daemon: Arc<Daemon>,
    pub addr: std::net::SocketAddr,
    tasks: Vec<JoinHandle<()>>,
    driver: Option<std::thread::JoinHandle<()>>,
}
impl Running {
    pub async fn stop(mut self) {
        self.daemon.shutdown.send_replace(true);
        for mut task in self.tasks.drain(..) {
            if tokio::time::timeout(Duration::from_secs(3), &mut task)
                .await
                .is_err()
            {
                task.abort();
                let _ = task.await;
            }
        }
        self.daemon.player.lock().await.close().await;
        if let Some(driver) = self.driver.take() {
            let _ = tokio::task::spawn_blocking(move || driver.join()).await;
        }
    }
}
impl Drop for Running {
    fn drop(&mut self) {
        self.daemon.shutdown.send_replace(true);
        for t in &self.tasks {
            t.abort();
        }
    }
}
pub fn reachability_warning(url: &str) -> Option<String> {
    let u = reqwest::Url::parse(url).ok()?;
    let host = u.host_str()?.trim_matches(['[', ']']);
    if host.ends_with(".ts.net") {
        return None;
    }
    if host.ends_with(".local") || host == "localhost" {
        return Some("callback is local; set ORB_PUBLIC_BASE to a Tailscale Funnel URL".into());
    }
    if let Ok(ip) = host.parse::<std::net::IpAddr>() {
        let private = match ip {
            std::net::IpAddr::V4(v) => {
                let o = v.octets();
                if o[0] == 100 && (64..=127).contains(&o[1]) {
                    return Some(
                        "callback requires the runner on this tailnet or Tailscale Funnel".into(),
                    );
                }
                v.is_private()
                    || v.is_loopback()
                    || v.is_link_local()
                    || v.is_unspecified()
                    || v.is_broadcast()
                    || v.is_multicast()
            }
            std::net::IpAddr::V6(v) => {
                v.is_loopback()
                    || v.is_unspecified()
                    || v.is_unique_local()
                    || v.is_unicast_link_local()
                    || v.is_multicast()
            }
        };
        if private {
            return Some("callback is not reachable from the cloud; use Tailscale Funnel or the same tailnet".into());
        }
    }
    None
}
