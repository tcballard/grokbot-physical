//! Environment and command-line configuration. Secrets are redacted in Debug.
use anyhow::{Context, Result, ensure};
use clap::Parser;
use std::{collections::HashMap, ffi::OsString, path::PathBuf};
#[derive(Clone)]
pub struct Config {
    pub xai_api_key: String,
    pub xai_realtime_model: String,
    pub xai_realtime_url: String,
    pub xai_voice: String,
    pub xai_tts_url: String,
    pub webhook_url: String,
    pub webhook_bearer: String,
    pub automation_key: String,
    pub callback_host: String,
    pub callback_port: u16,
    pub public_base: String,
    pub speak_token: String,
    pub wake_model_path: String,
    pub wake_fallback: String,
    pub wake_assets_dir: PathBuf,
    pub wake_threshold: f64,
    pub wake_cooldown_s: f64,
    pub audio_input_device: String,
    pub audio_output_device: String,
    pub audio_rate: u32,
    pub barge_in: bool,
    pub session_idle_s: f64,
    pub halo_pin: i32,
    pub halo_channel: usize,
    pub halo_count: usize,
    pub halo_brightness: f64,
    pub halo_backend: String,
    pub tts_fallback: String,
    pub dry_run: bool,
    pub simulate_wake: bool,
    pub simulate_text: String,
    pub simulate_bot: String,
    pub log_level: String,
    pub webhook_timeout_s: f64,
}
impl std::fmt::Debug for Config {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Config")
            .field("dry_run", &self.dry_run)
            .field("callback_port", &self.callback_port)
            .finish_non_exhaustive()
    }
}
impl Default for Config {
    fn default() -> Self {
        Self::from_map(&HashMap::new()).expect("built-in config is valid")
    }
}
#[derive(Debug, Parser)]
#[command(
    name = "grokbot-orb",
    version,
    about = "Hey Grok Bot Orb: local wake word, xAI Realtime voice, Grok Bot webhooks."
)]
struct Args {
    #[arg(long)]
    env_file: Option<PathBuf>,
    #[arg(long)]
    dry_run: bool,
    #[arg(long)]
    simulate_wake: bool,
    #[arg(long)]
    text: Option<String>,
    #[arg(long)]
    bot: Option<String>,
    #[arg(long)]
    log_level: Option<String>,
}
impl Config {
    pub fn merge_env(env: &mut HashMap<String, String>, text: &str) {
        for line in text.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let line = line.strip_prefix("export ").unwrap_or(line).trim();
            if let Some((key, v)) = line.split_once('=') {
                let v = v.trim();
                let v = if v.len() >= 2
                    && ((v.starts_with('"') && v.ends_with('"'))
                        || (v.starts_with('\'') && v.ends_with('\'')))
                {
                    &v[1..v.len() - 1]
                } else {
                    v
                };
                env.entry(key.trim().into()).or_insert_with(|| v.into());
            }
        }
    }
    pub fn from_args<I, T>(args: I, mut env: HashMap<String, String>) -> Result<Self>
    where
        I: IntoIterator<Item = T>,
        T: Into<OsString> + Clone,
    {
        let a = Args::try_parse_from(args)?;
        if let Some(p) = a.env_file {
            Self::merge_env(
                &mut env,
                &std::fs::read_to_string(&p).with_context(|| format!("reading {}", p.display()))?,
            );
        }
        let mut c = Self::from_map(&env)?;
        c.dry_run |= a.dry_run;
        c.simulate_wake |= a.simulate_wake;
        if let Some(v) = a.text {
            c.simulate_text = v
        }
        if let Some(v) = a.bot {
            c.simulate_bot = v
        }
        if let Some(v) = a.log_level {
            c.log_level = v.to_uppercase()
        }
        Ok(c)
    }
    pub fn from_map(env: &HashMap<String, String>) -> Result<Self> {
        let s = |k: &str, d: &str| {
            env.get(k)
                .map(|v| v.trim())
                .filter(|v| !v.is_empty())
                .unwrap_or(d)
                .to_owned()
        };
        let b = |k: &str, d: bool| {
            env.get(k)
                .filter(|v| !v.trim().is_empty())
                .map(|v| {
                    matches!(
                        v.trim().to_lowercase().as_str(),
                        "1" | "true" | "yes" | "on"
                    )
                })
                .unwrap_or(d)
        };
        let pin = s("HALO_PIN", "12").parse::<i32>()?;
        let c = Self {
            xai_api_key: s("XAI_API_KEY", ""),
            xai_realtime_model: s("XAI_REALTIME_MODEL", "grok-voice-latest"),
            xai_realtime_url: s("XAI_REALTIME_URL", ""),
            xai_voice: s("XAI_VOICE", "eve"),
            xai_tts_url: s("XAI_TTS_URL", "https://api.x.ai/v1/tts"),
            webhook_url: s("GROKBOT_WEBHOOK_URL", ""),
            webhook_bearer: s("GROKBOT_WEBHOOK_BEARER", ""),
            automation_key: s("GROKBOT_AUTOMATION_KEY", ""),
            callback_host: s("ORB_CALLBACK_HOST", "0.0.0.0"),
            callback_port: s("ORB_CALLBACK_PORT", "8765").parse()?,
            public_base: s("ORB_PUBLIC_BASE", ""),
            speak_token: s("ORB_SPEAK_TOKEN", ""),
            wake_model_path: s("WAKE_MODEL_PATH", "models/hey_grok_bot.onnx"),
            wake_fallback: s("WAKE_FALLBACK", "hey_jarvis"),
            wake_assets_dir: s("WAKE_ASSETS_DIR", "models").into(),
            wake_threshold: s("WAKE_THRESHOLD", "0.5").parse()?,
            wake_cooldown_s: s("WAKE_COOLDOWN_S", "2").parse()?,
            audio_input_device: s("AUDIO_INPUT_DEVICE", ""),
            audio_output_device: s("AUDIO_OUTPUT_DEVICE", ""),
            audio_rate: s("AUDIO_RATE", "0").parse()?,
            barge_in: b("BARGE_IN", true),
            session_idle_s: s("SESSION_IDLE_S", "20").parse()?,
            halo_pin: pin,
            halo_channel: s(
                "HALO_CHANNEL",
                if matches!(pin, 13 | 19) { "1" } else { "0" },
            )
            .parse()?,
            halo_count: s("HALO_COUNT", "16").parse()?,
            halo_brightness: s("HALO_BRIGHTNESS", "0.35").parse()?,
            halo_backend: s("HALO_BACKEND", "auto").to_lowercase(),
            tts_fallback: s("TTS_FALLBACK", "auto").to_lowercase(),
            dry_run: b("DRY_RUN", false),
            simulate_wake: b("SIMULATE_WAKE", false),
            simulate_text: s("SIMULATE_TEXT", ""),
            simulate_bot: s("SIMULATE_BOT", "bob"),
            log_level: s("LOG_LEVEL", "INFO").to_uppercase(),
            webhook_timeout_s: s("WEBHOOK_TIMEOUT_S", "20").parse()?,
        };
        c.validate()?;
        Ok(c)
    }
    pub fn validate(&self) -> Result<()> {
        ensure!(
            self.wake_threshold > 0.0 && self.wake_threshold <= 1.0,
            "WAKE_THRESHOLD must be in (0, 1]"
        );
        ensure!(
            self.wake_cooldown_s.is_finite() && self.wake_cooldown_s >= 0.0,
            "WAKE_COOLDOWN_S must be >= 0"
        );
        ensure!(
            self.halo_brightness > 0.0 && self.halo_brightness <= 1.0,
            "HALO_BRIGHTNESS must be in (0, 1]"
        );
        ensure!(
            self.halo_count > 0 && i32::try_from(self.halo_count).is_ok(),
            "HALO_COUNT must fit a positive i32"
        );
        ensure!(self.halo_channel <= 1, "HALO_CHANNEL must be 0 or 1");
        for (name, v) in [
            ("SESSION_IDLE_S", self.session_idle_s),
            ("WEBHOOK_TIMEOUT_S", self.webhook_timeout_s),
        ] {
            ensure!(
                v.is_finite() && v > 0.0,
                "{name} must be finite and positive"
            );
        }
        ensure!(
            ["auto", "espeak", "none"].contains(&self.tts_fallback.as_str()),
            "TTS_FALLBACK must be auto, espeak, or none"
        );
        ensure!(
            ["auto", "log", "ws281x"].contains(&self.halo_backend.as_str()),
            "HALO_BACKEND must be auto, log, or ws281x"
        );
        Ok(())
    }
    pub fn realtime_url(&self) -> String {
        if self.xai_realtime_url.trim().is_empty() {
            format!(
                "wss://api.x.ai/v1/realtime?model={}",
                self.xai_realtime_model
            )
        } else {
            self.xai_realtime_url.trim().into()
        }
    }
    pub fn output_rate(&self) -> u32 {
        if self.audio_rate == 0 {
            16000
        } else {
            self.audio_rate
        }
    }
}
