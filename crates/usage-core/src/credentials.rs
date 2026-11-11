use crate::{Config, ErrorKind, ProviderId, UsageError};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};
use time::{format_description::well_known::Rfc3339, OffsetDateTime};

/// An injected environment keeps tests and other applications' accounts isolated.
pub type SecureStoreReader = fn(&str) -> Option<Vec<u8>>;

pub struct Environment {
    pub home: PathBuf,
    pub values: BTreeMap<String, String>,
    pub secure_store: Option<SecureStoreReader>,
}

impl Environment {
    pub fn system(home: PathBuf) -> Self {
        const KEYS: &[&str] = &[
            "CODEX_HOME",
            "CLAUDE_CONFIG_DIR",
            "CLAUDE_SECURESTORAGE_CONFIG_DIR",
            "HERDR_CODEX_OAUTH_TOKEN",
            "HERDR_CLAUDE_OAUTH_TOKEN",
            "CODEXBAR_CLAUDE_OAUTH_TOKEN",
            "CODEXBAR_CLAUDE_OAUTH_SCOPES",
            "Z_AI_API_KEY",
            "Z_AI_API_HOST",
            "Z_AI_QUOTA_URL",
            "Z_AI_BIGMODEL_ORGANIZATION",
            "Z_AI_BIGMODEL_PROJECT",
            "BIGMODEL_API_KEY",
            "ZHIPU_API_KEY",
            "ZHIPUAI_API_KEY",
            "GLM_API_KEY",
        ];
        Self {
            home,
            values: KEYS
                .iter()
                .filter_map(|key| std::env::var(key).ok().map(|value| ((*key).into(), value)))
                .collect(),
            secure_store: None,
        }
    }
    pub fn get(&self, key: &str) -> Option<&str> {
        self.values
            .get(key)
            .map(String::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
    }
    pub fn codex_home(&self) -> PathBuf {
        self.get("CODEX_HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| self.home.join(".codex"))
    }
    pub fn claude_home(&self) -> PathBuf {
        // An explicit empty secure-storage override means the default directory.
        if let Some(value) = self.values.get("CLAUDE_SECURESTORAGE_CONFIG_DIR") {
            return if value.is_empty() {
                self.home.join(".claude")
            } else {
                PathBuf::from(value)
            };
        }
        self.values
            .get("CLAUDE_CONFIG_DIR")
            .filter(|value| !value.is_empty())
            .map(PathBuf::from)
            .unwrap_or_else(|| self.home.join(".claude"))
    }
    pub(crate) fn zai_cn(&self, config: &Config) -> bool {
        match config.zai.region.as_deref() {
            Some("bigmodel-cn") => true,
            Some(_) => false,
            None => config
                .zai
                .endpoint
                .as_deref()
                .or_else(|| self.get("Z_AI_QUOTA_URL"))
                .or_else(|| self.get("Z_AI_API_HOST"))
                .and_then(|raw| {
                    reqwest::Url::parse(&if raw.contains("://") {
                        raw.to_owned()
                    } else {
                        format!("https://{raw}")
                    })
                    .ok()
                })
                .is_some_and(|url| url.host_str() == Some("open.bigmodel.cn")),
        }
    }
}

pub struct Credential {
    pub(crate) token: String,
    pub(crate) account_id: Option<String>,
    pub(crate) account: Option<String>,
    pub(crate) plan: Option<String>,
    pub owner_key: String,
    pub(crate) expires_at: Option<i64>,
    pub(crate) require_expiry: bool,
    pub(crate) expiry_margin_seconds: i64,
}

impl std::fmt::Debug for Credential {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Credential")
            .field("token", &"[redacted]")
            .finish_non_exhaustive()
    }
}

impl Credential {
    pub fn check_expiry(&self, now: OffsetDateTime) -> Result<(), UsageError> {
        if self.expires_at.is_some_and(|expiry| {
            expiry
                <= now
                    .unix_timestamp()
                    .saturating_add(self.expiry_margin_seconds)
        }) || (self.require_expiry && self.expires_at.is_none())
        {
            return Err(UsageError::new(ErrorKind::Authentication,
                "CLI-owned login needs refreshing; run the provider CLI. Herdr never rotates its tokens."));
        }
        Ok(())
    }
    pub(crate) fn bearer(&self) -> String {
        format!("Bearer {}", self.token)
    }
}

fn fingerprint(
    provider: ProviderId,
    source: &str,
    secret: &str,
    account_id: Option<&str>,
) -> String {
    let mut hash = Sha256::new();
    for part in [provider.as_str(), source, secret, account_id.unwrap_or("")] {
        hash.update(part.as_bytes());
        hash.update([0]);
    }
    format!("{:x}", hash.finalize())
}

fn token_override(provider: ProviderId, token: &str, source: &str) -> Credential {
    Credential {
        token: token.into(),
        account_id: None,
        account: None,
        plan: None,
        owner_key: fingerprint(provider, source, token, None),
        expires_at: None,
        require_expiry: false,
        expiry_margin_seconds: 0,
    }
}

fn read_json(path: &Path) -> Result<Value, UsageError> {
    let metadata = std::fs::metadata(path).map_err(|error| {
        let kind = if error.kind() == std::io::ErrorKind::NotFound {
            ErrorKind::MissingCredentials
        } else {
            ErrorKind::Permission
        };
        UsageError::new(kind, "Could not read the provider's credential file")
    })?;
    if metadata.len() > 1024 * 1024 {
        return Err(UsageError::parse("Credential file exceeds 1 MiB"));
    }
    let bytes = std::fs::read(path)
        .map_err(|_| UsageError::new(ErrorKind::Permission, "Could not read credential file"))?;
    serde_json::from_slice(&bytes)
        .map_err(|_| UsageError::parse("Malformed provider credential file"))
}

fn string_at(value: &Value, names: &[&str]) -> Option<String> {
    names
        .iter()
        .find_map(|key| {
            value
                .get(*key)
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty())
        })
        .map(str::to_owned)
}

