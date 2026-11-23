//! Gathering each account's usage: cache, token refresh, one thread each.

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use jiff::{SignedDuration, Timestamp};
use serde::{Deserialize, Serialize};

use crate::api::{self, ApiError, Tokens, Usage};
use crate::config::Account;
use crate::credentials;

/// The network side, so tests can script it.
pub trait Net: Sync {
    fn usage(&self, token: &str) -> Result<Usage, ApiError>;
    fn refresh(&self, refresh_token: &str) -> Result<Tokens, ApiError>;
}

/// The real thing.
pub struct Http;

impl Net for Http {
    fn usage(&self, token: &str) -> Result<Usage, ApiError> {
        api::fetch_usage(token)
    }
    fn refresh(&self, refresh_token: &str) -> Result<Tokens, ApiError> {
        api::refresh(refresh_token)
    }
}

#[derive(Debug, Clone)]
pub enum Status {
    Ready {
        usage: Usage,
        fetched_at: Timestamp,
        stale: bool,
    },
    NotLoggedIn,
    LoginExpired,
    Unavailable,
}

#[derive(Serialize, Deserialize)]
struct Entry {
    fetched_at: Timestamp,
    usage: Usage,
}

fn cache_path(account: &Account, cache_dir: &Path) -> PathBuf {
    cache_dir.join(format!("{}.json", account.name))
}

/// The cached usage and when it was fetched. A missing, unreadable or
/// corrupt file counts as no cache.
pub fn cached(account: &Account, cache_dir: &Path) -> Option<(Usage, Timestamp)> {
    let text = fs::read_to_string(cache_path(account, cache_dir)).ok()?;
    let e: Entry = serde_json::from_str(&text).ok()?;
    Some((e.usage, e.fetched_at))
}

/// Best effort: a cache that can't be written is not worth failing for.
fn write_cache(account: &Account, cache_dir: &Path, usage: &Usage, at: Timestamp) {
    let _ = try_write_cache(account, cache_dir, usage, at);
}

fn try_write_cache(
    account: &Account,
    cache_dir: &Path,
    usage: &Usage,
    at: Timestamp,
) -> anyhow::Result<()> {
    fs::create_dir_all(cache_dir)?;
    let body = serde_json::to_vec(&serde_json::json!({ "fetched_at": at, "usage": usage }))?;
    let mut tmp = tempfile::NamedTempFile::new_in(cache_dir)?;
    tmp.write_all(&body)?;
    tmp.persist(cache_path(account, cache_dir))?;
    Ok(())
}

pub fn status(
    account: &Account,
    cache_dir: &Path,
    max_age: SignedDuration,
    at: Timestamp,
    net: &dyn Net,
) -> Status {
    let cache = cached(account, cache_dir);
    if let Some((usage, fetched_at)) = &cache
        && at.duration_since(*fetched_at) <= max_age
    {
        return Status::Ready {
            usage: usage.clone(),
            fetched_at: *fetched_at,
            stale: false,
        };
    }
    // Offline or unreadable: the old value if there is one.
    let fallback = |cache: Option<(Usage, Timestamp)>| match cache {
        Some((usage, fetched_at)) => Status::Ready {
            usage,
            fetched_at,
            stale: true,
        },
        None => Status::Unavailable,
    };

    let oauth = match credentials::read(&account.dir) {
        Ok(Some(o)) => o,
        Ok(None) => return Status::NotLoggedIn,
        Err(_) => return fallback(cache),
    };
    let at_ms = at.as_millisecond();

    let token = if oauth.expires_at <= at_ms {
        match renew(account, cache_dir, at_ms, net) {
            Renewed::Token(t) => t,
            Renewed::Done(status) => return status,
            Renewed::Offline => return fallback(cache),
        }
    } else {
        oauth.access_token
    };

    match net.usage(&token) {
        Ok(usage) => {
            write_cache(account, cache_dir, &usage, at);
            Status::Ready {
                usage,
                fetched_at: at,
                stale: false,
            }
        }
        Err(ApiError::Unauthorized) => Status::LoginExpired,
        Err(ApiError::Other(_)) => fallback(cache),
    }
}

