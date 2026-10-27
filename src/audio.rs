//! PCM16 conversion and bounded preroll. Playback uses ALSA's native aplay utility.
use anyhow::{Result, ensure};
use std::{collections::VecDeque, process::Stdio};
use tokio::{
    io::AsyncWriteExt,
    process::{Child, Command},
};
/// Linearly resample mono little-endian PCM16; discard an incomplete final sample.
pub fn resample(pcm: &[u8], src: u32, dst: u32) -> Result<Vec<u8>> {
    ensure!(src > 0 && dst > 0, "sample rates must be positive");
    let pcm = &pcm[..pcm.len() / 2 * 2];
    if pcm.is_empty() || src == dst {
        return Ok(pcm.to_vec());
    }
    let samples: Vec<f64> = pcm
        .as_chunks::<2>()
        .0
        .iter()
        .map(|p| f64::from(i16::from_le_bytes([p[0], p[1]])))
        .collect();
    if samples.len() == 1 {
        return Ok(pcm.to_vec());
    }
    let n = (samples.len() as f64 * f64::from(dst) / f64::from(src))
        .round_ties_even()
        .max(1.0) as usize;
    let mut out = Vec::with_capacity(n * 2);
    for i in 0..n {
        let pos = i as f64 * samples.len() as f64 / n as f64;
        let a = pos.floor() as usize;
        let b = (a + 1).min(samples.len() - 1);
        let v = samples[a] + (samples[b] - samples[a]) * (pos - a as f64);
        out.extend_from_slice(&(v.round_ties_even().clamp(-32768.0, 32767.0) as i16).to_le_bytes());
    }
    Ok(out)
}
#[derive(Debug)]
pub struct PcmRing {
    samples: VecDeque<i16>,
    capacity: usize,
}
impl PcmRing {
    pub fn new(capacity: usize) -> Result<Self> {
        ensure!(capacity > 0, "ring capacity must be >= 1");
        Ok(Self {
            samples: VecDeque::with_capacity(capacity),
            capacity,
        })
    }
    pub fn extend(&mut self, pcm: &[u8]) {
        for p in pcm.as_chunks::<2>().0 {
            if self.samples.len() == self.capacity {
                self.samples.pop_front();
            }
            self.samples.push_back(i16::from_le_bytes([p[0], p[1]]));
        }
    }
    pub fn snapshot(&self) -> Vec<u8> {
        self.samples.iter().flat_map(|v| v.to_le_bytes()).collect()
    }
}
#[derive(Debug)]
pub struct Player {
    child: Option<Child>,
    device: String,
    rate: u32,
    dry_run: bool,
}
impl Player {
    pub fn new(config: &crate::config::Config) -> Self {
        Self {
            child: None,
            device: config.audio_output_device.clone(),
            rate: config.output_rate(),
            dry_run: config.dry_run,
        }
    }
    pub async fn write(&mut self, pcm: &[u8], rate: u32) -> Result<()> {
        if pcm.is_empty() || self.dry_run {
            return Ok(());
        }
        let pcm = resample(pcm, rate, self.rate)?;
        if self.child.is_none() {
            let mut cmd = Command::new("aplay");
            cmd.args([
                "-q",
                "-t",
                "raw",
                "-f",
                "S16_LE",
                "-c",
                "1",
                "-r",
                &self.rate.to_string(),
            ]);
            if !self.device.is_empty() {
                cmd.args(["-D", &self.device]);
            }
            self.child = Some(
                cmd.stdin(Stdio::piped())
                    .stdout(Stdio::null())
                    .kill_on_drop(true)
                    .spawn()?,
            );
        }
        let result = if let Some(stdin) = self.child.as_mut().and_then(|c| c.stdin.as_mut()) {
            stdin.write_all(&pcm).await
        } else {
            Err(std::io::Error::other("speaker stdin closed"))
        };
        if result.is_err() {
            self.close().await;
        }
        result?;
        Ok(())
    }
    pub async fn close(&mut self) {
        if let Some(mut child) = self.child.take() {
            let _ = child.kill().await;
            let _ = child.wait().await;
        }
    }
}
