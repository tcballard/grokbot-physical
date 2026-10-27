use grokbot_orb::{
    audio::{PcmRing, resample},
    bots::normalise,
    config::Config,
    halo::Halo,
    protocol,
    speak::{Status, parse_request},
    wakeword::{WakeDetector, describe_model},
    webhook,
};
use serde_json::json;
use std::collections::HashMap;

fn pcm(x: &[i16]) -> Vec<u8> {
    x.iter().flat_map(|v| v.to_le_bytes()).collect()
}
#[test]
fn test_resample_known_ratios_and_identity() {
    assert_eq!(resample(&vec![0; 960], 48000, 16000).unwrap().len(), 320);
    assert_eq!(
        resample(&pcm(&[1000, -1000].repeat(160)), 16000, 24000)
            .unwrap()
            .len(),
        960
    );
    assert_eq!(resample(&vec![0; 960], 48000, 24000).unwrap().len(), 480);
    let p = pcm(&[1, -2, 30000]);
    assert_eq!(resample(&p, 16000, 16000).unwrap(), p);
    assert!(resample(&[], 16000, 24000).unwrap().is_empty());
    assert_eq!(resample(&[1, 2, 3], 16000, 16000).unwrap(), vec![1, 2]);
}
#[test]
fn test_resample_holds_a_constant_level() {
    assert_eq!(
        resample(&pcm(&[1000; 120]), 16000, 48000).unwrap(),
        pcm(&[1000; 360])
    );
}
#[test]
fn test_ring_keeps_the_newest_samples() {
    let mut r = PcmRing::new(4).unwrap();
    r.extend(&pcm(&[1, 2, 3, 4, 5]));
    assert_eq!(r.snapshot(), pcm(&[2, 3, 4, 5]));
    r.extend(&pcm(&[6, 7]));
    r.extend(&[]);
    assert_eq!(r.snapshot(), pcm(&[4, 5, 6, 7]));
}
#[test]
fn test_env_file_and_process_env() {
    let mut env = HashMap::from([
        ("WAKE_THRESHOLD".into(), "0.7".into()),
        ("DRY_RUN".into(), "yes".into()),
    ]);
    Config::merge_env(
        &mut env,
        "# comment\nexport WAKE_THRESHOLD=0.42\nORB_CALLBACK_PORT=9001\nXAI_VOICE=\"ara\"\nHALO_PIN=13",
    );
    let c = Config::from_map(&env).unwrap();
    assert_eq!(c.wake_threshold, 0.7);
    assert_eq!(c.callback_port, 9001);
    assert_eq!(c.xai_voice, "ara");
    assert_eq!(c.halo_pin, 13);
    assert_eq!(c.halo_channel, 1);
    assert!(c.dry_run);
}
#[test]
fn test_invalid_threshold_is_rejected() {
    assert!(Config::from_map(&HashMap::from([("WAKE_THRESHOLD".into(), "0".into())])).is_err());
}
#[test]
fn test_cli_flags_override_env() {
    let c = Config::from_args(
        [
            "orb",
            "--dry-run",
            "--simulate-wake",
            "--text",
            "Check the milk",
            "--bot",
            "bob",
            "--log-level",
            "debug",
        ],
        HashMap::from([("SIMULATE_BOT".into(), "writer".into())]),
    )
    .unwrap();
    assert!(c.dry_run && c.simulate_wake);
    assert_eq!(c.simulate_text, "Check the milk");
    assert_eq!(c.simulate_bot, "bob");
    assert_eq!(c.log_level, "DEBUG");
}
#[test]
fn test_help_exits_cleanly() {
    assert!(
        std::process::Command::new(env!("CARGO_BIN_EXE_grokbot-orb"))
            .arg("--help")
            .output()
            .unwrap()
            .status
            .success()
    );
}
#[test]
fn test_normalise_bot_names() {
    for (input, expected) in [
        ("Bob", "bob"),
        ("Oma Bot", "omabot"),
        ("Oma-Bot", "omabot"),
        ("writer", "writer"),
        ("Research Bot", "researchbot"),
    ] {
        assert_eq!(normalise(input).unwrap(), expected);
    }
    assert!(normalise("  ").is_err());
}
#[test]
fn test_state_transitions_and_bot_colours() {
    let mut h = Halo::new(16, 1.0).unwrap();
    for s in [
        "listening",
        "working:Bob",
        "working:Bob",
        "working:omabot",
        "working:writer",
        "working:research",
        "speaking",
        "error",
        "idle",
    ] {
        h.set(s).unwrap();
    }
    assert_eq!(
        h.history,
        vec![
            "idle",
            "listening",
            "working:bob",
            "working:omabot",
            "working:writer",
            "working:research",
            "speaking",
            "error",
            "idle"
        ]
    );
    for (s, c) in [
        ("working:bob", [40, 130, 255]),
        ("working:omabot", [16, 185, 110]),
        ("working:writer", [168, 85, 247]),
        ("working:research", [170, 186, 204]),
        ("listening", [255, 132, 28]),
    ] {
        h.set(s).unwrap();
        assert_eq!(h.colour(), c);
    }
}
#[test]
fn test_unknown_state_is_rejected() {
    assert!(Halo::new(16, 1.0).unwrap().set("disco").is_err());
}
#[test]
fn test_listening_breathes_and_working_comets() {
    let mut h = Halo::new(16, 1.0).unwrap();
    h.set("listening").unwrap();
    let a = h.render(0.0);
    assert_ne!(a, h.render(0.45));
    assert!(a.iter().all(|p| *p == a[0]));
    assert!(a[0][0] > a[0][2]);
    h.set("working:bob").unwrap();
    let p = h.render(0.2);
    assert_eq!(p.len(), 16);
    assert!(p.iter().any(|x| *x != p[0]));
}
#[test]
fn test_brightness_caps_the_frame() {
    let h = Halo::new(16, 0.01).unwrap();
    assert!(h.render(0.0)[0].iter().all(|v| *v < 20));
}
#[test]
fn test_session_update_is_24k_server_vad_with_the_tool() {
    let c = Config {
        xai_voice: "ara".into(),
        ..Config::default()
    };
    let e = protocol::session_update(&c);
    let s = &e["session"];
    assert_eq!(e["type"], "session.update");
    assert_eq!(s["voice"], "ara");
    assert_eq!(s["turn_detection"], json!({"type":"server_vad"}));
    assert_eq!(s["reasoning"], json!({"effort":"none"}));
    for k in ["input", "output"] {
        assert_eq!(
            s["audio"][k]["format"],
            json!({"type":"audio/pcm","rate":24000})
        );
    }
    assert_eq!(s["tools"][0]["name"], "delegate_to_grok_bot");
    assert_eq!(
        s["tools"][0]["parameters"]["required"],
        json!(["task", "bot_name"])
    );
    assert!(s["instructions"].as_str().unwrap().contains("Hey Grok Bot"));
}
#[test]
fn test_force_message_asks_for_the_words_verbatim() {
    let e = protocol::force_message("Soup is ready.");
    assert_eq!(e[0]["type"], "conversation.item.create");
    let t = e[0]["item"]["content"][0]["text"].as_str().unwrap();
    assert!(t.starts_with(protocol::FORCE_PREAMBLE) && t.contains("Soup is ready."));
    assert_eq!(e[1], json!({"type":"response.create"}));
}
#[test]
fn test_parse_tool_arguments() {
    let a = json!({"task":"a","bot_name":"bob"});
    assert_eq!(
        protocol::parse_tool_arguments(&json!(a.to_string())).unwrap(),
        a
    );
    assert_eq!(
        protocol::parse_tool_arguments(&json!({"task":"a"})).unwrap(),
        json!({"task":"a"})
    );
    assert_eq!(
        protocol::parse_tool_arguments(&json!("")).unwrap(),
        json!({})
    );
    for v in ["{", "[1]"] {
        assert!(protocol::parse_tool_arguments(&json!(v)).is_err());
    }
}
#[test]
fn test_parse_speak_request() {
    let r =
        parse_request(&json!({"spoken_text":" Soup is ready. ","status":"done","bot_name":"Bob"}))
            .unwrap();
    assert_eq!(r.spoken_text, "Soup is ready.");
    assert_eq!(r.status, Status::Done);
    assert_eq!(r.bot_name.as_deref(), Some("Bob"));
    let r = parse_request(&json!({"status":"WORKING"})).unwrap();
    assert_eq!(r.status, Status::Working);
    assert!(r.spoken_text.is_empty() && r.bot_name.is_none());
    for v in [
        json!({"status":"done"}),
        json!({"spoken_text":"no","status":"later"}),
        json!(["nope"]),
    ] {
        assert!(parse_request(&v).is_err());
    }
}
#[test]
fn test_threshold_and_cooldown() {
    let mut d = WakeDetector::new(0.5, 2.0).unwrap();
    assert!(d.feed(0.9, 0.0));
    assert!(!d.feed(0.9, 1.0));
    assert!(d.feed(0.9, 2.1));
    assert!(!d.feed(0.2, 5.0));
}
#[test]
fn test_describe_missing_model_names_the_fallback() {
    assert_eq!(
        describe_model("/missing/model.onnx", "hey_jarvis"),
        "hey_jarvis (fallback)"
    );
    assert_eq!(describe_model("Cargo.toml", "hey_jarvis"), "Cargo");
}
#[test]
fn test_headers_and_payload_keep_secrets_apart() {
    let c = Config {
        webhook_bearer: "hook-bearer".into(),
        automation_key: "auto-key".into(),
        speak_token: "speak-secret".into(),
        ..Config::default()
    };
    let h = webhook::headers(&c).unwrap();
    assert_eq!(h["authorization"], "Bearer hook-bearer");
    assert_eq!(h["x-automation-key"], "auto-key");
    let p = webhook::payload(
        " check the milk ",
        "OmaBot",
        "http://orb.local:8765/speak",
        &c.speak_token,
        "2026-10-25T21:00:00+00:00",
    )
    .unwrap();
    assert_eq!(p["task"], "check the milk");
    assert_eq!(p["bot_name"], "omabot");
    assert_eq!(
        p["callback"]["headers"]["Authorization"],
        "Bearer speak-secret"
    );
    assert_eq!(p["requested_at"], "2026-10-25T21:00:00+00:00");
    assert!(!p.to_string().contains("hook-bearer") && !p.to_string().contains("auto-key"));
}
#[tokio::test]
async fn test_dispatch_without_url_does_not_raise() {
    let r = webhook::dispatch(&Config::default(), "task", "bob", "http://orb.local/speak").await;
    assert!(!r.ok);
    assert!(r.body.contains("GROKBOT_WEBHOOK_URL"));
    assert_eq!(r.payload["bot_name"], "bob");
}
#[test]
fn test_reachability_warnings() {
    use grokbot_orb::daemon::reachability_warning as w;
    assert!(w("http://127.0.0.1:8765/speak").is_some());
    assert!(w("http://192.168.1.20/speak").is_some());
    assert!(w("http://100.64.1.2/speak").unwrap().contains("tailnet"));
    assert!(w("https://orb.example.ts.net/speak").is_none());
}
