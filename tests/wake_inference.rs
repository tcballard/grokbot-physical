#![cfg(feature = "wake")]
use grokbot_orb::{config::Config, tts::wav_pcm, wakeword::WakeModel};
fn compare(name: &str, expected_wake: bool) {
    let mut model = WakeModel::load(&Config::default()).unwrap();
    let path = format!("tests/fixtures/{name}");
    let (pcm, rate) = wav_pcm(&std::fs::read(format!("{path}.wav")).unwrap()).unwrap();
    assert_eq!(rate, 16000);
    let expected: Vec<f64> =
        serde_json::from_slice(&std::fs::read(format!("{path}.json")).unwrap()).unwrap();
    let actual: Vec<f64> = pcm
        .as_chunks::<2560>()
        .0
        .iter()
        .map(|f| model.score(f).unwrap())
        .collect();
    assert_eq!(actual.len(), expected.len());
    for (i, (a, b)) in actual.iter().zip(&expected).enumerate().skip(16) {
        assert!(
            (a - b).abs() < 0.00001,
            "{name} frame {i}: Rust={a}, Python={b}"
        );
    }
    assert_eq!(actual.iter().any(|v| *v >= 0.5), expected_wake);
    assert!(actual[..5].iter().all(|v| *v == 0.0));
}
#[test]
#[ignore = "requires scripts/fetch_wake_models.sh; run by CI after downloading verified models"]
fn native_positive_matches_python() {
    compare("hey_jarvis", true);
}
#[test]
#[ignore = "requires scripts/fetch_wake_models.sh; run by CI after downloading verified models"]
fn native_negative_matches_python() {
    compare("negative", false);
}
