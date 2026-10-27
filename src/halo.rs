//! Halo animation and an optional Raspberry Pi PWM driver.
use crate::bots;
use anyhow::{Result, bail};
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum State {
    Idle,
    Listening,
    Speaking,
    Working(Option<String>),
    Error,
}
impl std::fmt::Display for State {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Idle => f.write_str("idle"),
            Self::Listening => f.write_str("listening"),
            Self::Speaking => f.write_str("speaking"),
            Self::Working(Some(b)) => write!(f, "working:{b}"),
            Self::Working(None) => f.write_str("working"),
            Self::Error => f.write_str("error"),
        }
    }
}
impl std::str::FromStr for State {
    type Err = anyhow::Error;
    fn from_str(s: &str) -> Result<Self> {
        let k = s.trim().to_lowercase();
        Ok(match k.as_str() {
            "idle" => Self::Idle,
            "listening" => Self::Listening,
            "speaking" => Self::Speaking,
            "working" => Self::Working(None),
            "error" => Self::Error,
            _ => {
                if let Some(b) = k.strip_prefix("working:") {
                    Self::Working(Some(bots::normalise(b)?))
                } else {
                    bail!("unknown halo state {s:?}")
                }
            }
        })
    }
}
#[derive(Debug)]
pub struct Halo {
    pub state: State,
    pub history: Vec<String>,
    count: usize,
    brightness: f64,
}
impl Halo {
    pub fn new(count: usize, brightness: f64) -> Result<Self> {
        anyhow::ensure!(count > 0, "LED count must be >= 1");
        Ok(Self {
            state: State::Idle,
            history: vec!["idle".into()],
            count,
            brightness,
        })
    }
    pub fn set(&mut self, s: &str) -> Result<()> {
        self.set_state(s.parse()?);
        Ok(())
    }
    pub fn set_state(&mut self, s: State) {
        if self.state != s {
            tracing::info!(halo=%s,"halo state");
            self.history.push(s.to_string());
            if self.history.len() > 128 {
                self.history.remove(0);
            }
            self.state = s;
        }
    }
    pub fn colour(&self) -> [u8; 3] {
        match &self.state {
            State::Idle => [92, 44, 14],
            State::Listening => [255, 132, 28],
            State::Speaking => [255, 214, 170],
            State::Error => [220, 40, 40],
            State::Working(b) => bots::colour(b.as_deref().unwrap_or("")),
        }
    }
    pub fn render(&self, t: f64) -> Vec<[u8; 3]> {
        let (period, min, max) = match &self.state {
            State::Idle => (4.2, 0.08, 0.28),
            State::Listening => (1.25, 0.35, 1.0),
            State::Speaking => (0.9, 0.45, 1.0),
            State::Working(None) => (1.6, 0.28, 0.9),
            State::Working(Some(_)) => (1.6, 0.28, 0.95),
            State::Error => (0.42, 0.15, 1.0),
        };
        let base = min + (max - min) * ((std::f64::consts::TAU * t / period).sin() + 1.0) / 2.0;
        (0..self.count)
            .map(|i| {
                let mut level = base;
                if matches!(self.state, State::Working(_)) && self.count > 1 {
                    let d = (i as f64 / self.count as f64 - (t / period).rem_euclid(1.0)).abs();
                    let d = d.min(1.0 - d);
                    let width = 1.2 / self.count as f64;
                    let bump = if d < width { 1.0 - d / width } else { 0.0 };
                    level = (base * 0.55).max(bump);
                }
                self.colour().map(|c| {
                    (f64::from(c) * (level * self.brightness).clamp(0.0, 1.0)).round_ties_even()
                        as u8
                })
            })
            .collect()
    }
}
/// Runs the non-Send PWM controller entirely on its own thread and blanks it on exit.
pub fn run_driver(
    config: crate::config::Config,
    mut frames: tokio::sync::watch::Receiver<Vec<[u8; 3]>>,
) {
    #[cfg(feature = "pi")]
    {
        use rs_ws281x::{ChannelBuilder, ControllerBuilder, StripType};
        let result = ControllerBuilder::new()
            .freq(800000)
            .dma(10)
            .channel(
                config.halo_channel,
                ChannelBuilder::new()
                    .pin(config.halo_pin)
                    .count(config.halo_count as i32)
                    .strip_type(StripType::Ws2811Grb)
                    .brightness(255)
                    .build(),
            )
            .build();
        match result {
            Ok(mut driver) => {
                while frames.has_changed().is_ok() {
                    let pixels = frames.borrow_and_update().clone();
                    for (led, rgb) in driver.leds_mut(config.halo_channel).iter_mut().zip(pixels) {
                        *led = [rgb[2], rgb[1], rgb[0], 0];
                    }
                    if let Err(e) = driver.render() {
                        tracing::warn!(error=%e,"halo write failed");
                        break;
                    }
                    std::thread::sleep(std::time::Duration::from_millis(33));
                }
                driver.leds_mut(config.halo_channel).fill([0; 4]);
                let _ = driver.render();
            }
            Err(e) => tracing::warn!(error=%e,"NeoPixel unavailable; using log backend"),
        }
    }
    #[cfg(not(feature = "pi"))]
    {
        let _ = (&config, &mut frames);
        tracing::warn!("NeoPixel support requires cargo build --features pi; using log backend");
    }
}
