//! xAI wire events, kept separate from transport for compatibility tests.
use crate::{bots, config::Config};
use anyhow::{Result, ensure};
use serde_json::{Value, json};
pub const FORCE_PREAMBLE: &str = "The countertop orb must speak the following update aloud, verbatim, with no preamble and no extra words:\n";
pub fn session_update(c: &Config) -> Value {
    json!({"type":"session.update","session":{
     "voice":c.xai_voice,"instructions":format!("You are the voice of Hey Grok Bot, a small countertop orb in a kitchen. The person is speaking out loud, hands free, with no phone in front of them.\n\nTalk like a person in the room. One or two short sentences. Spoken English. No markdown, no lists, no symbols, no URLs.\n\nWhen they ask you to do something that needs tools, lookup, writing, scheduling, messages, or more than a sentence of knowledge, call delegate_to_grok_bot exactly once. bot_name is one of: {}. task is their request in their words, including the details they gave.\n\nAfter the tool returns ok, say only a short acknowledgement such as \"Bob's on it.\" Do not invent the result. The orb speaks the result later, out loud, when the bot finishes.\n\nAnswer a quick question yourself, in one breath, and do not call the tool.\n\nIf they say stop, goodbye, or that's all, say goodbye in a few words and stop.",bots::ROSTER),
     "reasoning":{"effort":"none"},"turn_detection":{"type":"server_vad"},"audio":{"input":{"format":{"type":"audio/pcm","rate":24000},"transcription":{"keyterms":["Hey Grok Bot","Grok Bot","Bob","OmaBot","Writer"]}},"output":{"format":{"type":"audio/pcm","rate":24000}}},
     "tools":[{"type":"function","name":"delegate_to_grok_bot","description":format!("Hand a real task to a Grok Bot that can use tools and take actions. Call this for lookup, writing, scheduling, messages, or any multi-step work. Do not pretend the work is already done. bot_name is one of: {}.",bots::ROSTER),"parameters":{"type":"object","properties":{"task":{"type":"string","description":"The task in the person's words, with the details they gave."},"bot_name":{"type":"string","enum":bots::NAMES,"description":"Which Grok Bot should do the work."}},"required":["task","bot_name"]}}]
    }})
}
pub fn user_text(text: &str) -> Vec<Value> {
    vec![
        json!({"type":"conversation.item.create","item":{"type":"message","role":"user","content":[{"type":"input_text","text":text}]}}),
        json!({"type":"response.create"}),
    ]
}
pub fn force_message(text: &str) -> Vec<Value> {
    user_text(&format!("{FORCE_PREAMBLE}{}", text.trim()))
}
pub fn tool_result(call_id: &str, output: Value) -> Vec<Value> {
    vec![
        json!({"type":"conversation.item.create","item":{"type":"function_call_output","call_id":call_id,"output":output.to_string()}}),
        json!({"type":"response.create"}),
    ]
}
pub fn parse_tool_arguments(raw: &Value) -> Result<Value> {
    let v = match raw {
        Value::Null => json!({}),
        Value::String(s) if s.is_empty() => json!({}),
        Value::String(s) => serde_json::from_str(s)?,
        v => v.clone(),
    };
    ensure!(v.is_object(), "tool arguments must be a JSON object");
    Ok(v)
}
