//! Proxy userinfo belongs in the OS credential store, never preference JSON.
use crate::app_error::AppError;
use serde_json::Value;
use url::Url;
use uuid::Uuid;

fn error() -> AppError {
    AppError::SecretStoreFailed("Proxy credentials are unavailable; unlock the OS credential store or replace/remove the saved credentials.".into())
}

fn endpoint(value: &str) -> Result<(String, bool), AppError> {
    if value.trim().is_empty() {
        return Ok((String::new(), false));
    }
    let input = if value.contains("://") {
        value.trim().to_owned()
    } else {
        format!("http://{}", value.trim())
    };
    let mut url = Url::parse(&input).map_err(|_| AppError::internal("Invalid proxy address"))?;
    if !matches!(url.scheme(), "http" | "https")
        || url.host_str().is_none()
        || url.path() != "/"
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err(AppError::internal(
            "Proxy requires an HTTP(S) endpoint without query or fragment",
        ));
    }
    let authenticated = !url.username().is_empty() || url.password().is_some();
    url.set_username("").map_err(|_| error())?;
    url.set_password(None).map_err(|_| error())?;
    Ok((url.to_string(), authenticated))
}

fn slot(id: &str) -> Result<String, AppError> {
    let id = Uuid::parse_str(id).map_err(|_| error())?;
    Ok(format!("network.proxy.{}", id.hyphenated()))
}

/// Write a fresh immutable credential slot before committing sanitized preferences.
/// A failed DB commit can leave an unused keyring item, but cannot replace a
/// previously referenced credential or persist plaintext as a fallback.
pub fn protect(
    preferences: &mut Value,
    previous: Option<&Value>,
    mut put: impl FnMut(&str, &str) -> Result<(), AppError>,
) -> Result<(), AppError> {
    let input = preferences
        .get("networkProxyUrl")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let (clean, authenticated) = endpoint(input)?;
    let id = if authenticated {
        let id = Uuid::new_v4().to_string();
        let full = if input.contains("://") {
            input.trim().to_owned()
        } else {
            format!("http://{}", input.trim())
        };
        put(&slot(&id)?, &full).map_err(|_| error())?;
        Some(id)
    } else if let Some(id) = preferences
        .get("networkProxyCredentialId")
        .and_then(Value::as_str)
    {
        slot(id)?;
        let valid = previous.is_some_and(|old| {
            old.get("networkProxyCredentialId").and_then(Value::as_str) == Some(id)
                && old.get("networkProxyUrl").and_then(Value::as_str) == Some(clean.as_str())
        });
        if !valid {
            return Err(error());
        }
        Some(id.to_owned())
    } else {
        None
    };
    preferences["networkProxyUrl"] = Value::String(clean);
    preferences["networkProxyCredentialId"] = id.map(Value::String).unwrap_or(Value::Null);
    Ok(())
}

/// Rewrite only proxy fields; preserve future/unknown preferences and reject a
/// concurrent proxy edit instead of overwriting it with a migration snapshot.
pub fn migrate(
    connection: &rusqlite::Connection,
    old: &Value,
    clean: &Value,
) -> Result<(), AppError> {
    connection.execute_batch("PRAGMA secure_delete = ON;")?;
    let changed = connection.execute(
        "UPDATE app_settings SET value_json = json_set(value_json, '$.networkProxyUrl', ?1, '$.networkProxyCredentialId', ?2), updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now') WHERE key = 'app.preferences' AND json_extract(value_json, '$.networkProxyUrl') = ?3 AND json_extract(value_json, '$.networkProxyCredentialId') IS ?4",
        rusqlite::params![clean["networkProxyUrl"].as_str(), clean["networkProxyCredentialId"].as_str(), old["networkProxyUrl"].as_str(), old["networkProxyCredentialId"].as_str()],
    )?;
    if changed != 1 {
        return Err(AppError::internal(
            "Proxy preferences changed during migration; retry",
        ));
    }
    // Existing backups and externally held snapshots cannot be securely erased here.
    let _ = connection.execute_batch("PRAGMA wal_checkpoint(TRUNCATE);");
    Ok(())
}

