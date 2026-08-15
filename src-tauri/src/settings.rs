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
    pub telemetry_enabled: bool,
    pub save_chat_history: bool,
    pub local_runtime_autostart: bool,
    pub confirm_before_delete: bool,
    pub web_search_enabled: bool,
    pub deep_research_enabled: bool,
    pub thinking_mode_enabled: bool,
    pub typing_indicator_enabled: bool,
    pub socket_realtime_enabled: bool,
    pub socket_url: String,
    pub open_artifacts_after_generation: bool,
    pub auto_check_app_updates: bool,
    pub auto_download_app_updates: bool,
    pub notify_model_updates: bool,
    pub auto_download_model_updates: bool,
    pub update_channel: String,
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
            local_runtime_autostart: false,
            confirm_before_delete: true,
            web_search_enabled: true,
            deep_research_enabled: true,
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

    pub fn get_preferences(&self) -> Result<AppPreferences, AppError> {
        let value: Option<String> = self
            .database
            .connection()
            .query_row(
                "SELECT value_json FROM app_settings WHERE key = ?1",
                params![SETTINGS_KEY],
                |row| row.get(0),
            )
            .optional()?;

        match value {
            Some(value) => {
                serde_json::from_str(&value).map_err(|error| AppError::internal(error.to_string()))
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
        self.save_json(SETTINGS_KEY, preferences)?;
        Ok(preferences.clone())
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