pub(crate) fn jwt_claims(token: &str) -> Option<Value> {
    let payload = token.split('.').nth(1)?;
    if payload.len() > 64 * 1024 {
        return None;
    }
    let bytes = URL_SAFE_NO_PAD.decode(payload.trim_end_matches('=')).ok()?;
    serde_json::from_slice(&bytes).ok()
}

pub(crate) fn codex(config: &Config, env: &Environment) -> Result<Credential, UsageError> {
    if let Some(token) = env.get("HERDR_CODEX_OAUTH_TOKEN") {
        return Ok(token_override(
            ProviderId::Codex,
            token,
            config.codex.endpoint.as_deref().unwrap_or("environment"),
        ));
    }
    let path = config
        .codex
        .credential_file
        .clone()
        .unwrap_or_else(|| env.codex_home().join("auth.json"));
    let json = read_json(&path)?;
    let tokens = json.get("tokens").ok_or_else(|| {
        UsageError::new(
            ErrorKind::Authentication,
            "Codex subscription usage requires a ChatGPT login, not OPENAI_API_KEY",
        )
    })?;
    let access = string_at(tokens, &["access_token", "accessToken"]).ok_or_else(|| {
        UsageError::new(
            ErrorKind::Authentication,
            "Codex OAuth access token is missing",
        )
    })?;
    let claims = string_at(tokens, &["id_token", "idToken"]).and_then(|token| jwt_claims(&token));
    let access_claims = jwt_claims(&access);
    let identities = [claims.as_ref(), access_claims.as_ref()];
    let auth_claims = identities
        .iter()
        .flatten()
        .find_map(|claims| claims.get("https://api.openai.com/auth"));
    let account_id = string_at(tokens, &["account_id", "accountId"]).or_else(|| {
        identities.iter().flatten().find_map(|claims| {
            string_at(claims, &["chatgpt_account_id"])
                .or_else(|| {
                    claims
                        .get("https://api.openai.com/auth")
                        .and_then(|auth| string_at(auth, &["chatgpt_account_id"]))
                })
                .or_else(|| {
                    claims
                        .get("organizations")?
                        .as_array()?
                        .first()
                        .and_then(|organization| string_at(organization, &["id"]))
                })
        })
    });
    let expiry = access_claims
        .as_ref()
        .and_then(|claims| claims.get("exp").and_then(Value::as_i64))
        .or_else(|| {
            string_at(&json, &["last_refresh", "lastRefresh"])
                .and_then(|value| OffsetDateTime::parse(&value, &Rfc3339).ok())
                .map(|time| time.unix_timestamp() + 8 * 24 * 60 * 60)
        });
    let stable_secret =
        string_at(tokens, &["refresh_token", "refreshToken"]).unwrap_or_else(|| access.clone());
    Ok(Credential {
        owner_key: fingerprint(
            ProviderId::Codex,
            &format!(
                "{}:{}",
                path.display(),
                config.codex.endpoint.as_deref().unwrap_or("native")
            ),
            &stable_secret,
            account_id.as_deref(),
        ),
        token: access,
        account_id,
        account: identities.iter().flatten().find_map(|claims| {
            string_at(claims, &["email"]).or_else(|| {
                claims
                    .get("https://api.openai.com/profile")
                    .and_then(|profile| string_at(profile, &["email"]))
            })
        }),
        plan: auth_claims.and_then(|claims| string_at(claims, &["chatgpt_plan_type"])),
        expires_at: expiry,
        require_expiry: true,
        expiry_margin_seconds: 300,
    })
}

