//! Local ONNX wake scoring and microphone capture. No network before a wake hit.
use anyhow::{Result, ensure};
use std::path::Path;
#[derive(Debug)]
pub struct WakeDetector {
    threshold: f64,
    cooldown: f64,
    last: Option<f64>,
}
impl WakeDetector {
    pub fn new(threshold: f64, cooldown: f64) -> Result<Self> {
        ensure!(
            threshold > 0.0 && threshold <= 1.0,
            "threshold must be in (0, 1]"
        );
        ensure!(
            cooldown.is_finite() && cooldown >= 0.0,
            "cooldown must be finite and nonnegative"
        );
        Ok(Self {
            threshold,
            cooldown,
            last: None,
        })
    }
    pub fn feed(&mut self, score: f64, now: f64) -> bool {
        if !score.is_finite()
            || score < self.threshold
            || self.last.is_some_and(|last| now - last < self.cooldown)
        {
            return false;
        }
        self.last = Some(now);
        true
    }
}
pub fn describe_model(path: &str, fallback: &str) -> String {
    let p = Path::new(path);
    if p.is_file() {
        p.file_stem()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned()
    } else {
        format!("{fallback} (fallback)")
    }
}
#[cfg(feature = "wake")]
mod native {
    use super::*;
    use crate::config::Config;
    use tract_onnx::prelude::{
        DatumExt, Framework, InferenceModelExt, IntoRunnable, Tensor, TypedRunnableModel, tvec,
    };
    type Model = std::sync::Arc<TypedRunnableModel>;
    #[derive(Debug)]
    pub struct WakeModel {
        mel_first: Model,
        mel: Model,
        embedding: Model,
        classifier: Model,
        lookback: Vec<f32>,
        mels: Vec<f32>,
        features: Vec<f32>,
        frames: usize,
        feature_count: usize,
    }
    fn load(path: &Path, shape: &[usize]) -> Result<Model> {
        use anyhow::Context;
        tract_onnx::onnx()
            .model_for_path(path)
            .with_context(|| {
                format!(
                    "loading {}; run scripts/fetch_wake_models.sh",
                    path.display()
                )
            })?
            .with_input_fact(0, f32::fact(shape).into())?
            .into_optimized()?
            .into_runnable()
    }
    fn infer(model: &Model, shape: &[usize], data: &[f32]) -> Result<Vec<f32>> {
        let outputs = model.run(tvec!(Tensor::from_shape(shape, data)?.into()))?;
        Ok(outputs[0]
            .to_plain_array_view::<f32>()?
            .iter()
            .copied()
            .collect())
    }
    impl WakeModel {
        pub fn load(c: &Config) -> Result<Self> {
            let assets = &c.wake_assets_dir;
            let custom = Path::new(&c.wake_model_path);
            let fallback = assets.join(format!("{}_v0.1.onnx", c.wake_fallback));
            let path = if custom.is_file() { custom } else { &fallback };
            let classifier = tract_onnx::onnx()
                .model_for_path(path)?
                .into_optimized()?
                .into_runnable()?;
            let shape = classifier
                .model()
                .input_fact(0)?
                .shape
                .as_concrete()
                .ok_or_else(|| anyhow::anyhow!("wake classifier must have a concrete input shape"))?
                .to_vec();
            ensure!(
                shape.len() == 3 && shape[0] == 1 && shape[2] == 96,
                "wake classifier must accept [1, frames, 96]"
            );
            let n = shape[1];
            ensure!(
                n > 0 && n <= 120,
                "wake classifier frame window must be 1..120"
            );
            let mel_path = assets.join("melspectrogram.onnx");
            Ok(Self {
                mel_first: load(&mel_path, &[1, 1280])?,
                mel: load(&mel_path, &[1, 1760])?,
                embedding: load(&assets.join("embedding_model.onnx"), &[1, 76, 32, 1])?,
                classifier,
                lookback: Vec::new(),
                mels: vec![1.0; 76 * 32],
                features: vec![0.0; n * 96],
                frames: 0,
                feature_count: n,
            })
        }
        pub fn score(&mut self, pcm: &[u8]) -> Result<f64> {
            ensure!(
                pcm.len() == 2560,
                "wake frame must contain 1280 PCM16 samples"
            );
            let mut input = std::mem::take(&mut self.lookback);
            input.extend(
                pcm.as_chunks::<2>()
                    .0
                    .iter()
                    .map(|p| f32::from(i16::from_le_bytes([p[0], p[1]]))),
            );
            self.lookback = input[input.len() - 480..].to_vec();
            let model = if self.frames == 0 {
                &self.mel_first
            } else {
                &self.mel
            };
            let mel = infer(model, &[1, input.len()], &input)?;
            self.mels.extend(mel.into_iter().map(|v| v / 10.0 + 2.0));
            self.mels.drain(..self.mels.len() - 76 * 32);
            let emb = infer(&self.embedding, &[1, 76, 32, 1], &self.mels)?;
            ensure!(emb.len() == 96, "embedding must have 96 values");
            self.features.drain(..96);
            self.features.extend(emb);
            let scores = infer(
                &self.classifier,
                &[1, self.feature_count, 96],
                &self.features,
            )?;
            ensure!(!scores.is_empty(), "wake classifier returned no scores");
            self.frames += 1;
            if self.frames <= 5 {
                return Ok(0.0);
            }
            Ok(f64::from(
                scores.into_iter().fold(f32::NEG_INFINITY, f32::max),
            ))
        }
    }
}
#[cfg(feature = "wake")]
pub use native::WakeModel;
#[cfg(feature = "wake")]
pub async fn capture(d: std::sync::Arc<crate::daemon::Daemon>) -> Result<()> {
    use crate::{
        audio::{PcmRing, resample},
        realtime,
    };
    use std::{process::Stdio, time::Instant};
    use tokio::{io::AsyncReadExt, process::Command, sync::broadcast};
    let c = d.config.clone();
    let mut model = tokio::task::spawn_blocking(move || WakeModel::load(&c)).await??;
    let mut detector = WakeDetector::new(d.config.wake_threshold, d.config.wake_cooldown_s)?;
    let rate = d.config.output_rate();
    let mut cmd = Command::new("arecord");
    cmd.args([
        "-q",
        "-t",
        "raw",
        "-f",
        "S16_LE",
        "-c",
        "1",
        "-r",
        &rate.to_string(),
    ]);
    if !d.config.audio_input_device.is_empty() {
        cmd.args(["-D", &d.config.audio_input_device]);
    }
    let mut child = cmd.stdout(Stdio::piped()).kill_on_drop(true).spawn()?;
    let mut stdout = child
        .stdout
        .take()
        .ok_or_else(|| anyhow::anyhow!("capture stdout unavailable"))?;
    let (captured, mut blocks) = broadcast::channel::<Result<Vec<u8>, String>>(32);
    let mut capture_task = tokio::task::JoinSet::new();
    capture_task.spawn(async move {
        let mut block = vec![0; ((rate / 50).max(1) * 2) as usize];
        loop {
            match stdout.read_exact(&mut block).await {
                Ok(_) => {
                    if captured.send(Ok(block.clone())).is_err() {
                        break;
                    }
                }
                Err(e) => {
                    let _ = captured.send(Err(e.to_string()));
                    break;
                }
            }
        }
        let _ = child.kill().await;
        let _ = child.wait().await;
    });
    let mut ring = PcmRing::new(32000)?;
    let mut buf = Vec::new();
    let mut sessions = tokio::task::JoinSet::new();
    let mut live: Option<broadcast::Sender<Vec<u8>>> = None;
    let start = Instant::now();
    loop {
        while let Some(result) = sessions.try_join_next() {
            live = None;
            match result {
                Ok(Ok(())) => {}
                Ok(Err(e)) => tracing::warn!(error=%e,"voice session failed"),
                Err(e) => tracing::warn!(error=%e,"voice session task failed"),
            }
        }
        let block = match blocks.recv().await {
            Ok(Ok(block)) => block,
            Ok(Err(e)) => return Err(anyhow::anyhow!("microphone capture stopped: {e}")),
            Err(broadcast::error::RecvError::Lagged(_)) => continue,
            Err(broadcast::error::RecvError::Closed) => anyhow::bail!("microphone capture closed"),
        };
        let pcm = resample(&block, rate, 16000)?;
        ring.extend(&pcm);
        if let Some(tx) = &live {
            let speaking = d.state.lock().expect("runtime state poisoned").speaking;
            if d.config.barge_in || !speaking {
                let _ = tx.send(resample(&block, rate, 24000)?);
            }
            continue;
        }
        buf.extend(pcm);
        while buf.len() >= 2560 {
            let frame: Vec<u8> = buf.drain(..2560).collect();
            let result = tokio::task::spawn_blocking(move || {
                let score = model.score(&frame);
                (model, score)
            })
            .await?;
            model = result.0;
            let hit = detector.feed(result.1?, start.elapsed().as_secs_f64());
            let allowed = {
                let s = d.state.lock().expect("runtime state poisoned");
                !s.opening && s.session.is_none() && Instant::now() >= s.cooldown_until
            };
            if hit && allowed {
                d.state.lock().expect("runtime state poisoned").opening = true;
                let (tx, rx) = broadcast::channel(64);
                live = Some(tx);
                let daemon = d.clone();
                let preroll = ring.snapshot();
                sessions.spawn(async move { realtime::run(daemon, preroll, None, Some(rx)).await });
                break;
            }
        }
    }
}
#[cfg(not(feature = "wake"))]
pub async fn capture(_: std::sync::Arc<crate::daemon::Daemon>) -> Result<()> {
    anyhow::bail!("local wake detection requires the default wake feature")
}
