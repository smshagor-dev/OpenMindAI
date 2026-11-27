//! Bounded scholarly metadata discovery from Crossref and OpenAlex.
//! Metadata resolution confirms registry records only, never scientific validity.
use futures_util::{stream, StreamExt};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashSet;

use crate::app_error::AppError;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ScholarlyRecord {
    pub title: String,
    pub doi: Option<String>,
    pub authors: Vec<String>,
    pub publication_year: Option<i64>,
    pub container_title: Option<String>,
    pub landing_url: Option<String>,
    pub pdf_url: Option<String>,
    pub license_url: Option<String>,
    /// Abstract text returned by OpenAlex (never a substitute for full text).
    pub abstract_text: Option<String>,
    pub provider: String,
    pub metadata_verified: bool,
    pub verification_status: String,
}

pub(crate) async fn discover(query: &str) -> (Vec<ScholarlyRecord>, Vec<String>) {
    let client = crate::net::http_client();
    let (crossref, openalex) = tokio::join!(crossref(&client, query), openalex(&client, query));
    let mut records = Vec::new();
    let mut errors = Vec::new();
    match crossref {
        Ok(mut found) => records.append(&mut found),
        Err(error) => errors.push(format!("Crossref: {error}")),
    }
    match openalex {
        Ok(mut found) => records.append(&mut found),
        Err(error) => errors.push(format!("OpenAlex: {error}")),
    }
    // Crossref ranks first in the combined result set. Preserve an OpenAlex
    // abstract when its registry record deduplicates against the same DOI/title.
    let abstracts: std::collections::HashMap<String, String> = records
        .iter()
        .filter_map(|record| {
            record.abstract_text.as_ref().map(|abstract_text| {
                let key = record
                    .doi
                    .as_deref()
                    .map(normalize_doi)
                    .filter(|doi| !doi.is_empty())
                    .unwrap_or_else(|| normalize(&record.title));
                (key, abstract_text.clone())
            })
        })
        .collect();
    for record in &mut records {
        if record.abstract_text.is_none() {
            let key = record
                .doi
                .as_deref()
                .map(normalize_doi)
                .filter(|doi| !doi.is_empty())
                .unwrap_or_else(|| normalize(&record.title));
            record.abstract_text = abstracts.get(&key).cloned();
        }
    }
    let mut dois = HashSet::new();
    let mut titles = HashSet::new();
    records.retain(|record| {
        let title = normalize(&record.title);
        if title.is_empty() {
            return false;
        }
        if let Some(doi) = record.doi.as_deref() {
            let doi = normalize_doi(doi);
            if !doi.is_empty() && !dois.insert(doi) {
                return false;
            }
        }
        titles.insert(title)
    });
    let verified = stream::iter(records.into_iter().take(8).map(|mut record| {
        let client = client.clone();
        async move {
            if let Some(doi) = record.doi.as_deref() {
                record.verification_status = verify_doi(&client, doi, &record.title).await;
            }
            record
        }
    }))
    .buffer_unordered(4)
    .collect::<Vec<_>>()
    .await;
    (verified, errors)
}

async fn verify_doi(client: &reqwest::Client, doi: &str, expected_title: &str) -> String {
    let Ok(mut url) = reqwest::Url::parse("https://api.crossref.org/works/") else {
        return "unverified".into();
    };
    if let Ok(mut segments) = url.path_segments_mut() {
        segments.push(doi);
    } else {
        return "unverified".into();
    }
    let response = match client
        .get(url)
        .timeout(std::time::Duration::from_secs(7))
        .send()
        .await
    {
        Ok(response) if response.status().is_success() => response,
        _ => return "unverified".into(),
    };
    let Ok(body) = response.json::<Value>().await else {
        return "unverified".into();
    };
    let Some(item) = body.get("message") else {
        return "unverified".into();
    };
    if item
        .get("update-to")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .any(|update| {
            update
                .get("type")
                .and_then(Value::as_str)
                .is_some_and(|kind| kind.to_ascii_lowercase().contains("retraction"))
        })
    {
        return "retracted".into();
    }
    let returned_doi = item.get("DOI").and_then(Value::as_str).map(normalize_doi);
    let returned_title = first_text(item.get("title"));
    let expected_doi = normalize_doi(doi);
    if returned_doi.as_deref() != Some(expected_doi.as_str()) {
        return "mismatch".into();
    }
    if returned_title
        .as_deref()
        .is_some_and(|title| normalize(title) != normalize(expected_title))
    {
        return "mismatch".into();
    }
    "verified".into()
}

