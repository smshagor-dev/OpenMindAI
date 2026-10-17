//! Search credentials are stored in the existing OS credential backend.
//! Only presence booleans are returned to the frontend, never secret values.
use serde::Serialize;
use crate::{app_error::AppError, github::secret_store};

const BRAVE_SLOT: &str = "web-search-brave-key";
const GOOGLE_KEY_SLOT: &str = "web-search-google-key";
const GOOGLE_ENGINE_SLOT: &str = "web-search-google-engine";

#[derive(Default, Clone)]
pub(crate) struct SearchCredentials {
    pub brave: Option<String>,
    pub google_key: Option<String>,
    pub google_engine: Option<String>,
}
impl SearchCredentials {
    pub fn brave_ready(&self) -> bool { self.brave.is_some() }
    pub fn google_ready(&self) -> bool {
        self.google_key.is_some() && self.google_engine.is_some()
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ProviderConfiguration {
    pub brave_configured: bool,
    pub google_configured: bool,
    pub google_key_configured: bool,
    pub google_engine_configured: bool,
}

fn optional_value(slot: &str, env: &str) -> Result<Option<String>, AppError> {
    // Credentials from the OS store take precedence over inherited environment
    // variables. Empty values are treated as unconfigured.
    Ok(secret_store::get_secret(slot)?.filter(|v| !v.trim().is_empty())
        .or_else(|| std::env::var(env).ok().filter(|v| !v.trim().is_empty())))
}
pub(crate) fn load_credentials() -> Result<SearchCredentials, AppError> {
    Ok(SearchCredentials {
        brave: optional_value(BRAVE_SLOT, "OPENMINDAI_BRAVE_SEARCH_KEY")?,
        google_key: optional_value(GOOGLE_KEY_SLOT, "OPENMINDAI_GOOGLE_CSE_KEY")?,
        google_engine: optional_value(GOOGLE_ENGINE_SLOT, "OPENMINDAI_GOOGLE_CSE_ID")?,
    })
}
pub(crate) fn credential_status() -> Result<ProviderConfiguration, AppError> {
    let c = load_credentials()?;
    Ok(ProviderConfiguration {
        brave_configured: c.brave_ready(),
        google_configured: c.google_ready(),
        google_key_configured: c.google_key.is_some(),
        google_engine_configured: c.google_engine.is_some(),
    })
}
pub(crate) fn update_credential(field: &str, value: &str) -> Result<(), AppError> {
    let slot = match field {
        "brave" => BRAVE_SLOT,
        "googleKey" => GOOGLE_KEY_SLOT,
        "googleEngine" => GOOGLE_ENGINE_SLOT,
        _ => return Err(AppError::internal("unsupported search credential field")),
    };
    let value = value.trim();
    if value.len() > 500 || value.contains('\n') || value.contains('\r') {
        return Err(AppError::internal("search credential is invalid or too long"));
    }
    if value.is_empty() { secret_store::delete_secret(slot) }
    else { secret_store::set_secret(slot, value) }
}

pub(crate) fn valid_preference(value: &str) -> bool {
    matches!(value, "auto" | "brave" | "google" | "duckduckgo" | "bing")
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn provider_preference_is_allowlisted() {
        for good in ["auto", "brave", "google", "duckduckgo", "bing"] {
            assert!(valid_preference(good));
        }
        for bad in ["", "http://127.0.0.1", "something", "GOOGLE"] {
            assert!(!valid_preference(bad));
        }
    }
}
