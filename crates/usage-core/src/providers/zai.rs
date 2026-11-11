use super::text;
use crate::{
    credentials, http, model, Config, Credential, Environment, ErrorKind, HttpTransport,
    ProviderId, ProviderUsage, UsageError, UsageProvider, Window,
};
use serde_json::Value;
use time::OffsetDateTime;

pub struct Zai;
impl UsageProvider for Zai {
    fn id(&self) -> ProviderId {
        ProviderId::Zai
    }
    fn credentials(&self, config: &Config, env: &Environment) -> Result<Credential, UsageError> {
        credentials::zai(config, env)
    }
    fn fetch(
        &self,
        config: &Config,
        env: &Environment,
        credential: &Credential,
        transport: &dyn HttpTransport,
        now: OffsetDateTime,
    ) -> Result<ProviderUsage, UsageError> {
        if config
            .zai
            .region
            .as_deref()
            .is_some_and(|region| !matches!(region, "bigmodel-cn" | "global"))
        {
            return Err(UsageError::new(
                ErrorKind::Configuration,
                "Z.AI region must be global or bigmodel-cn",
            ));
        }
        let cn = env.zai_cn(config);
        let raw = config
            .zai
            .endpoint
            .as_deref()
            .or_else(|| env.get("Z_AI_QUOTA_URL"));
        let endpoint = if let Some(raw) = raw {
            normalize_url(raw)?
        } else if let Some(host) = env.get("Z_AI_API_HOST") {
            let mut url = reqwest::Url::parse(&normalize_url(host)?)
                .map_err(|_| UsageError::new(ErrorKind::Configuration, "Invalid Z.AI API host"))?;
            if url.path() == "/" || url.path().is_empty() {
                url.set_path("/api/monitor/usage/quota/limit");
            }
            url.to_string()
        } else {
            format!(
                "https://{}/api/monitor/usage/quota/limit",
                if cn { "open.bigmodel.cn" } else { "api.z.ai" }
            )
        };
        let mut url =
            reqwest::Url::parse(&endpoint).map_err(|_| UsageError::parse("Invalid Z.AI URL"))?;
        if matches!(
            (cn, url.host_str()),
            (true, Some("api.z.ai")) | (false, Some("open.bigmodel.cn"))
        ) {
            return Err(UsageError::new(
                ErrorKind::Configuration,
                "Z.AI endpoint does not match the selected region",
            ));
        }
        let bearer = credential.bearer();
        let mut headers = vec![
            ("Authorization", bearer.as_str()),
            ("Accept", "application/json"),
        ];
        if config.zai.team {
            let org = env.get("Z_AI_BIGMODEL_ORGANIZATION").ok_or_else(|| {
                UsageError::new(
                    ErrorKind::Configuration,
                    "Z.AI team usage requires an organization",
                )
            })?;
            let project = env.get("Z_AI_BIGMODEL_PROJECT").ok_or_else(|| {
                UsageError::new(
                    ErrorKind::Configuration,
                    "Z.AI team usage requires a project",
                )
            })?;
            headers.extend([
                ("Bigmodel-Organization", org),
                ("Bigmodel-Project", project),
            ]);
            let pairs: Vec<(String, String)> = url
                .query_pairs()
                .filter(|(key, _)| key != "type")
                .map(|(key, value)| (key.into_owned(), value.into_owned()))
                .collect();
            url.set_query(None);
            url.query_pairs_mut()
                .extend_pairs(pairs)
                .append_pair("type", "2");
        }
        let json = http::json_response(transport.get(url.as_str(), &headers)?, now)?;
        parse(&json, now)
    }
}

fn normalize_url(value: &str) -> Result<String, UsageError> {
    http::checked_url(&if value.contains("://") {
        value.to_owned()
    } else {
        format!("https://{value}")
    })
}

