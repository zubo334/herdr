use crate::ProviderId;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

fn enabled() -> bool {
    true
}
fn interval() -> u64 {
    60
}
fn timeout() -> u64 {
    10
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    pub enabled: bool,
    pub interval_seconds: u64,
    pub timeout_seconds: u64,
    pub codex: ProviderSettings,
    pub claude: ProviderSettings,
    pub zai: ProviderSettings,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            enabled: enabled(),
            interval_seconds: interval(),
            timeout_seconds: timeout(),
            codex: ProviderSettings::default(),
            claude: ProviderSettings::default(),
            zai: ProviderSettings::default(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ProviderSettings {
    pub enabled: bool,
    pub credential_file: Option<PathBuf>,
    /// Explicit endpoint overrides must use HTTPS; tokens never follow cross-origin redirects.
    pub endpoint: Option<String>,
    /// Z.AI: global or bigmodel-cn.
    pub region: Option<String>,
    pub team: bool,
}

impl Default for ProviderSettings {
    fn default() -> Self {
        Self {
            enabled: true,
            credential_file: None,
            endpoint: None,
            region: None,
            team: false,
        }
    }
}

impl Config {
    pub fn settings(&self, provider: ProviderId) -> &ProviderSettings {
        match provider {
            ProviderId::Codex => &self.codex,
            ProviderId::Claude => &self.claude,
            ProviderId::Zai => &self.zai,
        }
    }
    pub fn interval_seconds(&self) -> u64 {
        self.interval_seconds.clamp(30, 86_400)
    }
    pub fn timeout_seconds(&self) -> u64 {
        self.timeout_seconds.clamp(1, 60)
    }
}
