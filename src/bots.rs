//! Bot identities shared by tools, webhooks and the halo.
use anyhow::{Result, ensure};
/// Convert human bot names into webhook identifiers, retaining unknown names.
pub fn normalise(name: &str) -> Result<String> {
    let key: String = name
        .to_lowercase()
        .chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .take(32)
        .collect();
    ensure!(!key.is_empty(), "bot_name is empty");
    Ok(match key.as_str() {
        "oma" => "omabot".into(),
        "write" => "writer".into(),
        _ => key,
    })
}
/// Stable tool enum order matches the original insertion-ordered roster.
pub const NAMES: [&str; 3] = ["bob", "omabot", "writer"];
/// Spoken descriptions used to choose a bot.
pub const ROSTER: &str = "bob (general operator for research, calendar, messages, and anything else); omabot (home and household); writer (drafts and edits)";
/// RGB for known bots and a neutral colour for unknown bots.
pub fn colour(name: &str) -> [u8; 3] {
    match name {
        "bob" => [40, 130, 255],
        "omabot" => [16, 185, 110],
        "writer" => [168, 85, 247],
        _ => [170, 186, 204],
    }
}
