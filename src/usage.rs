//! Shared account quota collection and the compatibility JSON cache.
//! No work in this module is driven by pane output or rendering.
use std::{
    fs::{File, OpenOptions},
    io::{self, Read, Write},
    path::{Path, PathBuf},
};

pub(crate) mod presentation;

pub(crate) fn config_path() -> PathBuf {
    crate::config::config_dir().join("usage.toml")
}
pub(crate) fn cache_path() -> PathBuf {
    crate::plugin_paths::plugin_state_dir("usage-monitor").join("account-usage.json")
}

pub(crate) fn read_config(path: &Path) -> Result<herdr_usage::Config, herdr_usage::UsageError> {
    match std::fs::read_to_string(path) {
        Ok(text) => toml::from_str(&text).map_err(|_| {
            herdr_usage::UsageError::new(
                herdr_usage::ErrorKind::Configuration,
                "Invalid usage.toml; check provider usage configuration",
            )
        }),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(herdr_usage::Config::default()),
        Err(_) => Err(herdr_usage::UsageError::new(
            herdr_usage::ErrorKind::Configuration,
            "Could not read usage.toml",
        )),
    }
}

pub(crate) fn read_cache(path: &Path) -> io::Result<herdr_usage::UsageSnapshot> {
    let mut bytes = Vec::new();
    File::open(path)?
        .take(1024 * 1024 + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() > 1024 * 1024 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "Usage cache exceeds 1 MiB",
        ));
    }
    let snapshot: herdr_usage::UsageSnapshot =
        serde_json::from_slice(&bytes).map_err(io::Error::other)?;
    if !snapshot.validate() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "Invalid usage cache",
        ));
    }
    Ok(snapshot)
}

pub(crate) fn write_cache(path: &Path, snapshot: &herdr_usage::UsageSnapshot) -> io::Result<()> {
    let parent = path
        .parent()
        .filter(|path| !path.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    std::fs::create_dir_all(parent)?;
    static SEQUENCE: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let temporary = parent.join(format!(
        ".account-usage-{}-{}.tmp",
        std::process::id(),
        SEQUENCE.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    ));
    let mut file = crate::platform::create_config_temporary(&temporary, true)?;
    let result = (|| {
        serde_json::to_writer_pretty(&mut file, snapshot).map_err(io::Error::other)?;
        file.write_all(b"\n")?;
        file.sync_all()?;
        drop(file);
        crate::platform::replace_file(&temporary, path)
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&temporary);
    }
    result
}

pub(crate) struct Refresh {
    pub(crate) generation: u64,
    pub(crate) snapshot: std::sync::Arc<herdr_usage::UsageSnapshot>,
    pub(crate) interval_seconds: u64,
}

impl std::fmt::Debug for Refresh {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("UsageRefresh")
            .field("generation", &self.generation)
            .finish_non_exhaustive()
    }
}

pub(crate) fn environment() -> io::Result<herdr_usage::Environment> {
    let home = std::env::var_os("HOME")
        .filter(|value| !value.is_empty())
        .or_else(|| std::env::var_os("USERPROFILE").filter(|value| !value.is_empty()))
        .map(PathBuf::from)
        .ok_or_else(|| io::Error::other("Could not resolve user home"))?;
    let mut env = herdr_usage::Environment::system(home);
    env.secure_store = Some(crate::platform::read_usage_secure_store);
    Ok(env)
}

pub(crate) fn fetch(
    config: &herdr_usage::Config,
    previous: &herdr_usage::UsageSnapshot,
) -> Result<herdr_usage::UsageSnapshot, herdr_usage::UsageError> {
    let env = environment().map_err(|_| {
        herdr_usage::UsageError::new(
            herdr_usage::ErrorKind::Configuration,
            "Could not resolve the usage credential home",
        )
    })?;
    let transport = herdr_usage::ReqwestTransport::new(config.timeout_seconds())?;
    Ok(herdr_usage::collect(
        config,
        &env,
        previous,
        &transport,
        time::OffsetDateTime::now_utc(),
    ))
}

