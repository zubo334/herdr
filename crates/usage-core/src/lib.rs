//! Account quotas, independent of Herdr terminals, panes, and presentation.
//!
//! Provider mappings are adapted from CodexBar; see ../NOTICE.md.

mod config;
mod credentials;
mod http;
mod model;
pub mod providers;

pub use config::{Config, ProviderSettings};
pub use credentials::{Credential, Environment, SecureStoreReader};
pub use http::{HttpResponse, HttpTransport, ReqwestTransport};
pub use model::{
    Allowance, Credits, ErrorKind, ProviderId, ProviderUsage, UsageError, UsageSnapshot, Window,
};

use time::OffsetDateTime;

/// A provider owns credential discovery and conversion to the common account model.
/// No provider is allowed to modify another application's credentials.
pub trait UsageProvider: Sync {
    fn id(&self) -> ProviderId;
    fn credentials(&self, config: &Config, env: &Environment) -> Result<Credential, UsageError>;
    fn fetch(
        &self,
        config: &Config,
        env: &Environment,
        credential: &Credential,
        transport: &dyn HttpTransport,
        now: OffsetDateTime,
    ) -> Result<ProviderUsage, UsageError>;
}

/// Retain each provider independently; one account switch must not erase other
/// providers' last-good samples, and must never relabel another account's data.
pub fn owned_snapshot(
    config: &Config,
    env: &Environment,
    previous: &UsageSnapshot,
) -> UsageSnapshot {
    let mut snapshot = previous.clone();
    snapshot.platforms.retain(|name, usage| {
        if name != usage.provider.as_str() || !config.settings(usage.provider).enabled {
            return false;
        }
        let provider = providers::registry()
            .into_iter()
            .find(|provider| provider.id() == usage.provider);
        match provider.and_then(|provider| provider.credentials(config, env).ok()) {
            Some(credential) => usage.owner_key.as_deref() == Some(credential.owner_key.as_str()),
            None => usage.owner_key.is_none() && !usage.has_data(),
        }
    });
    snapshot
}

pub fn collect(
    config: &Config,
    env: &Environment,
    previous: &UsageSnapshot,
    transport: &dyn HttpTransport,
    now: OffsetDateTime,
) -> UsageSnapshot {
    let mut snapshot = UsageSnapshot::default();
    for provider in providers::registry() {
        let id = provider.id();
        if !config.settings(id).enabled {
            continue;
        }
        let old = previous.platforms.get(id.as_str());
        let credentials = provider.credentials(config, env);
        let owner = credentials
            .as_ref()
            .ok()
            .map(|value| value.owner_key.clone());
        if old
            .filter(|old| owner.is_some() && old.owner_key == owner)
            .and_then(|usage| usage.retry_at)
            .is_some_and(|retry| retry > now.unix_timestamp())
        {
            if let Some(old) = old {
                snapshot.platforms.insert(id.as_str().into(), old.clone());
            }
            continue;
        }
        let result = credentials.and_then(|credential| {
            credential.check_expiry(now)?;
            provider.fetch(config, env, &credential, transport, now)
        });
        let mut usage = match result {
            Ok(mut usage) => {
                usage.owner_key = owner;
                usage.updated_at = Some(model::timestamp(now));
                usage
            }
            Err(error) => {
                // A retained sample must belong to this credential, never another login.
                let mut usage = old
                    .filter(|old| owner.is_some() && old.owner_key == owner)
                    .cloned()
                    .unwrap_or_else(|| ProviderUsage::new(id));
                usage.owner_key = owner;
                usage.retry_at = error.retry_at;
                usage.error = Some(error);
                usage
            }
        };
        usage.last_attempt_at = Some(model::timestamp(now));
        usage.update_legacy_fields();
        snapshot.platforms.insert(id.as_str().into(), usage);
    }
    snapshot.updated_at = Some(model::timestamp(now));
    snapshot
}

#[cfg(test)]
mod tests;