async fn crossref(client: &reqwest::Client, query: &str) -> Result<Vec<ScholarlyRecord>, AppError> {
    let mut url = reqwest::Url::parse("https://api.crossref.org/works")
        .map_err(|error| AppError::internal(error.to_string()))?;
    url.query_pairs_mut()
        .append_pair("query.bibliographic", query)
        .append_pair("rows", "8");
    let body: Value = client
        .get(url)
        .timeout(std::time::Duration::from_secs(12))
        .send()
        .await
        .map_err(|error| AppError::internal(format!("Crossref request failed: {error}")))?
        .error_for_status()
        .map_err(|error| AppError::internal(format!("Crossref returned an error: {error}")))?
        .json()
        .await
        .map_err(|error| AppError::internal(format!("Crossref response was invalid: {error}")))?;
    Ok(parse_crossref(&body))
}

async fn openalex(client: &reqwest::Client, query: &str) -> Result<Vec<ScholarlyRecord>, AppError> {
    let mut url = reqwest::Url::parse("https://api.openalex.org/works")
        .map_err(|error| AppError::internal(error.to_string()))?;
    url.query_pairs_mut()
        .append_pair("search", query)
        .append_pair("per-page", "8");
    let body: Value = client
        .get(url)
        .timeout(std::time::Duration::from_secs(12))
        .send()
        .await
        .map_err(|error| AppError::internal(format!("OpenAlex request failed: {error}")))?
        .error_for_status()
        .map_err(|error| AppError::internal(format!("OpenAlex returned an error: {error}")))?
        .json()
        .await
        .map_err(|error| AppError::internal(format!("OpenAlex response was invalid: {error}")))?;
    Ok(parse_openalex(&body))
}

fn parse_crossref(body: &Value) -> Vec<ScholarlyRecord> {
    body.pointer("/message/items")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|item| {
            let title = first_text(item.get("title"))?;
            let authors = item
                .get("author")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(|author| {
                    let name = [
                        author.get("given").and_then(Value::as_str),
                        author.get("family").and_then(Value::as_str),
                    ]
                    .into_iter()
                    .flatten()
                    .collect::<Vec<_>>()
                    .join(" ");
                    (!name.trim().is_empty()).then_some(name)
                })
                .collect();
            let year = item
                .pointer("/published-print/date-parts/0/0")
                .and_then(Value::as_i64)
                .or_else(|| {
                    item.pointer("/published-online/date-parts/0/0")
                        .and_then(Value::as_i64)
                })
                .or_else(|| {
                    item.pointer("/created/date-parts/0/0")
                        .and_then(Value::as_i64)
                });
            let doi = item
                .get("DOI")
                .and_then(Value::as_str)
                .map(normalize_doi)
                .filter(|v| !v.is_empty());
            let landing_url = item.get("URL").and_then(Value::as_str).map(str::to_owned);
            let pdf_url = item
                .get("link")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .find(|link| {
                    link.get("content-type")
                        .and_then(Value::as_str)
                        .is_some_and(|v| v.contains("pdf"))
                })
                .and_then(|link| link.get("URL"))
                .and_then(Value::as_str)
                .map(str::to_owned);
            let license_url = item
                .get("license")
                .and_then(Value::as_array)
                .and_then(|list| list.first())
                .and_then(|license| license.get("URL"))
                .and_then(Value::as_str)
                .map(str::to_owned);
            Some(ScholarlyRecord {
                title,
                doi,
                authors,
                publication_year: year,
                container_title: first_text(item.get("container-title")),
                landing_url,
                pdf_url,
                license_url,
                abstract_text: item
                    .get("abstract")
                    .and_then(Value::as_str)
                    .map(str::to_owned),
                provider: "Crossref".into(),
                metadata_verified: true,
                verification_status: "unverified".into(),
            })
        })
        .collect()
}