enum Renewed {
    Token(String),
    Done(Status),
    Offline,
}

/// Get a usable access token for an expired login. Refresh tokens rotate, so
/// the new one must never be lost: refreshes are serialised per account, the
/// folder is proven writable first, and a login that can't be stored is
/// copied to the cache folder.
fn renew(account: &Account, cache_dir: &Path, at_ms: i64, net: &dyn Net) -> Renewed {
    // Prove we can store the result before spending the refresh token.
    if tempfile::NamedTempFile::new_in(&account.dir).is_err() {
        return Renewed::Offline;
    }
    if fs::create_dir_all(cache_dir).is_err() {
        return Renewed::Offline;
    }
    let lock_path = cache_dir.join(format!("{}.lock", account.name));
    let Ok(lock) = fs::File::create(&lock_path) else {
        return Renewed::Offline;
    };
    if lock.lock().is_err() {
        return Renewed::Offline;
    }
    // The lock is released when `lock` drops, after the tokens are stored.

    // Someone else may have refreshed while we waited.
    let oauth = match credentials::read(&account.dir) {
        Ok(Some(o)) => o,
        Ok(None) => return Renewed::Done(Status::NotLoggedIn),
        Err(_) => return Renewed::Offline,
    };
    if oauth.expires_at > at_ms {
        return Renewed::Token(oauth.access_token);
    }
    let Some(rt) = oauth.refresh_token.as_deref() else {
        return Renewed::Done(Status::LoginExpired);
    };
    match net.refresh(rt) {
        Ok(t) => {
            let expires_at = at_ms.saturating_add(t.expires_in.max(0).saturating_mul(1000));
            let store = || {
                credentials::store_tokens(
                    &account.dir,
                    &t.access_token,
                    t.refresh_token.as_deref(),
                    expires_at,
                )
            };
            if store().is_err() && store().is_err() {
                rescue(account, cache_dir, &t, expires_at);
            }
            Renewed::Token(t.access_token)
        }
        Err(ApiError::Unauthorized) => {
            // Claude may have just refreshed it: look once more.
            match credentials::read(&account.dir) {
                Ok(Some(o)) if o.expires_at > at_ms => Renewed::Token(o.access_token),
                _ => Renewed::Done(Status::LoginExpired),
            }
        }
        Err(ApiError::Other(_)) => Renewed::Offline,
    }
}

/// Keep a login that couldn't be written back, so the user can recover it.
fn rescue(account: &Account, cache_dir: &Path, t: &Tokens, expires_at: i64) {
    let path = cache_dir.join(format!("{}.rescue.json", account.name));
    let saved = (|| -> anyhow::Result<()> {
        let body = serde_json::to_vec(&serde_json::json!({
            "access_token": t.access_token,
            "refresh_token": t.refresh_token,
            "expires_at": expires_at,
        }))?;
        // Temp files are created 0600.
        let mut tmp = tempfile::NamedTempFile::new_in(cache_dir)?;
        tmp.write_all(&body)?;
        tmp.persist(&path)?;
        Ok(())
    })();
    if saved.is_ok() {
        eprintln!(
            "tally: i couldn't save the new login for {}; a copy is in {}",
            account.name,
            path.display()
        );
    }
}

