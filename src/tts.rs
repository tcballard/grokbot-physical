//! xAI PCM synthesis with local espeak fallback and WAV decoding.
use crate::config::Config;
use anyhow::{Result, ensure};
use serde_json::json;
use std::{io::Cursor, process::Stdio, time::Duration};
/// WAV PCM16 uses the first channel, matching the original implementation.
pub fn wav_pcm(data: &[u8]) -> Result<(Vec<u8>, u32)> {
    let mut reader = hound::WavReader::new(Cursor::new(data))?;
    let spec = reader.spec();
    if spec.bits_per_sample != 16 {
        return Ok((Vec::new(), spec.sample_rate));
    }
    ensure!(spec.channels > 0, "WAV has no channels");
    let mut out = Vec::new();
    for (i, s) in reader.samples::<i16>().enumerate() {
        match s {
            Ok(s) if i % usize::from(spec.channels) == 0 => out.extend_from_slice(&s.to_le_bytes()),
            Ok(_) => {}
            Err(hound::Error::IoError(e)) if e.kind() == std::io::ErrorKind::UnexpectedEof => break,
            Err(e) => return Err(e.into()),
        }
    }
    Ok((out, spec.sample_rate))
}
async fn xai(c: &Config, text: &str) -> Result<(Vec<u8>, u32)> {
    let response=reqwest::Client::builder().timeout(Duration::from_secs(30)).redirect(reqwest::redirect::Policy::none()).build()?.post(&c.xai_tts_url).bearer_auth(&c.xai_api_key).json(&json!({"text":text,"voice_id":c.xai_voice,"language":"en","output_format":{"codec":"pcm","sample_rate":24000}})).send().await.map_err(|e|e.without_url())?.error_for_status().map_err(|e|e.without_url())?;
    let body = response.bytes().await?;
    ensure!(!body.is_empty(), "TTS returned an empty body");
    if body.starts_with(b"RIFF") {
        wav_pcm(&body)
    } else {
        Ok((body.to_vec(), 24000))
    }
}
pub async fn synthesize(c: &Config, text: &str) -> Result<(Vec<u8>, u32)> {
    if !c.xai_api_key.is_empty() {
        match xai(c, text).await {
            Ok(p) => return Ok(p),
            Err(e) => tracing::warn!(error=%e,"xAI TTS failed"),
        }
    }
    if c.tts_fallback != "none" {
        for exe in ["espeak-ng", "espeak"] {
            match tokio::process::Command::new(exe)
                .args(["--stdout", "-s", "155", text])
                .stderr(Stdio::null())
                .kill_on_drop(true)
                .output()
                .await
            {
                Ok(o) if o.status.success() && !o.stdout.is_empty() => return wav_pcm(&o.stdout),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
                _ => break,
            }
        }
    }
    Ok((Vec::new(), 24000))
}
