use chrono::Utc;
use reqwest::{header, Client};
use rusqlite::{params, OptionalExtension};
use serde::{Deserialize, Serialize};

use crate::{app_error::AppError, database::Database};

const SETTINGS_KEY: &str = "app.github";
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
struct StoredGithub {
    token: String,
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
        Ok(self.get_stored()?.map(|stored| stored.account))
    }

    pub fn get_token(&self) -> Result<Option<String>, AppError> {
        Ok(self.get_stored()?.map(|stored| stored.token))
    }

    fn get_stored(&self) -> Result<Option<StoredGithub>, AppError> {
        let value: Option<String> = self
            .database
            .connection()
            .query_row(
                "SELECT value_json FROM app_settings WHERE key = ?1",
                params![SETTINGS_KEY],
                |row| row.get(0),
            )
            .optional()?;
        Ok(value.and_then(|value| serde_json::from_str(&value).ok()))
    }

    pub fn save(&self, token: &str, account: &GithubAccount) -> Result<(), AppError> {
        let stored = StoredGithub {
            token: token.to_string(),
            account: account.clone(),
        };
        let payload = serde_json::to_string(&stored)
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

    pub fn clear(&self) -> Result<(), AppError> {
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
