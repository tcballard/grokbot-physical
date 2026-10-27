//! Task delivery with one retry for gateway errors and separate callback credentials.
use crate::{bots, config::Config};
use anyhow::Result;
use reqwest::header::{HeaderMap, HeaderValue};
use serde_json::{Value, json};
#[derive(Clone)]
pub struct DispatchResult {
    pub ok: bool,
    pub status: Option<u16>,
    pub body: String,
    pub bot_name: String,
    pub payload: Value,
}
impl std::fmt::Debug for DispatchResult {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DispatchResult")
            .field("ok", &self.ok)
            .field("status", &self.status)
            .field("bot_name", &self.bot_name)
            .finish_non_exhaustive()
    }
}
pub fn headers(c: &Config) -> Result<HeaderMap> {
    let mut h = HeaderMap::new();
    for (k, v) in [
        ("content-type", "application/json"),
        ("accept", "application/json"),
        ("user-agent", "hey-grokbot-orb"),
    ] {
        h.insert(k, HeaderValue::from_static(v));
    }
    if !c.webhook_bearer.is_empty() {
        let mut v = HeaderValue::from_str(&format!("Bearer {}", c.webhook_bearer))?;
        v.set_sensitive(true);
        h.insert("authorization", v);
    }
    if !c.automation_key.is_empty() {
        let mut v = HeaderValue::from_str(&c.automation_key)?;
        v.set_sensitive(true);
        h.insert("x-automation-key", v);
    }
    Ok(h)
}
pub fn payload(task: &str, bot: &str, callback: &str, token: &str, now: &str) -> Result<Value> {
    let bot = bots::normalise(bot)?;
    let mut h = json!({"Content-Type":"application/json"});
    if !token.is_empty() {
        h["Authorization"] = json!(format!("Bearer {token}"));
    }
    Ok(
        json!({"task":task.trim(),"bot_name":bot,"source":"hey-grokbot-orb","callback":{"url":callback,"method":"POST","headers":h,"json_shape":{"spoken_text":"what the orb should say, one or two sentences","status":"done","bot_name":bot}},"requested_at":now}),
    )
}
pub async fn dispatch(c: &Config, task: &str, bot: &str, callback: &str) -> DispatchResult {
    let mut r = DispatchResult {
        ok: false,
        status: None,
        body: String::new(),
        bot_name: bot.into(),
        payload: json!({}),
    };
    let run = async {
        r.bot_name = bots::normalise(bot)?;
        anyhow::ensure!(!task.trim().is_empty(), "task is empty");
        r.payload = payload(
            task,
            &r.bot_name,
            callback,
            &c.speak_token,
            &chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Micros, false),
        )?;
        anyhow::ensure!(!c.webhook_url.is_empty(), "GROKBOT_WEBHOOK_URL is not set");
        let url = reqwest::Url::parse(c.webhook_url.trim())
            .map_err(|_| anyhow::anyhow!("GROKBOT_WEBHOOK_URL is not an http(s) URL"))?;
        anyhow::ensure!(
            ["http", "https"].contains(&url.scheme()) && url.host_str().is_some(),
            "GROKBOT_WEBHOOK_URL is not an http(s) URL"
        );
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs_f64(c.webhook_timeout_s))
            .redirect(reqwest::redirect::Policy::none())
            .build()?;
        let h = headers(c)?;
        for attempt in 0..2 {
            let response = client
                .post(url.clone())
                .headers(h.clone())
                .json(&r.payload)
                .send()
                .await
                .map_err(|e| e.without_url())?;
            let status = response.status().as_u16();
            r.status = Some(status);
            r.ok = (200..300).contains(&status);
            r.body = response
                .text()
                .await
                .map_err(|e| e.without_url())?
                .chars()
                .take(2000)
                .collect();
            if attempt == 0 && [502, 503, 504].contains(&status) {
                tokio::time::sleep(std::time::Duration::from_millis(400)).await;
            } else {
                break;
            }
        }
        Ok::<_, anyhow::Error>(())
    };
    if let Err(e) = run.await {
        r.ok = false;
        r.body = e.to_string();
    }
    r
}
