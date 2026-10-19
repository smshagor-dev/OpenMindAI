//! Sampling settings for every llama-server request, in one place so a value
//! tuned for one kind of output never leaks into another.
//!
//! - [`SamplingProfile::CORE_CHAT`]: conversational chat with a Qwen3 model
//!   (OpenMindAI Core) in both chat and thinking mode. Live tests on Qwen3-4B Q4_K_M found Qwen's
//!   thinking-mode values (0.6 / 0.95 / 20) more factually reliable in Bengali
//!   than its non-thinking values (0.7 / 0.8), so both modes use them. The
//!   presence penalty and DRY stop short romanized prompts (e.g. Banglish) from
//!   looping on one phrase until `max_tokens`.
//! - [`SamplingProfile::GENERAL_CHAT`]: conversational chat with any other model
//!   family. Same values without the penalties: live tests showed Qwen-tuned
//!   penalties make Nemotron emit `<tool_call>` runs in plain chat.
//! - [`SamplingProfile::AGENT`]: OpenAgent / Agent Setup (Nemotron). Low
//!   temperature and no repetition penalties, because the edit protocol, code
//!   and tool calls legitimately repeat identifiers and must stay exact.
//! - [`SamplingProfile::STRUCTURED`]: JSON-producing calls (Connected Apps,
//!   parallel planning workers).
//! - [`SamplingProfile::DETERMINISTIC`]: greedy decoding for extraction (OCR).

use serde_json::{json, Map, Value};

// The engine is launched with a bounded context window. Longer writing tasks
// need more output room than a short chat reply, but must leave enough space
// for the prompt and the model's chat template.
const CHAT_CONTEXT_RESERVE_TOKENS: u32 = 512;
const MIN_RESPONSE_TOKENS: u32 = 256;
const CHAT_OUTPUT_TOKENS: u32 = 2_048;
const LONG_OUTPUT_TOKENS: u32 = 4_096;
const SEARCH_OUTPUT_TOKENS: u32 = 3_072;

/// Estimate the input budget without scanning image base64 bytes as prose.
/// This is intentionally conservative; it does not replace the model tokenizer.
fn estimated_prompt_tokens(messages: &[Value]) -> u32 {
    let mut tokens: u64 = 128;
    for message in messages {
        tokens = tokens.saturating_add(32);
        let content = &message["content"];
        if let Some(text) = content.as_str() {
            tokens = tokens.saturating_add((text.len() as u64).div_ceil(3));
        } else if let Some(parts) = content.as_array() {
            for part in parts {
                if let Some(text) = part["text"].as_str() {
                    tokens = tokens.saturating_add((text.len() as u64).div_ceil(3));
                } else if part["type"] == "image_url" {
                    // Vision models expand a compressed image into visual tokens.
                    tokens = tokens.saturating_add(2_048);
                } else {
                    tokens = tokens.saturating_add(256);
                }
            }
        }
    }
    tokens.min(u64::from(u32::MAX)) as u32
}

/// Pick the largest useful response budget that still fits the loaded
/// llama-server context. None means the prompt leaves insufficient headroom.
pub fn chat_output_budget(mode: &str, context_size: u32, messages: &[Value]) -> Option<u32> {
    let desired = match mode.trim().to_ascii_lowercase().as_str() {
        "document" | "pdf" | "research" | "thinking" => LONG_OUTPUT_TOKENS,
        "search" => SEARCH_OUTPUT_TOKENS,
        _ => CHAT_OUTPUT_TOKENS,
    };
    let remaining = context_size.saturating_sub(
        estimated_prompt_tokens(messages).saturating_add(CHAT_CONTEXT_RESERVE_TOKENS),
    );
    (remaining >= MIN_RESPONSE_TOKENS).then_some(desired.min(remaining))
}


#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SamplingProfile {
    pub temperature: f64,
    pub top_p: f64,
    pub top_k: u32,
    /// `None` leaves llama-server's own default in place.
    pub min_p: Option<f64>,
    pub presence_penalty: f64,
    /// llama.cpp DRY sampler strength (0 disables it); penalizes repeating a
    /// phrase verbatim.
    pub dry_multiplier: f64,
}

impl SamplingProfile {
    pub const CORE_CHAT: Self = Self {
        temperature: 0.6,
        top_p: 0.95,
        top_k: 20,
        min_p: Some(0.0),
        presence_penalty: 1.5,
        dry_multiplier: 0.8,
    };

    pub const GENERAL_CHAT: Self = Self {
        temperature: 0.6,
        top_p: 0.95,
        top_k: 20,
        min_p: Some(0.0),
        presence_penalty: 0.0,
        dry_multiplier: 0.0,
    };

    /// Chat profile for a conversation bound to a model of `family` (the GGUF
    /// architecture name recorded in the model registry, e.g. "qwen3").
    pub fn for_chat_model(family: &str) -> Self {
        if family.trim().to_ascii_lowercase().starts_with("qwen3") {
            Self::CORE_CHAT
        } else {
            Self::GENERAL_CHAT
        }
    }

    pub const AGENT: Self = Self {
        temperature: 0.15,
        top_p: 0.85,
        top_k: 20,
        min_p: None,
        presence_penalty: 0.0,
        dry_multiplier: 0.0,
    };

    pub const STRUCTURED: Self = Self {
        temperature: 0.1,
        top_p: 0.85,
        top_k: 20,
        min_p: None,
        presence_penalty: 0.0,
        dry_multiplier: 0.0,
    };