/// A short-lived OS lock shared by named servers and explicit CLI writes.
pub(crate) fn cache_lock(path: &Path) -> io::Result<File> {
    let parent = path
        .parent()
        .filter(|path| !path.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    std::fs::create_dir_all(parent)?;
    let lock = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(path.with_extension("lock"))?;
    lock.try_lock().map_err(|_| {
        io::Error::new(
            io::ErrorKind::WouldBlock,
            "Another quota collector is active",
        )
    })?;
    Ok(lock)
}

fn disabled_snapshot(path: &Path) -> herdr_usage::UsageSnapshot {
    let snapshot = herdr_usage::UsageSnapshot::default();
    if let Ok(_lock) = cache_lock(path) {
        if let Err(error) = write_cache(path, &snapshot) {
            tracing::warn!(%error, "could not clear disabled usage cache");
        }
    }
    snapshot
}

pub(crate) fn collect_refresh(
    generation: u64,
    previous: std::sync::Arc<herdr_usage::UsageSnapshot>,
) -> Refresh {
    let mut interval_seconds = 60;
    let result: Result<herdr_usage::UsageSnapshot, herdr_usage::UsageError> = (|| {
        let config = read_config(&config_path())?;
        interval_seconds = config.interval_seconds();
        let path = cache_path();
        if !config.enabled {
            return Ok(disabled_snapshot(&path));
        }
        let env = environment().map_err(|_| {
            herdr_usage::UsageError::new(
                herdr_usage::ErrorKind::Configuration,
                "Could not resolve credential home",
            )
        })?;
        let previous_cache = read_cache(&path).unwrap_or_else(|_| (*previous).clone());
        let owned = herdr_usage::owned_snapshot(&config, &env, &previous_cache);
        let now = time::OffsetDateTime::now_utc();
        let enabled_count = herdr_usage::providers::registry()
            .iter()
            .filter(|provider| config.settings(provider.id()).enabled)
            .count();
        let fresh = owned.platforms.len() == enabled_count
            && owned.platforms.values().all(|usage| {
                usage
                    .retry_at
                    .is_some_and(|retry| retry > now.unix_timestamp())
                    || usage
                        .last_attempt_at
                        .as_deref()
                        .and_then(|value| {
                            time::OffsetDateTime::parse(
                                value,
                                &time::format_description::well_known::Rfc3339,
                            )
                            .ok()
                        })
                        .is_some_and(|updated| {
                            let age = (now - updated).whole_seconds();
                            age >= 0 && age < interval_seconds as i64
                        })
            });
        if fresh {
            return Ok(owned);
        }
        let _lock = match cache_lock(&path) {
            Ok(lock) => lock,
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                interval_seconds = 5;
                return Ok(owned);
            }
            Err(_) => {
                return Err(herdr_usage::UsageError::new(
                    herdr_usage::ErrorKind::Configuration,
                    "Could not lock the usage cache",
                ))
            }
        };
        let transport = herdr_usage::ReqwestTransport::new(config.timeout_seconds())?;
        let snapshot = herdr_usage::collect(&config, &env, &owned, &transport, now);
        if let Err(error) = write_cache(&path, &snapshot) {
            tracing::warn!(%error, "could not persist usage cache");
        }
        Ok(snapshot)
    })();
    let snapshot = match result {
        Ok(snapshot) => snapshot,
        Err(error) => {
            let mut snapshot = (*previous).clone();
            for provider in herdr_usage::providers::registry() {
                let usage = snapshot
                    .platforms
                    .entry(provider.id().as_str().into())
                    .or_insert_with(|| herdr_usage::ProviderUsage::new(provider.id()));
                usage.error = Some(error.clone());
            }
            snapshot
        }
    };
    Refresh {
        generation,
        snapshot: std::sync::Arc::new(snapshot),
        interval_seconds,
    }
}

pub(crate) fn legacy_projection(
    snapshot: &herdr_usage::UsageSnapshot,
) -> Vec<crate::protocol::PluginUsageEntry> {
    snapshot
        .platforms
        .iter()
        .map(|(name, usage)| crate::protocol::PluginUsageEntry {
            name: name.clone(),
            session_percent: usage.session_percent,
            weekly_percent: usage.weekly_percent,
            weekly_fable_percent: usage.weekly_fable_percent,
            reset_credits: usage.reset_credits,
            session_resets: usage.session_resets.clone(),
            weekly_resets: usage.weekly_resets.clone(),
            updated_at: usage.updated_at.clone(),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    struct Temp(PathBuf);
    impl Temp {
        fn new() -> Self {
            let stamp = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let path = std::env::temp_dir()
                .join(format!("herdr-usage-cache-{}-{stamp}", std::process::id()));
            std::fs::create_dir(&path).unwrap();
            Self(path)
        }
    }
    impl Drop for Temp {
        fn drop(&mut self) {
            assert!(self.0.is_absolute() && self.0.starts_with(std::env::temp_dir()));
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    #[test]
    fn disabling_collection_replaces_old_cache_and_releases_the_writer_lock() {
        let root = Temp::new();
        let path = root.0.join("account-usage.json");
        let mut old = herdr_usage::UsageSnapshot::default();
        old.platforms.insert(
            "codex".into(),
            herdr_usage::ProviderUsage::new(herdr_usage::ProviderId::Codex),
        );
        write_cache(&path, &old).unwrap();
        assert!(!read_cache(&path).unwrap().platforms.is_empty());
        assert!(disabled_snapshot(&path).platforms.is_empty());
        assert!(read_cache(&path).unwrap().platforms.is_empty());
        let first = cache_lock(&path).unwrap();
        assert_eq!(
            cache_lock(&path).unwrap_err().kind(),
            io::ErrorKind::WouldBlock
        );
        drop(first);
        assert!(cache_lock(&path).is_ok());
        assert!(!std::fs::read_dir(&root.0).unwrap().any(|entry| entry
            .unwrap()
            .path()
            .extension()
            .is_some_and(|ext| ext == "tmp")));
    }
    #[test]
    fn corrupt_cache_is_rejected_instead_of_becoming_zero_usage() {
        let root = Temp::new();
        let path = root.0.join("account-usage.json");
        std::fs::write(&path, "truncated").unwrap();
        assert!(read_cache(&path).is_err());
        std::fs::write(&path, r#"{"version":999,"platforms":{}}"#).unwrap();
        assert!(read_cache(&path).is_err());
    }
}