pub(crate) fn claude(config: &Config, env: &Environment) -> Result<Credential, UsageError> {
    if let Some(token) = env
        .get("HERDR_CLAUDE_OAUTH_TOKEN")
        .or_else(|| env.get("CODEXBAR_CLAUDE_OAUTH_TOKEN"))
    {
        if env
            .get("CODEXBAR_CLAUDE_OAUTH_SCOPES")
            .is_some_and(|scopes| {
                !scopes
                    .split(',')
                    .any(|scope| scope.trim() == "user:profile")
            })
        {
            return Err(UsageError::new(
                ErrorKind::Permission,
                "Claude token override needs the user:profile scope",
            ));
        }
        return Ok(token_override(
            ProviderId::Claude,
            token,
            config.claude.endpoint.as_deref().unwrap_or("environment"),
        ));
    }
    let root = env.claude_home();
    let path = config
        .claude
        .credential_file
        .clone()
        .unwrap_or_else(|| root.join(".credentials.json"));
    let json = match read_json(&path) {
        Ok(json) => json,
        Err(error)
            if error.kind == ErrorKind::MissingCredentials
                && config.claude.credential_file.is_none()
                && root == env.home.join(".claude") =>
        {
            let bytes = env
                .secure_store
                .and_then(|read| read("Claude Code-credentials"))
                .ok_or(error)?;
            serde_json::from_slice(&bytes)
                .map_err(|_| UsageError::parse("Malformed Claude secure-storage credentials"))?
        }
        Err(error) => return Err(error),
    };
    let oauth = json.get("claudeAiOauth").ok_or_else(|| {
        UsageError::new(
            ErrorKind::Authentication,
            "Claude account OAuth credentials are missing (MCP credentials cannot read quotas)",
        )
    })?;
    let token = string_at(oauth, &["accessToken"]).ok_or_else(|| {
        UsageError::new(
            ErrorKind::Authentication,
            "Claude OAuth access token is missing",
        )
    })?;
    if !oauth
        .get("scopes")
        .and_then(Value::as_array)
        .is_some_and(|scopes| {
            scopes
                .iter()
                .any(|scope| scope.as_str() == Some("user:profile"))
        })
    {
        return Err(UsageError::new(
            ErrorKind::Permission,
            "Claude login needs the user:profile scope",
        ));
    }
    let stable_secret = string_at(oauth, &["refreshToken"]).unwrap_or_else(|| token.clone());
    Ok(Credential {
        owner_key: fingerprint(
            ProviderId::Claude,
            &format!(
                "{}:{}",
                path.display(),
                config.claude.endpoint.as_deref().unwrap_or("native")
            ),
            &stable_secret,
            None,
        ),
        token,
        account_id: None,
        account: None,
        plan: string_at(oauth, &["subscriptionType", "rateLimitTier"]),
        expires_at: oauth
            .get("expiresAt")
            .and_then(Value::as_f64)
            .filter(|value| value.is_finite())
            .map(|value| (value / 1000.0) as i64),
        require_expiry: true,
        expiry_margin_seconds: 0,
    })
}

pub(crate) fn zai(config: &Config, env: &Environment) -> Result<Credential, UsageError> {
    let cn = env.zai_cn(config);
    let mut token = env.get("Z_AI_API_KEY").map(str::to_owned);
    if cn && token.is_none() {
        token = [
            "BIGMODEL_API_KEY",
            "ZHIPU_API_KEY",
            "ZHIPUAI_API_KEY",
            "GLM_API_KEY",
        ]
        .iter()
        .find_map(|key| env.get(key).map(str::to_owned));
        if token.is_none() {
            token = [
                ".coding-relay/glm-api-key",
                ".config/bigmodel/api_key",
                ".config/zhipu/api_key",
            ]
            .iter()
            .find_map(|relative| {
                std::fs::read_to_string(env.home.join(relative))
                    .ok()
                    .and_then(|text| {
                        text.lines()
                            .find(|line| !line.trim().is_empty())
                            .map(|line| line.trim().to_owned())
                    })
            });
        }
    }
    let token = token.ok_or_else(|| {
        UsageError::new(
            ErrorKind::MissingCredentials,
            "Set Z_AI_API_KEY to read Z.AI account quotas",
        )
    })?;
    let mut credential = token_override(ProviderId::Zai, &token, "environment");
    let scope = format!(
        "{}:{}:{}:{}:{}",
        if cn { "cn" } else { "global" },
        config.zai.team,
        env.get("Z_AI_BIGMODEL_ORGANIZATION").unwrap_or(""),
        env.get("Z_AI_BIGMODEL_PROJECT").unwrap_or(""),
        config
            .zai
            .endpoint
            .as_deref()
            .or_else(|| env.get("Z_AI_QUOTA_URL"))
            .or_else(|| env.get("Z_AI_API_HOST"))
            .unwrap_or("native")
    );
    credential.owner_key = fingerprint(ProviderId::Zai, &scope, &token, None);
    Ok(credential)
}