/// Every account at once, in the order given.
pub fn gather(
    accounts: &[Account],
    cache_dir: &Path,
    max_age: SignedDuration,
    net: &dyn Net,
) -> Vec<Status> {
    let at = Timestamp::now();
    std::thread::scope(|s| {
        let handles: Vec<_> = accounts
            .iter()
            .map(|a| s.spawn(move || status(a, cache_dir, max_age, at, net)))
            .collect();
        handles
            .into_iter()
            .map(|h| h.join().unwrap_or(Status::Unavailable))
            .collect()
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::Window;
    use anyhow::anyhow;
    use std::sync::Mutex;

    #[derive(Default)]
    struct Fake {
        usage_calls: Mutex<Vec<String>>,
        refresh_calls: Mutex<Vec<String>>,
        usage_result: Mutex<Option<Result<Usage, ApiError>>>,
        refresh_result: Mutex<Option<Result<Tokens, ApiError>>>,
    }

    impl Fake {
        fn with_usage(r: Result<Usage, ApiError>) -> Fake {
            let f = Fake::default();
            *f.usage_result.lock().unwrap() = Some(r);
            f
        }
        fn usage_count(&self) -> usize {
            self.usage_calls.lock().unwrap().len()
        }
    }

    impl Net for Fake {
        fn usage(&self, token: &str) -> Result<Usage, ApiError> {
            self.usage_calls.lock().unwrap().push(token.to_string());
            match &*self.usage_result.lock().unwrap() {
                Some(Ok(u)) => Ok(u.clone()),
                Some(Err(ApiError::Unauthorized)) => Err(ApiError::Unauthorized),
                _ => Err(ApiError::Other(anyhow!("offline"))),
            }
        }
        fn refresh(&self, rt: &str) -> Result<Tokens, ApiError> {
            self.refresh_calls.lock().unwrap().push(rt.to_string());
            match self.refresh_result.lock().unwrap().take() {
                Some(r) => r,
                None => Err(ApiError::Other(anyhow!("offline"))),
            }
        }
    }

    fn usage(pct: f64) -> Usage {
        Usage {
            five_hour: None,
            seven_day: Some(Window {
                utilization: pct,
                resets_at: None,
            }),
        }
    }

    fn week(u: &Usage) -> f64 {
        u.seven_day.as_ref().unwrap().utilization
    }

    fn ts(s: i64) -> Timestamp {
        Timestamp::from_second(s).unwrap()
    }

    const MIN: SignedDuration = SignedDuration::from_secs(60);

    fn account(root: &Path, name: &str) -> Account {
        let dir = root.join(name);
        fs::create_dir_all(&dir).unwrap();
        Account {
            name: name.into(),
            dir,
            plan: None,
            is_main: false,
        }
    }

    fn login(a: &Account, access: &str, refresh: Option<&str>, expires_ms: i64) {
        let mut o = serde_json::json!({"accessToken": access, "expiresAt": expires_ms});
        if let Some(r) = refresh {
            o["refreshToken"] = r.into();
        }
        fs::write(
            a.dir.join(".credentials.json"),
            serde_json::json!({ "claudeAiOauth": o }).to_string(),
        )
        .unwrap();
    }

    fn put_cache(a: &Account, cache: &Path, pct: f64, at: Timestamp) {
        write_cache(a, cache, &usage(pct), at);
    }

    #[test]
    fn fresh_cache_skips_network() {
        let t = tempfile::tempdir().unwrap();
        let cache = t.path().join("cache");
        let a = account(t.path(), "a");
        put_cache(&a, &cache, 10.0, ts(1000));
        let net = Fake::default();
        let s = status(&a, &cache, MIN, ts(1030), &net);
        assert!(matches!(s, Status::Ready { stale: false, .. }));
        assert_eq!(net.usage_count(), 0);
        // Clock skew: a cache from the future is fresh.
        let s = status(&a, &cache, MIN, ts(900), &net);
        assert!(matches!(s, Status::Ready { stale: false, .. }));
        assert_eq!(net.usage_count(), 0);
    }

    #[test]
    fn not_logged_in() {
        let t = tempfile::tempdir().unwrap();
        let a = account(t.path(), "a");
        let net = Fake::default();
        let s = status(&a, &t.path().join("c"), MIN, ts(1000), &net);
        assert!(matches!(s, Status::NotLoggedIn));
    }

    #[test]
    fn valid_token_fetches_and_caches() {
        let t = tempfile::tempdir().unwrap();
        let cache = t.path().join("cache");
        let a = account(t.path(), "a");
        login(&a, "tok", None, 2_000_000);
        let net = Fake::with_usage(Ok(usage(40.0)));
        let s = status(&a, &cache, MIN, ts(1000), &net);
        let Status::Ready {
            usage,
            fetched_at,
            stale,
        } = s
        else {
            panic!("not ready")
        };
        assert_eq!(week(&usage), 40.0);
        assert_eq!(fetched_at, ts(1000));
        assert!(!stale);
        assert_eq!(*net.usage_calls.lock().unwrap(), vec!["tok"]);
        let (u, at) = cached(&a, &cache).unwrap();
        assert_eq!((week(&u), at), (40.0, ts(1000)));
    }

    #[test]
    fn expired_token_refreshes_first() {
        let t = tempfile::tempdir().unwrap();
        let a = account(t.path(), "a");
        login(&a, "old", Some("r1"), 1_000_000);
        let net = Fake::with_usage(Ok(usage(5.0)));
        *net.refresh_result.lock().unwrap() = Some(Ok(Tokens {
            access_token: "new".into(),
            refresh_token: Some("r2".into()),
            expires_in: 100,
        }));
        let s = status(&a, &t.path().join("c"), MIN, ts(1000), &net);
        assert!(matches!(s, Status::Ready { .. }));
        assert_eq!(*net.refresh_calls.lock().unwrap(), vec!["r1"]);
        assert_eq!(*net.usage_calls.lock().unwrap(), vec!["new"]);
        let o = credentials::read(&a.dir).unwrap().unwrap();
        assert_eq!(o.access_token, "new");
        assert_eq!(o.refresh_token.as_deref(), Some("r2"));
        assert_eq!(o.expires_at, 1_000_000 + 100_000);
    }

    #[test]
    fn expired_without_refresh_token_is_login_expired() {
        let t = tempfile::tempdir().unwrap();
        let a = account(t.path(), "a");
        login(&a, "old", None, 1_000_000);
        let net = Fake::default();
        let s = status(&a, &t.path().join("c"), MIN, ts(1000), &net);
        assert!(matches!(s, Status::LoginExpired));
    }

    #[test]
    fn rejected_refresh_rereads_file() {
        let t = tempfile::tempdir().unwrap();
        let a = account(t.path(), "a");
        login(&a, "old", Some("r1"), 1_000_000);
        // Claude refreshes meanwhile: the file changes while we are rejected.
        struct Racing<'a>(&'a Account, Fake);
        impl Net for Racing<'_> {
            fn usage(&self, token: &str) -> Result<Usage, ApiError> {
                self.1.usage(token)
            }
            fn refresh(&self, rt: &str) -> Result<Tokens, ApiError> {
                login(self.0, "claude-made", Some("r9"), 5_000_000);
                self.1.refresh(rt)
            }
        }
        let net = Racing(&a, Fake::with_usage(Ok(usage(5.0))));
        *net.1.refresh_result.lock().unwrap() = Some(Err(ApiError::Unauthorized));
        let s = status(&a, &t.path().join("c"), MIN, ts(1000), &net);
        assert!(matches!(s, Status::Ready { .. }));
        assert_eq!(net.1.refresh_calls.lock().unwrap().len(), 1);
        assert_eq!(*net.1.usage_calls.lock().unwrap(), vec!["claude-made"]);
    }

    #[test]
    fn rejected_refresh_is_login_expired() {
        let t = tempfile::tempdir().unwrap();
        let a = account(t.path(), "a");
        login(&a, "old", Some("r1"), 1_000_000);
        let net = Fake::default();
        *net.refresh_result.lock().unwrap() = Some(Err(ApiError::Unauthorized));
        let s = status(&a, &t.path().join("c"), MIN, ts(1000), &net);
        assert!(matches!(s, Status::LoginExpired));
        assert_eq!(net.usage_count(), 0);
    }

    #[test]
    fn rejected_usage_is_login_expired() {
        let t = tempfile::tempdir().unwrap();
        let a = account(t.path(), "a");
        login(&a, "tok", None, 2_000_000);
        let net = Fake::with_usage(Err(ApiError::Unauthorized));
        let s = status(&a, &t.path().join("c"), MIN, ts(1000), &net);
        assert!(matches!(s, Status::LoginExpired));
        assert_eq!(net.usage_count(), 1);
    }

    #[test]
    fn offline_falls_back_to_stale_cache() {
        let t = tempfile::tempdir().unwrap();
        let cache = t.path().join("cache");
        let a = account(t.path(), "a");
        login(&a, "tok", None, 2_000_000);
        let net = Fake::default(); // every call errors
        assert!(matches!(
            status(&a, &cache, MIN, ts(1000), &net),
            Status::Unavailable
        ));
        put_cache(&a, &cache, 30.0, ts(100));
        let Status::Ready { usage, stale, .. } = status(&a, &cache, MIN, ts(1000), &net) else {
            panic!("not ready")
        };
        assert_eq!(week(&usage), 30.0);
        assert!(stale);
    }

    #[test]
    fn malformed_credentials_fall_back_to_cache() {
        let t = tempfile::tempdir().unwrap();
        let cache = t.path().join("cache");
        let a = account(t.path(), "a");
        fs::write(a.dir.join(".credentials.json"), "{nope").unwrap();
        let net = Fake::default();
        assert!(matches!(
            status(&a, &cache, MIN, ts(1000), &net),
            Status::Unavailable
        ));
        put_cache(&a, &cache, 30.0, ts(100));
        assert!(matches!(
            status(&a, &cache, MIN, ts(1000), &net),
            Status::Ready { stale: true, .. }
        ));
    }

    #[test]
    fn corrupt_cache_counts_as_absent() {
        let t = tempfile::tempdir().unwrap();
        let cache = t.path().join("cache");
        fs::create_dir_all(&cache).unwrap();
        let a = account(t.path(), "a");
        fs::write(cache.join("a.json"), "garbage").unwrap();
        assert!(cached(&a, &cache).is_none());
        login(&a, "tok", None, 2_000_000);
        let net = Fake::with_usage(Ok(usage(1.0)));
        assert!(matches!(
            status(&a, &cache, MIN, ts(1000), &net),
            Status::Ready { stale: false, .. }
        ));
    }

    #[test]
    fn gather_keeps_order() {
        let t = tempfile::tempdir().unwrap();
        let cache = t.path().join("cache");
        let accounts: Vec<_> = ["a", "b", "c", "d"]
            .iter()
            .map(|n| account(t.path(), n))
            .collect();
        // b is logged out, the others are logged in.
        for a in [&accounts[0], &accounts[2], &accounts[3]] {
            login(a, "tok", None, i64::MAX / 2);
        }
        let net = Fake::with_usage(Ok(usage(7.0)));
        let out = gather(&accounts, &cache, MIN, &net);
        assert_eq!(out.len(), 4);
        assert!(matches!(out[0], Status::Ready { .. }));
        assert!(matches!(out[1], Status::NotLoggedIn));
        assert!(matches!(out[2], Status::Ready { .. }));
        assert!(matches!(out[3], Status::Ready { .. }));
    }

    #[test]
    fn gather_offline_uses_stale_cache_and_never_fails() {
        let t = tempfile::tempdir().unwrap();
        let cache = t.path().join("cache");
        let a = account(t.path(), "a");
        let b = account(t.path(), "b");
        login(&a, "tok", None, i64::MAX / 2);
        login(&b, "tok", None, i64::MAX / 2);
        put_cache(&a, &cache, 20.0, ts(1)); // long ago
        let net = Fake::default();
        let out = gather(&[a, b], &cache, MIN, &net);
        assert!(matches!(out[0], Status::Ready { stale: true, .. }));
        assert!(matches!(out[1], Status::Unavailable));
    }

    #[test]
    fn gather_survives_a_panicking_thread() {
        struct Boom;
        impl Net for Boom {
            fn usage(&self, token: &str) -> Result<Usage, ApiError> {
                if token == "boom" {
                    panic!("boom");
                }
                Ok(usage(1.0))
            }
            fn refresh(&self, _: &str) -> Result<Tokens, ApiError> {
                unreachable!()
            }
        }
        let t = tempfile::tempdir().unwrap();
        let a = account(t.path(), "a");
        let b = account(t.path(), "b");
        login(&a, "boom", None, i64::MAX / 2);
        login(&b, "ok", None, i64::MAX / 2);
        let out = gather(&[a, b], &t.path().join("c"), MIN, &Boom);
        assert!(matches!(out[0], Status::Unavailable));
        assert!(matches!(out[1], Status::Ready { .. }));
    }

    fn expired(t: &Path, refresh: Result<Tokens, ApiError>) -> (Account, Fake) {
        let a = account(t, "a");
        login(&a, "old", Some("r1"), 1_000_000);
        let net = Fake::with_usage(Ok(usage(5.0)));
        *net.refresh_result.lock().unwrap() = Some(refresh);
        (a, net)
    }

    fn other() -> Result<Tokens, ApiError> {
        Err(ApiError::Other(anyhow!("offline")))
    }

    #[test]
    fn failed_refresh_falls_back_to_cache() {
        let t = tempfile::tempdir().unwrap();
        let cache = t.path().join("cache");
        let (a, net) = expired(t.path(), other());
        assert!(matches!(
            status(&a, &cache, MIN, ts(1000), &net),
            Status::Unavailable
        ));
        put_cache(&a, &cache, 30.0, ts(100));
        *net.refresh_result.lock().unwrap() = Some(other());
        assert!(matches!(
            status(&a, &cache, MIN, ts(1000), &net),
            Status::Ready { stale: true, .. }
        ));
        assert_eq!(net.usage_count(), 0);
    }

    #[test]
    fn token_expiring_exactly_now_is_refreshed() {
        let t = tempfile::tempdir().unwrap();
        let a = account(t.path(), "a");
        login(&a, "old", Some("r1"), 1_000_000);
        let net = Fake::with_usage(Ok(usage(5.0)));
        let _ = status(&a, &t.path().join("c"), MIN, ts(1000), &net);
        assert_eq!(net.refresh_calls.lock().unwrap().len(), 1);
    }

    #[test]
    fn cache_exactly_max_age_is_fresh() {
        let t = tempfile::tempdir().unwrap();
        let cache = t.path().join("cache");
        let a = account(t.path(), "a");
        put_cache(&a, &cache, 10.0, ts(1000));
        let net = Fake::default();
        let s = status(&a, &cache, MIN, ts(1060), &net);
        assert!(matches!(s, Status::Ready { stale: false, .. }));
        assert_eq!(net.usage_count(), 0);
    }

    #[test]
    fn reread_finding_another_expired_token_is_login_expired() {
        let t = tempfile::tempdir().unwrap();
        let a = account(t.path(), "a");
        login(&a, "old", Some("r1"), 1_000_000);
        struct Racing<'a>(&'a Account, Fake);
        impl Net for Racing<'_> {
            fn usage(&self, token: &str) -> Result<Usage, ApiError> {
                self.1.usage(token)
            }
            fn refresh(&self, rt: &str) -> Result<Tokens, ApiError> {
                login(self.0, "other-old", Some("r9"), 500_000);
                self.1.refresh(rt)
            }
        }
        let net = Racing(&a, Fake::default());
        *net.1.refresh_result.lock().unwrap() = Some(Err(ApiError::Unauthorized));
        let s = status(&a, &t.path().join("c"), MIN, ts(1000), &net);
        assert!(matches!(s, Status::LoginExpired));
        assert_eq!(net.1.refresh_calls.lock().unwrap().len(), 1);
        assert_eq!(net.1.usage_count(), 0);
    }

    #[test]
    fn huge_expires_in_does_not_overflow() {
        let t = tempfile::tempdir().unwrap();
        let (a, net) = expired(
            t.path(),
            Ok(Tokens {
                access_token: "new".into(),
                refresh_token: None,
                expires_in: i64::MAX,
            }),
        );
        let s = status(&a, &t.path().join("c"), MIN, ts(1000), &net);
        assert!(matches!(s, Status::Ready { .. }));
        let o = credentials::read(&a.dir).unwrap().unwrap();
        assert_eq!(o.expires_at, i64::MAX);
    }

    #[test]
    fn unwritable_account_dir_does_not_refresh() {
        use std::os::unix::fs::PermissionsExt;
        let t = tempfile::tempdir().unwrap();
        let cache = t.path().join("cache");
        let (a, net) = expired(t.path(), other());
        put_cache(&a, &cache, 30.0, ts(100));
        fs::set_permissions(&a.dir, fs::Permissions::from_mode(0o555)).unwrap();
        let s = status(&a, &cache, MIN, ts(1000), &net);
        fs::set_permissions(&a.dir, fs::Permissions::from_mode(0o755)).unwrap();
        assert_eq!(net.refresh_calls.lock().unwrap().len(), 0);
        assert!(matches!(s, Status::Ready { stale: true, .. }));
    }

    #[test]
    fn failed_store_leaves_a_rescue_copy() {
        use std::os::unix::fs::PermissionsExt;
        let t = tempfile::tempdir().unwrap();
        let cache = t.path().join("cache");
        let a = account(t.path(), "a");
        login(&a, "old", Some("r1"), 1_000_000);
        // The file goes bad while the refresh is in flight, so storing fails.
        struct Breaking<'a>(&'a Account, Fake);
        impl Net for Breaking<'_> {
            fn usage(&self, token: &str) -> Result<Usage, ApiError> {
                self.1.usage(token)
            }
            fn refresh(&self, rt: &str) -> Result<Tokens, ApiError> {
                fs::write(self.0.dir.join(".credentials.json"), "{nope").unwrap();
                self.1.refresh(rt)
            }
        }
        let net = Breaking(&a, Fake::with_usage(Ok(usage(5.0))));
        *net.1.refresh_result.lock().unwrap() = Some(Ok(Tokens {
            access_token: "new".into(),
            refresh_token: Some("r2".into()),
            expires_in: 100,
        }));
        let s = status(&a, &cache, MIN, ts(1000), &net);
        assert!(matches!(s, Status::Ready { stale: false, .. }));
        assert_eq!(*net.1.usage_calls.lock().unwrap(), vec!["new"]);
        let path = cache.join("a.rescue.json");
        let mode = fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
        let v: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(v["access_token"], "new");
        assert_eq!(v["refresh_token"], "r2");
        assert_eq!(v["expires_at"], 1_000_000 + 100_000);
    }

    #[test]
    fn credentials_refreshed_while_waiting_for_the_lock_are_used() {
        let t = tempfile::tempdir().unwrap();
        let cache = t.path().join("cache");
        fs::create_dir_all(&cache).unwrap();
        let a = account(t.path(), "a");
        login(&a, "old", Some("r1"), 1_000_000);
        let net = Fake::with_usage(Ok(usage(5.0)));
        let held = fs::File::create(cache.join("a.lock")).unwrap();
        held.lock().unwrap();
        let s = std::thread::scope(|sc| {
            let h = sc.spawn(|| status(&a, &cache, MIN, ts(1000), &net));
            std::thread::sleep(std::time::Duration::from_millis(200));
            // Another process refreshed meanwhile.
            login(&a, "theirs", Some("r2"), 5_000_000);
            drop(held);
            h.join().unwrap()
        });
        assert!(matches!(s, Status::Ready { .. }));
        assert_eq!(net.refresh_calls.lock().unwrap().len(), 0);
        assert_eq!(*net.usage_calls.lock().unwrap(), vec!["theirs"]);
    }
}
