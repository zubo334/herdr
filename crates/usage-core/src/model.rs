use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use time::{format_description::well_known::Rfc3339, OffsetDateTime};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ProviderId {
    Codex,
    Claude,
    Zai,
}

impl ProviderId {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Codex => "codex",
            Self::Claude => "claude",
            Self::Zai => "zai",
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            Self::Codex => "Codex",
            Self::Claude => "Claude Code",
            Self::Zai => "Z.AI",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorKind {
    MissingCredentials,
    Authentication,
    Permission,
    RateLimited,
    Network,
    Api,
    Parse,
    Configuration,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UsageError {
    pub kind: ErrorKind,
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub retry_at: Option<i64>,
}

impl UsageError {
    pub fn new(kind: ErrorKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
            retry_at: None,
        }
    }
    pub fn parse(message: impl Into<String>) -> Self {
        Self::new(ErrorKind::Parse, message)
    }
}

impl std::fmt::Display for UsageError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}
impl std::error::Error for UsageError {}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Window {
    pub id: String,
    pub label: String,
    pub used_percent: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub window_seconds: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resets_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub used: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub unit: Option<String>,
}

impl Window {
    pub fn new(id: impl Into<String>, label: impl Into<String>, percent: f64) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
            used_percent: percent,
            window_seconds: None,
            resets_at: None,
            used: None,
            limit: None,
            unit: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Credits {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub balance: Option<f64>,
    #[serde(default)]
    pub unlimited: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub currency: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Allowance {
    pub label: String,
    pub used: Option<f64>,
    pub limit: Option<f64>,
    pub currency: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resets_at: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProviderUsage {
    pub provider: ProviderId,
    pub display_name: String,
    pub source: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub account: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plan: Option<String>,
    #[serde(default)]
    pub windows: Vec<Window>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub credits: Option<Credits>,
    #[serde(default)]
    pub allowances: Vec<Allowance>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reset_credits: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub updated_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_attempt_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<UsageError>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub retry_at: Option<i64>,
    /// One-way account/credential discriminator; never a token.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub owner_key: Option<String>,
    #[serde(default)]
    pub partial: bool,
    // Compatibility with the fork's original account-usage.json consumers.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_percent: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub weekly_percent: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub weekly_fable_percent: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_resets: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub weekly_resets: Option<String>,
}

impl ProviderUsage {
    pub fn new(id: ProviderId) -> Self {
        Self {
            provider: id,
            display_name: id.label().into(),
            source: String::new(),
            account: None,
            plan: None,
            windows: Vec::new(),
            credits: None,
            allowances: Vec::new(),
            reset_credits: None,
            updated_at: None,
            last_attempt_at: None,
            error: None,
            retry_at: None,
            owner_key: None,
            partial: false,
            session_percent: None,
            weekly_percent: None,
            weekly_fable_percent: None,
            session_resets: None,
            weekly_resets: None,
        }
    }

    pub fn update_legacy_fields(&mut self) {
        let session = self.windows.iter().find(|window| window.id == "session");
        let weekly = self.windows.iter().find(|window| window.id == "weekly");
        self.session_percent =
            session.map(|window| window.used_percent.clamp(0.0, 100.0).round() as u32);
        self.weekly_percent =
            weekly.map(|window| window.used_percent.clamp(0.0, 100.0).round() as u32);
        self.session_resets = session.and_then(|window| window.resets_at.clone());
        self.weekly_resets = weekly.and_then(|window| window.resets_at.clone());
    }

    pub fn has_data(&self) -> bool {
        !self.windows.is_empty() || self.credits.is_some() || !self.allowances.is_empty()
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct UsageSnapshot {
    pub version: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub updated_at: Option<String>,
    #[serde(default)]
    pub platforms: BTreeMap<String, ProviderUsage>,
}

impl Default for UsageSnapshot {
    fn default() -> Self {
        Self {
            version: 2,
            updated_at: None,
            platforms: BTreeMap::new(),
        }
    }
}

impl UsageSnapshot {
    pub fn validate(&self) -> bool {
        self.version == 2
            && self.platforms.len() <= 16
            && self.platforms.values().all(|usage| {
                usage.windows.len() <= 32
                    && usage
                        .windows
                        .iter()
                        .all(|window| window.used_percent.is_finite())
                    && usage.allowances.len() <= 16
            })
    }
}

pub(crate) fn timestamp(now: OffsetDateTime) -> String {
    now.to_offset(time::UtcOffset::UTC)
        .format(&Rfc3339)
        .unwrap_or_default()
}

pub(crate) fn unix_timestamp(value: i64) -> Option<String> {
    OffsetDateTime::from_unix_timestamp(value)
        .ok()
        .map(timestamp)
}

pub(crate) fn number(value: &serde_json::Value) -> Option<f64> {
    value
        .as_f64()
        .or_else(|| value.as_str().and_then(|text| text.parse().ok()))
        .filter(|number: &f64| number.is_finite())
}
