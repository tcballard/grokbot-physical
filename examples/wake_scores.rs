#[cfg(feature = "wake")]
fn main() -> anyhow::Result<()> {
    use grokbot_orb::{config::Config, wakeword::WakeModel};
    let mut args = std::env::args().skip(1);
    let assets = args
        .next()
        .ok_or_else(|| anyhow::anyhow!("usage: wake_scores MODEL_DIR WAV"))?;
    let wav = args
        .next()
        .ok_or_else(|| anyhow::anyhow!("missing WAV path"))?;
    let c = Config {
        wake_assets_dir: assets.into(),
        ..Config::default()
    };
    let mut model = WakeModel::load(&c)?;
    let (pcm, rate) = grokbot_orb::tts::wav_pcm(&std::fs::read(wav)?)?;
    let pcm = grokbot_orb::audio::resample(&pcm, rate, 16000)?;
    for frame in pcm.as_chunks::<2560>().0 {
        println!("{}", model.score(frame)?);
    }
    Ok(())
}
#[cfg(not(feature = "wake"))]
fn main() {
    eprintln!("wake_scores requires --features wake");
}
