//! Claude Code's logins: read `.credentials.json`, store refreshed tokens.

use std::fs;
use std::io::Write;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;

use anyhow::{Context, Result, bail};
use serde_json::Value;

const FILE: &str = ".credentials.json";

pub struct Oauth {
    pub access_token: String,
    pub refresh_token: Option<String>,
    /// Milliseconds since the epoch.
    pub expires_at: i64,
    pub rate_limit_tier: Option<String>,
}

/// The file as JSON, or None when there is none. Errors never quote the
/// contents: they hold tokens.
fn load(dir: &Path) -> Result<Option<Value>> {
    let path = dir.join(FILE);
    let text = match fs::read_to_string(&path) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e).with_context(|| format!("i couldn't read {}", path.display())),
    };
    let v = serde_json::from_str(&text)
        .with_context(|| format!("i couldn't parse {}", path.display()))?;
    Ok(Some(v))
}

/// The login in `dir`, or None when it isn't logged in (no file, no
/// `claudeAiOauth`, e.g. after a logout that left only `mcpOAuth`).
pub fn read(dir: &Path) -> Result<Option<Oauth>> {
    let Some(v) = load(dir)? else {
        return Ok(None);
    };
    let Some(o) = v.get("claudeAiOauth") else {
        return Ok(None);
    };
    let Some(access_token) = o.get("accessToken").and_then(Value::as_str) else {
        return Ok(None);
    };
    let text = |k: &str| o.get(k).and_then(Value::as_str).map(str::to_string);
    Ok(Some(Oauth {
        access_token: access_token.to_string(),
        refresh_token: text("refreshToken"),
        expires_at: o.get("expiresAt").and_then(Value::as_i64).unwrap_or(0),
        rate_limit_tier: text("rateLimitTier"),
    }))
}

/// Replace the tokens, keep every other field, and swap the file in
/// atomically. `refresh = None` keeps the old refresh token.
pub fn store_tokens(
    dir: &Path,
    access: &str,
    refresh: Option<&str>,
    expires_at: i64,
) -> Result<()> {
    let path = dir.join(FILE);
    let Some(mut v) = load(dir)? else {
        bail!("i couldn't update {}: no login there", path.display());
    };
    let Some(o) = v.get_mut("claudeAiOauth").and_then(Value::as_object_mut) else {
        bail!("i couldn't update {}: no login there", path.display());
    };
    o.insert("accessToken".into(), access.into());
    if let Some(r) = refresh {
        o.insert("refreshToken".into(), r.into());
    }
    o.insert("expiresAt".into(), expires_at.into());

    let mut tmp = tempfile::NamedTempFile::new_in(dir)
        .with_context(|| format!("i couldn't write in {}", dir.display()))?;
    tmp.write_all(serde_json::to_string(&v)?.as_bytes())
        .with_context(|| format!("i couldn't write {}", path.display()))?;
    tmp.as_file()
        .set_permissions(fs::Permissions::from_mode(0o600))
        .with_context(|| format!("i couldn't protect {}", path.display()))?;
    tmp.persist(&path)
        .map_err(|e| e.error)
        .with_context(|| format!("i couldn't replace {}", path.display()))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{Value, json};
    use std::fs;
    use std::os::unix::fs::PermissionsExt;

    fn write(dir: &Path, v: &Value) {
        fs::write(dir.join(".credentials.json"), v.to_string()).unwrap();
    }

    fn full() -> Value {
        json!({
            "claudeAiOauth": {
                "accessToken": "old-access",
                "refreshToken": "old-refresh",
                "expiresAt": 1000,
                "scopes": ["user:inference", "user:profile"],
                "subscriptionType": "team",
                "rateLimitTier": "default_claude_max_5x"
            },
            "mcpOAuth": { "runpod|abc": { "serverName": "runpod" } }
        })
    }

    fn stored(dir: &Path) -> Value {
        serde_json::from_str(&fs::read_to_string(dir.join(".credentials.json")).unwrap()).unwrap()
    }

    #[test]
    fn missing_file_is_none() {
        let d = tempfile::tempdir().unwrap();
        assert!(read(d.path()).unwrap().is_none());
    }

    #[test]
    fn only_mcp_oauth_is_none() {
        let d = tempfile::tempdir().unwrap();
        write(d.path(), &json!({"mcpOAuth": {"x": {}}}));
        assert!(read(d.path()).unwrap().is_none());
    }

    #[test]
    fn no_access_token_is_none() {
        let d = tempfile::tempdir().unwrap();
        write(d.path(), &json!({"claudeAiOauth": {"expiresAt": 1}}));
        assert!(read(d.path()).unwrap().is_none());
    }

    #[test]
    fn malformed_json_is_an_error_without_tokens() {
        let d = tempfile::tempdir().unwrap();
        fs::write(
            d.path().join(".credentials.json"),
            r#"{"accessToken":"sekret"#,
        )
        .unwrap();
        let e = format!("{:#}", read(d.path()).err().unwrap());
        assert!(e.contains("i couldn't parse"));
        assert!(!e.contains("sekret"));
    }

    #[test]
    fn reads_fields() {
        let d = tempfile::tempdir().unwrap();
        write(d.path(), &full());
        let o = read(d.path()).unwrap().unwrap();
        assert_eq!(o.access_token, "old-access");
        assert_eq!(o.refresh_token.as_deref(), Some("old-refresh"));
        assert_eq!(o.expires_at, 1000);
        assert_eq!(o.rate_limit_tier.as_deref(), Some("default_claude_max_5x"));
    }

    #[test]
    fn store_keeps_other_fields_and_mode() {
        let d = tempfile::tempdir().unwrap();
        write(d.path(), &full());
        store_tokens(d.path(), "new-access", Some("new-refresh"), 2000).unwrap();
        let v = stored(d.path());
        assert_eq!(v["mcpOAuth"], full()["mcpOAuth"]);
        let o = &v["claudeAiOauth"];
        assert_eq!(o["scopes"], full()["claudeAiOauth"]["scopes"]);
        assert_eq!(o["subscriptionType"], "team");
        assert_eq!(o["accessToken"], "new-access");
        assert_eq!(o["refreshToken"], "new-refresh");
        assert_eq!(o["expiresAt"], 2000);
        let mode = fs::metadata(d.path().join(".credentials.json"))
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o600);
        assert_eq!(
            fs::read_dir(d.path()).unwrap().count(),
            1,
            "no temp file left"
        );
    }

    #[test]
    fn store_without_refresh_keeps_old() {
        let d = tempfile::tempdir().unwrap();
        write(d.path(), &full());
        store_tokens(d.path(), "new-access", None, 2000).unwrap();
        assert_eq!(
            stored(d.path())["claudeAiOauth"]["refreshToken"],
            "old-refresh"
        );
    }

    #[test]
    fn store_errors_without_login() {
        let d = tempfile::tempdir().unwrap();
        write(d.path(), &json!({"mcpOAuth": {}}));
        assert!(store_tokens(d.path(), "a", None, 1).is_err());
        let d = tempfile::tempdir().unwrap();
        assert!(store_tokens(d.path(), "a", None, 1).is_err());
    }
}