    pub const DETERMINISTIC: Self = Self {
        temperature: 0.0,
        top_p: 1.0,
        top_k: 1,
        min_p: None,
        presence_penalty: 0.0,
        dry_multiplier: 0.0,
    };

    /// The llama-server request fields for this profile. DRY fields are only
    /// sent when DRY is enabled.
    pub fn fields(&self) -> Map<String, Value> {
        let mut fields = Map::new();
        fields.insert("temperature".into(), json!(self.temperature));
        fields.insert("top_p".into(), json!(self.top_p));
        fields.insert("top_k".into(), json!(self.top_k));
        if let Some(min_p) = self.min_p {
            fields.insert("min_p".into(), json!(min_p));
        }
        fields.insert("presence_penalty".into(), json!(self.presence_penalty));
        if self.dry_multiplier > 0.0 {
            fields.insert("dry_multiplier".into(), json!(self.dry_multiplier));
            fields.insert("dry_base".into(), json!(1.75));
            fields.insert("dry_allowed_length".into(), json!(2));
        }
        fields
    }

    /// Sets this profile's fields on a request body, replacing any present.
    pub fn apply(&self, body: &mut Value) {
        if let Some(object) = body.as_object_mut() {
            object.extend(self.fields());
        }
    }

    /// Fills in this profile's fields that the request does not set itself, so
    /// an explicit client choice wins (used for requests forwarded from VS Code).
    pub fn apply_defaults(&self, request: &mut Map<String, Value>) {
        for (key, value) in self.fields() {
            request.entry(key).or_insert(value);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn response_budget_grows_for_writing_but_respects_context() {
        let messages = vec![json!({"role": "user", "content": "Write a short report"})];
        assert_eq!(chat_output_budget("chat", 8_192, &messages), Some(2_048));
        assert_eq!(chat_output_budget("document", 8_192, &messages), Some(4_096));
        assert_eq!(chat_output_budget("pdf", 8_192, &messages), Some(4_096));
        assert_eq!(chat_output_budget("research", 8_192, &messages), Some(4_096));
        assert_eq!(chat_output_budget("thinking", 8_192, &messages), Some(4_096));
        assert_eq!(chat_output_budget("search", 8_192, &messages), Some(3_072));
        assert!(chat_output_budget("document", 4_096, &messages).unwrap() < 4_096);
    }

    #[test]
    fn output_budget_shrinks_for_large_prompts_and_rejects_overflow() {
        let large = vec![json!({"role": "user", "content": "a".repeat(16_000)})];
        let budget = chat_output_budget("document", 8_192, &large).unwrap();
        assert!(budget < 4_096);
        assert!(budget >= 256);

        let full = vec![json!({"role": "user", "content": "a".repeat(24_000)})];
        assert_eq!(chat_output_budget("document", 8_192, &full), None);
    }

    #[test]
    fn image_payloads_are_bounded_by_visual_token_estimate() {
        let messages = vec![json!({"role": "user", "content": [
            {"type": "text", "text": "Review this photo"},
            {"type": "image_url", "image_url": {"url": format!("data:image/png;base64,{}", "A".repeat(100_000))}}
        ]})];
        let budget = chat_output_budget("vision", 8_192, &messages).unwrap();
        assert!(budget > 0);
        assert!(budget <= 2_048);
    }

    #[test]
    fn only_core_chat_uses_repetition_penalties() {
        for profile in [
            SamplingProfile::AGENT,
            SamplingProfile::STRUCTURED,
            SamplingProfile::DETERMINISTIC,
        ] {
            let fields = profile.fields();
            assert_eq!(fields["presence_penalty"], json!(0.0));
            assert!(!fields.contains_key("dry_multiplier"), "{profile:?}");
        }
        let core = SamplingProfile::CORE_CHAT.fields();
        assert_eq!(core["presence_penalty"], json!(1.5));
        assert_eq!(core["dry_multiplier"], json!(0.8));
        assert_eq!(core["dry_allowed_length"], json!(2));
    }

    #[test]
    fn chat_penalties_apply_to_qwen3_only() {
        assert_eq!(SamplingProfile::for_chat_model("qwen3"), SamplingProfile::CORE_CHAT);
        assert_eq!(SamplingProfile::for_chat_model("Qwen3"), SamplingProfile::CORE_CHAT);
        for family in ["nemotron_h", "llama", "gemma3", "qwen2vl", "phi3", ""] {
            let profile = SamplingProfile::for_chat_model(family);
            assert_eq!(profile, SamplingProfile::GENERAL_CHAT, "{family}");
            assert!(!profile.fields().contains_key("dry_multiplier"));
            assert_eq!(profile.fields()["presence_penalty"], json!(0.0));
        }
    }

    #[test]
    fn apply_replaces_but_apply_defaults_keeps_client_choices() {
        let mut body = json!({"temperature": 0.9, "messages": []});
        SamplingProfile::AGENT.apply(&mut body);
        assert_eq!(body["temperature"], json!(0.15));
        assert!(body["messages"].is_array());

        let mut request = Map::new();
        request.insert("temperature".into(), json!(0.7));
        SamplingProfile::AGENT.apply_defaults(&mut request);
        assert_eq!(request["temperature"], json!(0.7));
        assert_eq!(request["top_k"], json!(20));
    }
}