pub fn restore(
    clean: &str,
    id: Option<&str>,
    mut get: impl FnMut(&str) -> Result<Option<String>, AppError>,
) -> Result<String, AppError> {
    let Some(id) = id else {
        return Ok(clean.to_owned());
    };
    let full = get(&slot(id)?).map_err(|_| error())?.ok_or_else(error)?;
    let (endpoint, authenticated) = endpoint(&full).map_err(|_| error())?;
    if !authenticated || endpoint != clean {
        return Err(error());
    }
    Ok(full)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::collections::HashMap;

    #[test]
    fn authenticated_url_roundtrip_keeps_userinfo_out_of_json() {
        let mut secrets = HashMap::new();
        let mut preferences =
            json!({"networkProxyUrl":"http://user:p%40ss@proxy.example:3128", "unrelated":42});
        protect(&mut preferences, None, |slot, full| {
            secrets.insert(slot.to_owned(), full.to_owned());
            Ok(())
        })
        .unwrap();
        assert_eq!(preferences["networkProxyUrl"], "http://proxy.example:3128/");
        assert_eq!(preferences["unrelated"], 42);
        assert!(!preferences.to_string().contains("p%40ss"));
        assert!(!preferences.to_string().contains("user:"));
        let full = restore(
            preferences["networkProxyUrl"].as_str().unwrap(),
            preferences["networkProxyCredentialId"].as_str(),
            |slot| Ok(secrets.get(slot).cloned()),
        )
        .unwrap();
        assert_eq!(full, "http://user:p%40ss@proxy.example:3128");
        let previous = preferences.clone();
        protect(&mut preferences, Some(&previous), |_, _| {
            panic!("unrelated save must not rewrite keyring")
        })
        .unwrap();
        assert_eq!(preferences, previous);
    }

    #[test]
    fn failed_keyring_never_mutates_preferences_or_discloses_input() {
        let mut preferences =
            json!({"networkProxyUrl":"http://dummy:synthetic-secret@proxy.example"});
        let previous = preferences.clone();
        let error = protect(&mut preferences, None, |_, _| {
            Err(AppError::internal("synthetic-secret"))
        })
        .unwrap_err();
        assert_eq!(preferences, previous);
        assert!(!error.to_string().contains("synthetic-secret"));
    }

    #[test]
    fn replacement_uses_new_slot_and_leaves_previous_reference_restorable() {
        let mut secrets = HashMap::new();
        let mut first = json!({"networkProxyUrl":"http://u:old-password@proxy.example"});
        protect(&mut first, None, |slot, full| {
            secrets.insert(slot.to_owned(), full.to_owned());
            Ok(())
        })
        .unwrap();
        let mut second = first.clone();
        second["networkProxyUrl"] = json!("http://u:new-password@proxy.example");
        protect(&mut second, Some(&first), |slot, full| {
            secrets.insert(slot.to_owned(), full.to_owned());
            Ok(())
        })
        .unwrap();
        assert_ne!(
            first["networkProxyCredentialId"],
            second["networkProxyCredentialId"]
        );
        assert!(restore(
            first["networkProxyUrl"].as_str().unwrap(),
            first["networkProxyCredentialId"].as_str(),
            |slot| Ok(secrets.get(slot).cloned())
        )
        .unwrap()
        .contains("old-password"));
    }

    #[test]
    fn forged_reference_endpoint_change_and_missing_keyring_are_rejected() {
        let id = Uuid::new_v4().to_string();
        let previous =
            json!({"networkProxyUrl":"http://proxy.example/", "networkProxyCredentialId":id});
        for mut value in [previous.clone(), previous.clone()] {
            value["networkProxyUrl"] = json!("http://other.example/");
            assert!(protect(&mut value, Some(&previous), |_, _| Ok(())).is_err());
        }
        let mut forged = previous.clone();
        assert!(protect(&mut forged, None, |_, _| Ok(())).is_err());
        assert!(restore("http://proxy.example/", Some(&id), |_| Ok(None)).is_err());
        assert!(
            restore("http://proxy.example/", Some("github.token"), |_| panic!(
                "invalid slot must never be read"
            ))
            .is_err()
        );
        assert!(restore("http://other.example/", Some(&id), |_| Ok(Some(
            "http://u:p@proxy.example/".into()
        )))
        .is_err());
    }

    #[test]
    fn bare_endpoints_and_explicit_unlink_need_no_keyring() {
        let mut value = json!({"networkProxyUrl":"proxy.example:3128"});
        protect(&mut value, None, |_, _| panic!("no credential")).unwrap();
        assert_eq!(value["networkProxyUrl"], "http://proxy.example:3128/");
        assert!(value["networkProxyCredentialId"].is_null());
        assert_eq!(restore("", None, |_| panic!("no credential")).unwrap(), "");
    }

    #[test]
    fn malformed_input_errors_do_not_echo_credentials() {
        for input in [
            "http://u:synthetic-secret@[invalid",
            "ftp://u:synthetic-secret@proxy",
            "http://proxy?password=synthetic-secret",
        ] {
            let error =
                protect(&mut json!({"networkProxyUrl":input}), None, |_, _| Ok(())).unwrap_err();
            assert!(!error.to_string().contains("synthetic-secret"));
        }
    }

    #[test]
    fn migration_preserves_future_fields_scrubs_snapshot_and_rejects_stale_edits() {
        let original = json!({"networkProxyUrl":"http://fixture:synthetic-password@proxy.example", "futureField":42});
        let mut value = original.clone();
        protect(&mut value, None, |_, _| Ok(())).unwrap();
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("preferences.db");
        let database = rusqlite::Connection::open(&path).unwrap();
        database.execute_batch("PRAGMA journal_mode=WAL; CREATE TABLE app_settings(key TEXT PRIMARY KEY, value_json TEXT, updated_at TEXT);").unwrap();
        database
            .execute(
                "INSERT INTO app_settings VALUES ('app.preferences', ?1, 'old')",
                [original.to_string()],
            )
            .unwrap();
        migrate(&database, &original, &value).unwrap();
        assert!(
            migrate(&database, &original, &value).is_err(),
            "stale migration must not overwrite a newer proxy"
        );
        let snapshot = temp.path().join("snapshot.db");
        database
            .execute("VACUUM INTO ?1", [snapshot.to_str().unwrap()])
            .unwrap();
        for file in [&snapshot, &path] {
            assert!(!std::fs::read(file)
                .unwrap()
                .windows(b"synthetic-password".len())
                .any(|bytes| bytes == b"synthetic-password"));
        }
        let copied = rusqlite::Connection::open(snapshot).unwrap();
        let json: String = copied
            .query_row("SELECT value_json FROM app_settings", [], |row| row.get(0))
            .unwrap();
        let value: Value = serde_json::from_str(&json).unwrap();
        assert_eq!(value["futureField"], 42);
        assert!(restore(
            value["networkProxyUrl"].as_str().unwrap(),
            value["networkProxyCredentialId"].as_str(),
            |_| Ok(None)
        )
        .is_err());
    }
}