pub fn parse(json: &Value, now: OffsetDateTime) -> Result<ProviderUsage, UsageError> {
    if json.get("success").and_then(Value::as_bool) != Some(true)
        || json.get("code").and_then(Value::as_i64) != Some(200)
    {
        return Err(UsageError::new(
            ErrorKind::Api,
            "Z.AI rejected the quota request; check the API key and plan",
        ));
    }
    let data = json
        .get("data")
        .ok_or_else(|| UsageError::parse("Z.AI response has no data"))?;
    let limits = data
        .get("limits")
        .and_then(Value::as_array)
        .ok_or_else(|| UsageError::parse("Z.AI response has no quota list"))?;
    let mut usage = ProviderUsage::new(ProviderId::Zai);
    usage.partial = limits.len() > 32;
    usage.source = "API key".into();
    usage.plan = ["planName", "plan", "plan_type", "packageName", "level"]
        .iter()
        .find_map(|name| text(data.get(*name)));
    let mut coding = Vec::new();
    for (index, raw) in limits.iter().take(32).enumerate() {
        let kind = raw
            .get("type")
            .and_then(Value::as_str)
            .ok_or_else(|| UsageError::parse("Z.AI quota has no type"))?;
        if !matches!(kind, "TOKENS_LIMIT" | "CREDIT_LIMIT" | "TIME_LIMIT") {
            usage.partial = true;
            continue;
        }
        let reported = raw
            .get("percentage")
            .and_then(Value::as_i64)
            .ok_or_else(|| UsageError::parse("Z.AI quota has no integer percentage"))?;
        let unit = raw
            .get("unit")
            .and_then(Value::as_i64)
            .ok_or_else(|| UsageError::parse("Z.AI quota has no integer unit"))?;
        let number = raw
            .get("number")
            .and_then(Value::as_i64)
            .ok_or_else(|| UsageError::parse("Z.AI quota has no integer duration"))?;
        let duration = if kind == "TIME_LIMIT" && unit == 5 && number == 1 {
            Some(30 * 86_400)
        } else {
            match unit {
                1 => Some(86_400u64),
                3 => Some(3_600),
                5 => Some(60),
                6 => Some(604_800),
                _ => None,
            }
            .and_then(|multiplier| {
                u64::try_from(number)
                    .ok()
                    .filter(|number| *number > 0)
                    .and_then(|number| multiplier.checked_mul(number))
            })
        };
        let allowance = integer(raw, "usage")?;
        let current = integer(raw, "currentValue")?;
        let remaining = integer(raw, "remaining")?;
        let (percent, used) = if let Some(cap) = allowance.filter(|limit| *limit > 0) {
            let derived = remaining.map(|remaining| cap.saturating_sub(remaining));
            let used = match (derived, current) {
                (Some(a), Some(b)) => Some(a.max(b)),
                (a, b) => a.or(b),
            };
            match used {
                Some(used) => {
                    let used = used.clamp(0, cap);
                    (used as f64 * 100.0 / cap as f64, Some(used as f64))
                }
                None => ((reported as f64).clamp(0.0, 100.0), None),
            }
        } else {
            ((reported as f64).clamp(0.0, 100.0), None)
        };
        let label = if kind == "TIME_LIMIT" {
            "MCP".into()
        } else {
            duration
                .map(super::codex::format_duration)
                .unwrap_or_else(|| "Quota".into())
        };
        let id = if kind == "TIME_LIMIT" {
            format!("mcp-{index}")
        } else {
            format!("quota-{index}")
        };
        let mut window = Window::new(id, label, percent);
        window.window_seconds = duration;
        window.used = used;
        window.limit = allowance.map(|value| value as f64);
        window.unit = Some(
            if kind == "TIME_LIMIT" {
                "calls"
            } else if kind == "CREDIT_LIMIT" {
                "credits"
            } else {
                "tokens"
            }
            .into(),
        );
        if let Some(reset_ms) = integer(raw, "nextResetTime")? {
            if let Ok(reset) =
                OffsetDateTime::from_unix_timestamp_nanos(i128::from(reset_ms) * 1_000_000)
            {
                if !(kind != "TIME_LIMIT"
                    && duration == Some(18_000)
                    && reset > now + time::Duration::seconds(18_060))
                {
                    window.resets_at = Some(model::timestamp(reset));
                }
            }
        }
        if kind == "TIME_LIMIT" {
            usage.windows.push(window);
        } else {
            coding.push(window);
        }
    }
    coding.sort_by_key(|window| window.window_seconds.unwrap_or(u64::MAX));
    for (index, mut window) in coding.into_iter().enumerate() {
        window.id = match window.window_seconds {
            Some(18_000) if index == 0 => "session".into(),
            Some(604_800) => "weekly".into(),
            _ => window.id,
        };
        usage.windows.insert(index, window);
    }
    if !usage.has_data() {
        return Err(UsageError::parse(
            "Z.AI returned no recognized quotas; check the Coding Plan usage dashboard",
        ));
    }
    usage.update_legacy_fields();
    Ok(usage)
}

fn integer(value: &Value, key: &str) -> Result<Option<i64>, UsageError> {
    match value.get(key).filter(|value| !value.is_null()) {
        Some(value) => value
            .as_i64()
            .map(Some)
            .ok_or_else(|| UsageError::parse(format!("Z.AI quota {key} is not an integer"))),
        None => Ok(None),
    }
}