fn parse_openalex(body: &Value) -> Vec<ScholarlyRecord> {
    body.get("results")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|item| {
            let title = item.get("title").and_then(Value::as_str)?.to_owned();
            let authors = item
                .get("authorships")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(|author| {
                    author
                        .pointer("/author/display_name")
                        .and_then(Value::as_str)
                        .map(str::to_owned)
                })
                .collect();
            let location = item.get("primary_location");
            let landing_url = location
                .and_then(|v| v.get("landing_page_url"))
                .and_then(Value::as_str)
                .map(str::to_owned)
                .or_else(|| item.get("id").and_then(Value::as_str).map(str::to_owned));
            let container_title = location
                .and_then(|v| v.pointer("/source/display_name"))
                .and_then(Value::as_str)
                .map(str::to_owned);
            let pdf_url = item
                .pointer("/best_oa_location/pdf_url")
                .and_then(Value::as_str)
                .map(str::to_owned);
            let doi = item
                .get("doi")
                .and_then(Value::as_str)
                .map(normalize_doi)
                .filter(|v| !v.is_empty());
            let abstract_text = item
                .get("abstract_inverted_index")
                .and_then(reconstruct_abstract);
            Some(ScholarlyRecord {
                title,
                doi,
                authors,
                publication_year: item.get("publication_year").and_then(Value::as_i64),
                container_title,
                landing_url,
                pdf_url,
                license_url: None,
                abstract_text,
                provider: "OpenAlex".into(),
                metadata_verified: true,
                verification_status: "unverified".into(),
            })
        })
        .collect()
}

fn reconstruct_abstract(index: &Value) -> Option<String> {
    let entries = index.as_object()?;
    let mut words = Vec::<Option<&str>>::new();
    for (word, positions) in entries {
        for position in positions
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(Value::as_u64)
        {
            let position = usize::try_from(position).ok()?;
            if position > 20_000 {
                continue;
            }
            if words.len() <= position {
                words.resize(position + 1, None);
            }
            words[position] = Some(word.as_str());
        }
    }
    let text = words.into_iter().flatten().collect::<Vec<_>>().join(" ");
    let text = text.trim();
    (!text.is_empty()).then(|| text.chars().take(8_000).collect())
}

fn first_text(value: Option<&Value>) -> Option<String> {
    value
        .and_then(Value::as_array)
        .and_then(|items| items.first())
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|text| !text.is_empty())
        .map(str::to_owned)
}
fn normalize(value: &str) -> String {
    value
        .chars()
        .filter(|c| c.is_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}
fn normalize_doi(value: &str) -> String {
    value
        .trim()
        .trim_start_matches("https://doi.org/")
        .trim_start_matches("http://doi.org/")
        .trim_start_matches("doi:")
        .trim()
        .to_ascii_lowercase()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_crossref_metadata_without_claiming_scientific_validation() {
        let body = serde_json::json!({"message":{"items":[{
            "title":["A careful study"], "DOI":"10.1234/ABC", "author":[{"given":"Ada","family":"Lovelace"}],
            "published-print":{"date-parts":[[2024,1,1]]}, "container-title":["Journal"], "URL":"https://doi.org/10.1234/abc"
        }]}});
        let items = parse_crossref(&body);
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].doi.as_deref(), Some("10.1234/abc"));
        assert_eq!(items[0].publication_year, Some(2024));
        assert!(items[0].metadata_verified);
    }

    #[test]
    fn deduplicates_registry_hits_by_canonical_doi() {
        assert_eq!(normalize_doi("https://doi.org/10.1234/ABC"), "10.1234/abc");
        assert_eq!(normalize("A Careful Study!"), normalize("a-careful study"));
    }

    #[test]
    fn reconstructs_openalex_abstract_in_word_order() {
        let index = serde_json::json!({"Canopy":[0],"cooling":[2],"trees":[1]});
        assert_eq!(
            reconstruct_abstract(&index).as_deref(),
            Some("Canopy trees cooling")
        );
    }
}
