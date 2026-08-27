pub(crate) mod secret_store;

use chrono::Utc;
use reqwest::{header, Client};
use rusqlite::{params, OptionalExtension};
use serde::{Deserialize, Serialize};

use crate::{app_error::AppError, database::Database};

const SETTINGS_KEY: &str = "app.github";
const SECRET_SLOT: &str = "github-token";
const USER_AGENT: &str = "OpenMindAI-Desktop";

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GithubAccount {
    pub login: String,
    pub name: Option<String>,
    pub avatar_url: Option<String>,
    pub html_url: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GithubRepoSummary {
    pub id: i64,
    pub name: String,
    pub full_name: String,
    pub private: bool,
    pub stargazers_count: i64,
    pub html_url: String,
    pub description: Option<String>,
    pub updated_at: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GithubIssueSummary {
    pub id: i64,
    pub number: i64,
    pub title: String,
    pub state: String,
    pub html_url: String,
    pub is_pull_request: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct StoredGithubAccount {
    account: GithubAccount,
}

#[derive(Debug, Deserialize)]
struct GithubUserResponse {
    login: String,
    name: Option<String>,
    avatar_url: Option<String>,
    html_url: String,
}

#[derive(Debug, Deserialize)]
struct GithubRepoResponse {
    id: i64,
    name: String,
    full_name: String,
    private: bool,
    stargazers_count: i64,
    html_url: String,
    description: Option<String>,
    updated_at: Option<String>,
}

#[derive(Debug, Deserialize)]
struct GithubIssueResponse {
    id: i64,
    number: i64,
    title: String,
    state: String,
    html_url: String,
    pull_request: Option<serde_json::Value>,
}

pub struct GithubRepository<'a> {
    database: &'a Database,
}

impl<'a> GithubRepository<'a> {
    pub fn new(database: &'a Database) -> Self {
        Self { database }
    }

    pub fn get_account(&self) -> Result<Option<GithubAccount>, AppError> {
        let Some((account, legacy_token)) = self.get_stored()? else {
            return Ok(None);
        };

        if let Some(token) = legacy_token {
            self.migrate_legacy(&account, &token)?;
        }
        Ok(Some(account))
    }

    pub fn get_token(&self) -> Result<Option<String>, AppError> {
        if let Some(token) = secret_store::get_secret(SECRET_SLOT)? {
            return Ok(Some(token));
        }

        let Some((account, legacy_token)) = self.get_stored()? else {
            return Ok(None);
        };
        let Some(token) = legacy_token else {
            return Ok(None);
        };

        self.migrate_legacy(&account, &token)?;
        Ok(Some(token))
    }

    fn get_stored(&self) -> Result<Option<(GithubAccount, Option<String>)>, AppError> {
        let value: Option<String> = self
            .database
            .connection()
            .query_row(
                "SELECT value_json FROM app_settings WHERE key = ?1",
                params![SETTINGS_KEY],
                |row| row.get(0),
            )
            .optional()?;

        let Some(value) = value else {
            return Ok(None);
        };
        let payload: serde_json::Value = serde_json::from_str(&value)
            .map_err(|error| AppError::internal(format!("invalid GitHub settings: {error}")))?;
        let account_value = payload
            .get("account")
            .cloned()
            .ok_or_else(|| AppError::internal("GitHub settings are missing account metadata"))?;
        let account: GithubAccount = serde_json::from_value(account_value)
            .map_err(|error| AppError::internal(format!("invalid GitHub account metadata: {error}")))?;
        let legacy_token = payload
            .get("token")
            .and_then(serde_json::Value::as_str)
            .filter(|token| !token.is_empty())
            .map(ToOwned::to_owned);
        Ok(Some((account, legacy_token)))
    }

    fn persist_account(&self, account: &GithubAccount) -> Result<(), AppError> {
        let payload = serde_json::to_string(&StoredGithubAccount {
            account: account.clone(),
        })
        .map_err(|error| AppError::internal(error.to_string()))?;
        let now = Utc::now().to_rfc3339();
        self.database.connection().execute(
            "INSERT INTO app_settings (key, value_json, updated_at)
             VALUES (?1, ?2, ?3)
             ON CONFLICT(key) DO UPDATE SET value_json = excluded.value_json, updated_at = excluded.updated_at",
            params![SETTINGS_KEY, payload, now],
        )?;
        Ok(())
    }

    fn migrate_legacy(&self, account: &GithubAccount, token: &str) -> Result<(), AppError> {
        secret_store::set_secret(SECRET_SLOT, token)?;
        self.persist_account(account)?;
        tracing::info!("migrated GitHub credential from SQLite to the OS credential store");
        Ok(())
    }

    pub fn save(&self, token: &str, account: &GithubAccount) -> Result<(), AppError> {
        if token.trim().is_empty() {
            return Err(AppError::GithubApiError(
                "GitHub token cannot be empty".to_string(),
            ));
        }
        secret_store::set_secret(SECRET_SLOT, token)?;
        self.persist_account(account)
    }

    pub fn clear(&self) -> Result<(), AppError> {
        secret_store::delete_secret(SECRET_SLOT)?;
        self.database.connection().execute(
            "DELETE FROM app_settings WHERE key = ?1",
            params![SETTINGS_KEY],
        )?;
        Ok(())
    }
}

fn auth_headers(token: &str) -> Result<header::HeaderMap, AppError> {
    let mut headers = header::HeaderMap::new();
    let mut auth_value = header::HeaderValue::from_str(&format!("Bearer {token}"))
        .map_err(|error| AppError::GithubApiError(error.to_string()))?;
    auth_value.set_sensitive(true);
    headers.insert(header::AUTHORIZATION, auth_value);
    headers.insert(
        header::ACCEPT,
        header::HeaderValue::from_static("application/vnd.github+json"),
    );
    headers.insert(
        header::USER_AGENT,
        header::HeaderValue::from_static(USER_AGENT),
    );
    Ok(headers)
}

pub async fn fetch_account(client: &Client, token: &str) -> Result<GithubAccount, AppError> {
    let response = client
        .get("https://api.github.com/user")
        .headers(auth_headers(token)?)
        .send()
        .await
        .map_err(|error| AppError::GithubApiError(error.to_string()))?;

    if !response.status().is_success() {
        return Err(AppError::GithubApiError(format!(
            "GitHub rejected this token (status {})",
            response.status()
        )));
    }

    let user: GithubUserResponse = response
        .json()
        .await
        .map_err(|error| AppError::GithubApiError(error.to_string()))?;

    Ok(GithubAccount {
        login: user.login,
        name: user.name,
        avatar_url: user.avatar_url,
        html_url: user.html_url,
    })
}

pub async fn fetch_repos(client: &Client, token: &str) -> Result<Vec<GithubRepoSummary>, AppError> {
    let response = client
        .get("https://api.github.com/user/repos?sort=updated&per_page=50")
        .headers(auth_headers(token)?)
        .send()
        .await
        .map_err(|error| AppError::GithubApiError(error.to_string()))?;

    if !response.status().is_success() {
        return Err(AppError::GithubApiError(format!(
            "GitHub request failed (status {})",
            response.status()
        )));
    }

    let repos: Vec<GithubRepoResponse> = response
        .json()
        .await
        .map_err(|error| AppError::GithubApiError(error.to_string()))?;

    Ok(repos
        .into_iter()
        .map(|repo| GithubRepoSummary {
            id: repo.id,
            name: repo.name,
            full_name: repo.full_name,
            private: repo.private,
            stargazers_count: repo.stargazers_count,
            html_url: repo.html_url,
            description: repo.description,
            updated_at: repo.updated_at,
        })
        .collect())
}

pub async fn fetch_issues(
    client: &Client,
    token: &str,
    repo_full_name: &str,
) -> Result<Vec<GithubIssueSummary>, AppError> {
    if !repo_full_name.contains('/') || repo_full_name.contains("..") {
        return Err(AppError::GithubApiError(
            "invalid repository name".to_string(),
        ));
    }
    let url =
        format!("https://api.github.com/repos/{repo_full_name}/issues?state=open&per_page=30");
    let response = client
        .get(url)
        .headers(auth_headers(token)?)
        .send()
        .await
        .map_err(|error| AppError::GithubApiError(error.to_string()))?;

    if !response.status().is_success() {
        return Err(AppError::GithubApiError(format!(
            "GitHub request failed (status {})",
            response.status()
        )));
    }

    let issues: Vec<GithubIssueResponse> = response
        .json()
        .await
        .map_err(|error| AppError::GithubApiError(error.to_string()))?;

    Ok(issues
        .into_iter()
        .map(|issue| GithubIssueSummary {
            id: issue.id,
            number: issue.number,
            title: issue.title,
            state: issue.state,
            html_url: issue.html_url,
            is_pull_request: issue.pull_request.is_some(),
        })
        .collect())
}
