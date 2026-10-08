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

/// Response budget for one Core chat turn.
pub const CORE_CHAT_MAX_TOKENS: u32 = 768;

use serde_json::{json, Map, Value};

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
