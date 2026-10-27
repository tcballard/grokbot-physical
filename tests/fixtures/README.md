# Wake inference parity fixtures

Audio source: `oww-rs` 0.3.3 (MIT), published by Skoky, from
https://github.com/skoky/oww_rs. The crate's `hey_jarvis.wav` (16 kHz) and
`negative.wav` (44.1 kHz) were reduced to the first channel, resampled using
the original orb's linear PCM16 algorithm, and padded with two seconds of
silence before and one second after. The files here are 16 kHz mono PCM16.
See THIRD_PARTY.md for the upstream MIT notice.

Golden JSON scores were produced with openWakeWord 0.6.0, ONNX Runtime CPU,
NumPy seed 42, `hey_jarvis_v0.1.onnx`, and the v0.5.1 feature models whose
hashes are pinned in scripts/fetch_wake_models.sh. Each score covers 1280
samples. Python uses random initial embeddings; Rust uses zeros. Compare
from frame 16 onward, when the classifier's history contains only real
embeddings. The first five scores are suppressed by both implementations.

Run `scripts/fetch_wake_models.sh`, then
`cargo test --test wake_inference -- --include-ignored`.
