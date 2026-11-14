use chrono::Utc;
use rusqlite::{params, OptionalExtension};
use serde::{Deserialize, Serialize};

use crate::{app_error::AppError, database::Database};

const SETTINGS_KEY: &str = "app.preferences";
const USER_PROFILE_KEY: &str = "app.user_profile";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum ThemeMode {
    System,
    Dark,
    Light,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
#[serde(rename_all = "camelCase")]
pub struct AppPreferences {
    pub theme: ThemeMode,
    pub language: String,
    pub compact_sidebar: bool,
    pub enter_to_send: bool,
    pub show_shortcut_hints: bool,
    pub auto_generate_titles: bool,
    pub code_copy_buttons: bool,
    pub markdown_rendering: bool,
    pub default_performance_profile: String,
    /// Compatibility only. This release has no usage-telemetry pipeline and
    /// must never treat a historical stored true value as user consent.
    pub telemetry_enabled: bool,
    /// Compatibility field: this release always persists local chat history.
    pub save_chat_history: bool,
    /// Remains set until an affected user explicitly acknowledges the correction.
    pub chat_history_notice_required: bool,
    pub local_runtime_autostart: bool,
    /// Initial setup choice: "cloud", "local", or "auto" (Both).
    pub ai_execution_mode: String,
    /// Separate coding-agent preference; starts with the same onboarding choice.
    pub coding_agent_execution_mode: String,
    pub confirm_before_delete: bool,
    pub web_search_enabled: bool,
    pub deep_research_enabled: bool,
    /// "auto", "brave", "google", "duckduckgo", or "bing"; other providers remain fallbacks.
    pub web_search_preferred_provider: String,
    pub thinking_mode_enabled: bool,
    pub typing_indicator_enabled: bool,
    pub socket_realtime_enabled: bool,
    pub socket_url: String,
    pub open_artifacts_after_generation: bool,
    pub auto_check_app_updates: bool,
    pub auto_download_app_updates: bool,
    pub notify_model_updates: bool,
    /// Compatibility field. Model downloads require manual confirmation.
    pub auto_download_model_updates: bool,
    pub update_channel: String,
    pub openagent_model_id: String,
    pub openagent_approval_mode: String,
    pub openagent_sandbox_mode: String,
    pub openagent_max_parallel_agents: u8,
    /// "automatic", "shared" or "dedicated"; see agent_runtime::decide_placement.
    pub openagent_runtime_mode: String,
    pub coding_enabled: bool,
    pub coding_token_budget: i64,
    pub coding_runtime_budget_minutes: i64,
    pub coding_max_parallel_workers: u8,
    pub coding_ci_repair_limit: u8,
    pub coding_context_size: u32,
    pub coding_gpu_layers: i32,
    pub coding_network_enabled: bool,
    pub coding_autonomy: String,
    /// "direct" (default: ignore the PC proxy), "manual" or "system"; see net.rs.
    pub network_proxy_mode: String,
    pub network_proxy_url: String,
    pub network_proxy_credential_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
#[serde(rename_all = "camelCase")]
pub struct UserProfile {
    pub full_name: String,
    pub email: String,
    pub about: String,
    pub occupation: String,
    pub preferred_name: String,
    pub response_style: String,
    pub custom_instructions: String,
    pub avatar_data_url: String,
}

impl UserProfile {
    pub fn to_system_context(&self) -> Option<String> {
        let mut lines = vec![
            "You are OpenMindAI, a local AI assistant. Your own name is OpenMindAI — always introduce \
             and refer to yourself that way, never by the user's name."
                .to_string(),
        ];

        let mut profile_lines = Vec::new();
        push_context_line(&mut profile_lines, "User's name", &self.full_name);
        push_context_line(
            &mut profile_lines,
            "User's preferred name",
            &self.preferred_name,
        );
        push_context_line(&mut profile_lines, "User's email", &self.email);
        push_context_line(&mut profile_lines, "About the user", &self.about);
        push_context_line(&mut profile_lines, "User's occupation", &self.occupation);
        push_context_line(
            &mut profile_lines,
            "Preferred response style",
            &self.response_style,
        );
        push_context_line(
            &mut profile_lines,
            "Custom instructions from the user",
            &self.custom_instructions,
        );

        if !profile_lines.is_empty() {
            lines.push(
                "The following describes the user you are assisting — it is information about \
                        them, not about you:"
                    .to_string(),
            );
            lines.extend(profile_lines);
        }

        Some(lines.join("\n"))
    }
}

impl Default for AppPreferences {
    fn default() -> Self {
        Self {
            theme: ThemeMode::Light,
            language: "English".to_string(),
            compact_sidebar: false,
            enter_to_send: true,
            show_shortcut_hints: true,
            auto_generate_titles: true,
            code_copy_buttons: true,
            markdown_rendering: true,
            default_performance_profile: "Auto".to_string(),
            telemetry_enabled: false,
            save_chat_history: true,
            chat_history_notice_required: false,
            local_runtime_autostart: true,
            ai_execution_mode: "auto".to_string(),
            // Existing installs used only local OpenAgent; require an explicit
            // Cloud/Both choice before sending project context remotely.
            coding_agent_execution_mode: "local".to_string(),
            confirm_before_delete: true,
            web_search_enabled: true,
            deep_research_enabled: true,
            web_search_preferred_provider: "auto".to_string(),
            thinking_mode_enabled: true,
            typing_indicator_enabled: true,
            socket_realtime_enabled: false,
            socket_url: "http://127.0.0.1:3001".to_string(),
            open_artifacts_after_generation: false,
            auto_check_app_updates: true,
            auto_download_app_updates: false,
            notify_model_updates: true,
            auto_download_model_updates: false,
            update_channel: "Stable".to_string(),
            openagent_model_id: String::new(),
            openagent_approval_mode: "risk_based".to_string(),
            openagent_sandbox_mode: "attached_workspace".to_string(),
            openagent_max_parallel_agents: 1,
            openagent_runtime_mode: "automatic".to_string(),
            coding_enabled: true,
            coding_token_budget: 0,
            coding_runtime_budget_minutes: 0,
            coding_max_parallel_workers: 2,
            coding_ci_repair_limit: 2,
            coding_context_size: 8_192,
            coding_gpu_layers: -1,
            coding_network_enabled: false,
            coding_autonomy: "bounded".to_string(),
            network_proxy_mode: "direct".to_string(),
            network_proxy_url: String::new(),
            network_proxy_credential_id: None,
        }
    }
}

pub struct SettingsRepository<'a> {
    database: &'a Database,
}

impl<'a> SettingsRepository<'a> {
    pub fn new(database: &'a Database) -> Self {
        Self { database }
    }

    fn load_saved_preferences(&self) -> Result<Option<AppPreferences>, AppError> {
        let value: Option<String> = self
            .database
            .connection()
            .query_row(
                "SELECT value_json FROM app_settings WHERE key = ?1",
                params![SETTINGS_KEY],
                |row| row.get(0),
            )
            .optional()?;

        value
            .map(|value| {
                serde_json::from_str(&value).map_err(|error| AppError::internal(error.to_string()))
            })
            .transpose()
    }

    pub fn get_preferences(&self) -> Result<AppPreferences, AppError> {
        match self.load_saved_preferences()? {
            Some(mut preferences) => {
                let serialized = serde_json::to_value(&preferences)
                    .map_err(|_| AppError::internal("Invalid preferences"))?;
                let mut sanitized = serialized.clone();
                crate::proxy_credentials::protect(
                    &mut sanitized,
                    Some(&serialized),
                    crate::github::secret_store::set_secret,
                )?;
                if sanitized != serialized {
                    crate::proxy_credentials::migrate(
                        self.database.connection(),
                        &serialized,
                        &sanitized,
                    )?;
                    preferences = serde_json::from_value(sanitized)
                        .map_err(|_| AppError::internal("Invalid preferences"))?;
                }
                if !preferences.save_chat_history {
                    // The old flag never stopped persistence. Correct the reported
                    // state and keep a durable, visible notice; do not delete chats.
                    preferences.save_chat_history = true;
                    preferences.chat_history_notice_required = true;
                    self.database.connection().execute(
                        "UPDATE app_settings
                         SET value_json = json_set(value_json,
                             '$.saveChatHistory', json('true'),
                             '$.chatHistoryNoticeRequired', json('true')),
                             updated_at = ?1
                         WHERE key = ?2",
                        params![Utc::now().to_rfc3339(), SETTINGS_KEY],
                    )?;
                }
                if preferences.telemetry_enabled {
                    // The Privacy toggle historically saved an inert boolean.
                    // Reset it so no later release can misinterpret it as valid
                    // informed consent to data collection or transmission.
                    self.database.connection().execute(
                        "UPDATE app_settings
                         SET value_json = json_set(value_json, '$.telemetryEnabled', json('false')),
                             updated_at = ?1
                         WHERE key = ?2",
                        params![Utc::now().to_rfc3339(), SETTINGS_KEY],
                    )?;
                    preferences.telemetry_enabled = false;
                }
                if preferences.auto_download_model_updates {
                    // No scheduler implements this legacy preference. Update only
                    // this field so notification choices and future fields survive.
                    self.database.connection().execute(
                        "UPDATE app_settings
                         SET value_json = json_set(value_json, '$.autoDownloadModelUpdates', json('false')),
                             updated_at = ?1
                         WHERE key = ?2",
                        params![Utc::now().to_rfc3339(), SETTINGS_KEY],
                    )?;
                    preferences.auto_download_model_updates = false;
                }
                Ok(preferences)
            }
            None => {
                let preferences = AppPreferences::default();
                self.save_preferences(&preferences)?;
                Ok(preferences)
            }
        }
    }

    pub fn save_preferences(
        &self,
        preferences: &AppPreferences,
    ) -> Result<AppPreferences, AppError> {
        if preferences.auto_download_model_updates {
            return Err(AppError::ModelUnsupported(
                "Automatic model downloading is not available. Confirm model downloads manually in Settings → Models.".into(),
            ));
        }
        if !preferences.save_chat_history {
            return Err(AppError::internal(
                "This version saves chat history locally. Temporary chats are not supported; history saving cannot be disabled.",
            ));
        }
        if !["auto", "local", "cloud"].contains(&preferences.ai_execution_mode.as_str())
            || !["auto", "local", "cloud"]
                .contains(&preferences.coding_agent_execution_mode.as_str())
        {
            return Err(AppError::internal("Invalid Cloud/Local execution mode"));
        }
        let mut saved = preferences.clone();
        // Telemetry consent cannot be solicited by this build: no collector
        // or privacy flow implements this legacy flag. Do not persist true,
        // including when an old UI/client sends it during an unrelated save.
        saved.telemetry_enabled = false;
        if let Some(existing) = self.load_saved_preferences()? {
            // Ordinary preference saves (including older clients) cannot silently
            // dismiss the notice. Acknowledgment has a separate, narrow operation.
            saved.chat_history_notice_required |=
                existing.chat_history_notice_required || !existing.save_chat_history;
        }
        let previous = self
            .load_saved_preferences()?
            .map(serde_json::to_value)
            .transpose()
            .map_err(|_| AppError::internal("Invalid preferences"))?;
        let mut sanitized =
            serde_json::to_value(&saved).map_err(|_| AppError::internal("Invalid preferences"))?;
        crate::proxy_credentials::protect(
            &mut sanitized,
            previous.as_ref(),
            crate::github::secret_store::set_secret,
        )?;
        self.database
            .connection()
            .execute_batch("PRAGMA secure_delete = ON;")?;
        self.save_json(SETTINGS_KEY, &sanitized)?;
        serde_json::from_value(sanitized).map_err(|_| AppError::internal("Invalid preferences"))
    }

    pub fn acknowledge_chat_history_notice(&self) -> Result<AppPreferences, AppError> {
        self.get_preferences()?;
        // Update only this field atomically, including when another app process
        // saves unrelated preferences between the read and acknowledgment.
        self.database.connection().execute(
            "UPDATE app_settings
             SET value_json = json_set(value_json, '$.chatHistoryNoticeRequired', json('false')),
                 updated_at = ?1
             WHERE key = ?2",
            params![Utc::now().to_rfc3339(), SETTINGS_KEY],
        )?;
        self.get_preferences()
    }

    pub fn get_user_profile(&self) -> Result<UserProfile, AppError> {
        let value: Option<String> = self
            .database
            .connection()
            .query_row(
                "SELECT value_json FROM app_settings WHERE key = ?1",
                params![USER_PROFILE_KEY],
                |row| row.get(0),
            )
            .optional()?;

        match value {
            Some(value) => {
                serde_json::from_str(&value).map_err(|error| AppError::internal(error.to_string()))
            }
            None => {
                let profile = UserProfile::default();
                self.save_user_profile(&profile)?;
                Ok(profile)
            }
        }
    }

    pub fn save_user_profile(&self, profile: &UserProfile) -> Result<UserProfile, AppError> {
        self.save_json(USER_PROFILE_KEY, profile)?;
        Ok(profile.clone())
    }

    fn save_json<T: Serialize>(&self, key: &str, value: &T) -> Result<(), AppError> {
        let now = Utc::now().to_rfc3339();
        let value =
            serde_json::to_string(value).map_err(|error| AppError::internal(error.to_string()))?;
        self.database.connection().execute(
            "INSERT INTO app_settings (key, value_json, updated_at)
             VALUES (?1, ?2, ?3)
             ON CONFLICT(key) DO UPDATE SET value_json = excluded.value_json, updated_at = excluded.updated_at",
            params![key, value, now],
        )?;
        Ok(())
    }
}

fn push_context_line(lines: &mut Vec<String>, label: &str, value: &str) {
    let value = value.trim();
    if !value.is_empty() {
        lines.push(format!("{label}: {value}"));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preferences_round_trip() {
        let database = Database::in_memory().unwrap();
        let repo = SettingsRepository::new(&database);
        let mut preferences = repo.get_preferences().unwrap();
        preferences.theme = ThemeMode::Light;
        preferences.enter_to_send = false;
        repo.save_preferences(&preferences).unwrap();

        let loaded = repo.get_preferences().unwrap();
        assert_eq!(loaded.theme, ThemeMode::Light);
        assert!(!loaded.enter_to_send);
    }

    #[test]
    fn obsolete_telemetry_toggle_never_grants_or_persists_consent() {
        let database = Database::in_memory().unwrap();
        let repo = SettingsRepository::new(&database);
        let mut requested = repo.get_preferences().unwrap();
        requested.language = "Russian".to_string();
        requested.telemetry_enabled = true;

        // Older UI clients can still submit the compatibility field, but it
        // must not enable a feature without an actual consent-gated collector.
        let saved = repo.save_preferences(&requested).unwrap();
        assert!(!saved.telemetry_enabled);
        assert_eq!(saved.language, "Russian");
        let stored: String = database
            .connection()
            .query_row(
                "SELECT value_json FROM app_settings WHERE key = ?1",
                params![SETTINGS_KEY],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&stored).unwrap()["telemetryEnabled"],
            false
        );

        // A profile saved by an older version may contain a phantom true.
        // Loading corrects that value durably while preserving other choices.
        database
            .connection()
            .execute(
                "UPDATE app_settings
                 SET value_json = json_set(value_json, '$.telemetryEnabled', json('true'))
                 WHERE key = ?1",
                params![SETTINGS_KEY],
            )
            .unwrap();
        let normalized = repo.get_preferences().unwrap();
        assert!(!normalized.telemetry_enabled);
        assert_eq!(normalized.language, "Russian");
        let reloaded = repo.get_preferences().unwrap();
        assert!(!reloaded.telemetry_enabled);
        let persisted: String = database
            .connection()
            .query_row(
                "SELECT value_json FROM app_settings WHERE key = ?1",
                params![SETTINGS_KEY],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&persisted).unwrap()["telemetryEnabled"],
            false
        );
    }

    #[test]
    fn old_preferences_json_loads_with_defaults() {
        let database = Database::in_memory().unwrap();
        let repo = SettingsRepository::new(&database);
        database
            .connection()
            .execute(
                "INSERT INTO app_settings (key, value_json, updated_at) VALUES (?1, ?2, ?3)",
                params![
                    SETTINGS_KEY,
                    r#"{"theme":"dark","language":"English"}"#,
                    Utc::now().to_rfc3339()
                ],
            )
            .unwrap();

        let loaded = repo.get_preferences().unwrap();
        assert!(loaded.thinking_mode_enabled);
        assert_eq!(loaded.socket_url, "http://127.0.0.1:3001");
        // Update preferences (added after this JSON shape existed) must
        // still default correctly for an already-installed user.
        assert!(loaded.auto_check_app_updates);
        assert!(!loaded.auto_download_app_updates);
        assert!(loaded.notify_model_updates);
        assert!(!loaded.auto_download_model_updates);
        assert_eq!(loaded.update_channel, "Stable");
        assert_eq!(loaded.network_proxy_mode, "direct");
        assert!(loaded.network_proxy_url.is_empty());
        assert!(loaded.save_chat_history);
        assert!(!loaded.chat_history_notice_required);
    }

    fn seed_unsupported_history_setting(database: &Database) {
        database.connection().execute(
            "INSERT INTO app_settings (key, value_json, updated_at) VALUES (?1, ?2, ?3)",
            params![SETTINGS_KEY, r#"{"saveChatHistory":false,"theme":"dark","language":"Bangla","enterToSend":false}"#, Utc::now().to_rfc3339()],
        ).unwrap();
    }

    #[test]
    fn unsupported_history_off_is_rejected_without_changing_saved_preferences() {
        let database = Database::in_memory().unwrap();
        let repo = SettingsRepository::new(&database);
        let original = repo.get_preferences().unwrap();
        let mut requested = original.clone();
        requested.save_chat_history = false;
        requested.language = "should not be saved".into();
        let error = repo.save_preferences(&requested).unwrap_err().to_string();
        assert!(error.contains("Temporary chats are not supported"));
        let actual = repo.get_preferences().unwrap();
        assert_eq!(
            serde_json::to_value(actual).unwrap(),
            serde_json::to_value(original).unwrap()
        );
    }

    #[test]
    fn legacy_off_setting_is_corrected_with_a_durable_notice_and_preserved_data() {
        use crate::chat::ChatRepository;
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("history.db");
        let conversation_id;
        {
            let database = Database::open(path.clone()).unwrap();
            seed_unsupported_history_setting(&database);
            let chat = ChatRepository::new(&database);
            let conversation = chat.create_conversation(Some("Existing chat")).unwrap();
            conversation_id = conversation.id;
            chat.add_message(
                &conversation_id,
                "user",
                "fixture text with attachment context",
                "completed",
                None,
            )
            .unwrap();
            chat.add_message(
                &conversation_id,
                "assistant",
                "fixture failed response",
                "failed",
                None,
            )
            .unwrap();
            let preferences = SettingsRepository::new(&database)
                .get_preferences()
                .unwrap();
            assert!(preferences.save_chat_history);
            assert!(preferences.chat_history_notice_required);
            assert_eq!(preferences.language, "Bangla");
            assert_eq!(preferences.theme, ThemeMode::Dark);
            assert!(!preferences.enter_to_send);
            assert_eq!(chat.list_messages(&conversation_id).unwrap().len(), 2);
        }
        let reopened = Database::open(path).unwrap();
        let preferences = SettingsRepository::new(&reopened)
            .get_preferences()
            .unwrap();
        assert!(preferences.save_chat_history);
        assert!(preferences.chat_history_notice_required);
        let messages = ChatRepository::new(&reopened)
            .list_messages(&conversation_id)
            .unwrap();
        assert_eq!(messages[0].content, "fixture text with attachment context");
        assert_eq!(messages[1].status, "failed");
    }

    #[test]
    fn ordinary_preference_saves_cannot_acknowledge_the_history_notice() {
        let database = Database::in_memory().unwrap();
        seed_unsupported_history_setting(&database);
        let repo = SettingsRepository::new(&database);
        let mut preferences = repo.get_preferences().unwrap();
        preferences.chat_history_notice_required = false;
        preferences.compact_sidebar = true;
        assert!(
            repo.save_preferences(&preferences)
                .unwrap()
                .chat_history_notice_required
        );
        let loaded = repo.get_preferences().unwrap();
        assert!(loaded.chat_history_notice_required);
        assert!(loaded.compact_sidebar);
    }

    #[test]
    fn acknowledgment_updates_only_current_notice_and_survives_restart() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("preferences.db");
        {
            let database = Database::open(path.clone()).unwrap();
            seed_unsupported_history_setting(&database);
            let repo = SettingsRepository::new(&database);
            let mut latest = repo.get_preferences().unwrap();
            latest.language = "Russian".into();
            latest.compact_sidebar = true;
            repo.save_preferences(&latest).unwrap();
            database.connection().execute(
                "UPDATE app_settings SET value_json = json_set(value_json, '$.futurePreference', 'keep-me') WHERE key = ?1",
                params![SETTINGS_KEY],
            ).unwrap();
            let acknowledged = repo.acknowledge_chat_history_notice().unwrap();
            assert!(!acknowledged.chat_history_notice_required);
            assert!(acknowledged.save_chat_history);
            assert_eq!(acknowledged.language, "Russian");
            assert!(acknowledged.compact_sidebar);
            let future: String = database.connection().query_row(
                "SELECT json_extract(value_json, '$.futurePreference') FROM app_settings WHERE key = ?1",
                params![SETTINGS_KEY],
                |row| row.get(0),
            ).unwrap();
            assert_eq!(future, "keep-me");
        }
        let database = Database::open(path).unwrap();
        let loaded = SettingsRepository::new(&database)
            .get_preferences()
            .unwrap();
        assert!(!loaded.chat_history_notice_required);
        assert!(loaded.save_chat_history);
        assert_eq!(loaded.language, "Russian");
    }

    #[test]
    fn failed_migration_or_acknowledgment_does_not_hide_the_notice() {
        let database = Database::in_memory().unwrap();
        seed_unsupported_history_setting(&database);
        let repo = SettingsRepository::new(&database);
        database
            .connection()
            .pragma_update(None, "query_only", true)
            .unwrap();
        assert!(repo.get_preferences().is_err());
        database
            .connection()
            .pragma_update(None, "query_only", false)
            .unwrap();
        assert!(repo.get_preferences().unwrap().chat_history_notice_required);
        database
            .connection()
            .pragma_update(None, "query_only", true)
            .unwrap();
        assert!(repo.acknowledge_chat_history_notice().is_err());
        database
            .connection()
            .pragma_update(None, "query_only", false)
            .unwrap();
        assert!(repo.get_preferences().unwrap().chat_history_notice_required);
    }

    #[test]
    fn automatic_model_download_requests_are_rejected_without_partial_saves() {
        let database = Database::in_memory().unwrap();
        let repo = SettingsRepository::new(&database);
        let original = repo.get_preferences().unwrap();
        let mut requested = original.clone();
        requested.auto_download_model_updates = true;
        requested.notify_model_updates = false;
        requested.language = "must not be saved".into();
        let error = repo.save_preferences(&requested).unwrap_err();
        assert!(matches!(error, AppError::ModelUnsupported(_)));
        assert!(error.to_string().contains("Settings → Models"));
        assert_eq!(
            serde_json::to_value(repo.get_preferences().unwrap()).unwrap(),
            serde_json::to_value(original).unwrap()
        );
    }

    #[test]
    fn legacy_model_download_flag_is_corrected_once_and_survives_restart() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("model-updates.db");
        {
            let database = Database::open(path.clone()).unwrap();
            database.connection().execute(
                "INSERT INTO app_settings (key, value_json, updated_at) VALUES (?1, ?2, 'legacy')",
                params![SETTINGS_KEY, r#"{"autoDownloadModelUpdates":true,"notifyModelUpdates":false,"autoDownloadAppUpdates":true,"chatHistoryNoticeRequired":true,"futurePreference":"keep-me"}"#],
            ).unwrap();
            let repo = SettingsRepository::new(&database);
            let loaded = repo.get_preferences().unwrap();
            assert!(!loaded.auto_download_model_updates);
            assert!(!loaded.notify_model_updates);
            assert!(loaded.auto_download_app_updates);
            assert!(loaded.chat_history_notice_required);
            let persisted: String = database
                .connection()
                .query_row(
                    "SELECT value_json FROM app_settings WHERE key = ?1",
                    params![SETTINGS_KEY],
                    |row| row.get(0),
                )
                .unwrap();
            let json: serde_json::Value = serde_json::from_str(&persisted).unwrap();
            assert_eq!(json["autoDownloadModelUpdates"], false);
            assert_eq!(json["futurePreference"], "keep-me");
            // A second read must not perform a needless write.
            database
                .connection()
                .pragma_update(None, "query_only", true)
                .unwrap();
            assert!(!repo.get_preferences().unwrap().auto_download_model_updates);
        }
        let database = Database::open(path).unwrap();
        let loaded = SettingsRepository::new(&database)
            .get_preferences()
            .unwrap();
        assert!(!loaded.auto_download_model_updates);
        assert!(!loaded.notify_model_updates);
        assert!(loaded.auto_download_app_updates);
    }

    #[test]
    fn model_notification_and_app_download_choices_remain_independent() {
        let database = Database::in_memory().unwrap();
        let repo = SettingsRepository::new(&database);
        let mut preferences = repo.get_preferences().unwrap();
        for notify in [false, true] {
            preferences.notify_model_updates = notify;
            preferences.auto_download_app_updates = !notify;
            repo.save_preferences(&preferences).unwrap();
            let loaded = repo.get_preferences().unwrap();
            assert_eq!(loaded.notify_model_updates, notify);
            assert_eq!(loaded.auto_download_app_updates, !notify);
            assert!(!loaded.auto_download_model_updates);
        }
    }

    #[test]
    fn failed_legacy_model_flag_correction_is_reported_and_can_retry() {
        let database = Database::in_memory().unwrap();
        database
            .connection()
            .execute(
                "INSERT INTO app_settings (key, value_json, updated_at) VALUES (?1, ?2, 'legacy')",
                params![SETTINGS_KEY, r#"{"autoDownloadModelUpdates":true}"#],
            )
            .unwrap();
        let repo = SettingsRepository::new(&database);
        database
            .connection()
            .pragma_update(None, "query_only", true)
            .unwrap();
        assert!(repo.get_preferences().is_err());
        let raw: bool = database.connection().query_row(
            "SELECT json_extract(value_json, '$.autoDownloadModelUpdates') FROM app_settings WHERE key = ?1",
            params![SETTINGS_KEY], |row| row.get(0),
        ).unwrap();
        assert!(raw);
        database
            .connection()
            .pragma_update(None, "query_only", false)
            .unwrap();
        assert!(!repo.get_preferences().unwrap().auto_download_model_updates);
    }

    #[test]
    fn user_profile_round_trip_and_context() {
        let database = Database::in_memory().unwrap();
        let repo = SettingsRepository::new(&database);
        let profile = UserProfile {
            full_name: "Sam".to_string(),
            email: "sam@example.com".to_string(),
            about: "Likes local AI".to_string(),
            occupation: "Engineer".to_string(),
            preferred_name: "Sam".to_string(),
            response_style: "Concise".to_string(),
            custom_instructions: "Use practical examples".to_string(),
            avatar_data_url: String::new(),
        };

        repo.save_user_profile(&profile).unwrap();
        let loaded = repo.get_user_profile().unwrap();
        assert_eq!(loaded.email, "sam@example.com");
        assert!(loaded
            .to_system_context()
            .unwrap()
            .contains("Preferred response style: Concise"));
    }
}
