use super::{finite, quota_percent, reset, text};
use crate::{
    credentials, http, Allowance, Config, Credential, Environment, HttpTransport, ProviderId,
    ProviderUsage, UsageError, UsageProvider, Window,
};
use serde_json::Value;
use time::OffsetDateTime;

pub struct Claude;
impl UsageProvider for Claude {
    fn id(&self) -> ProviderId {
        ProviderId::Claude
    }
    fn credentials(&self, config: &Config, env: &Environment) -> Result<Credential, UsageError> {
        credentials::claude(config, env)
    }
    fn fetch(
        &self,
        config: &Config,
        _env: &Environment,
        credential: &Credential,
        transport: &dyn HttpTransport,
        now: OffsetDateTime,
    ) -> Result<ProviderUsage, UsageError> {
        let base = http::checked_url(
            config
                .claude
                .endpoint
                .as_deref()
                .unwrap_or("https://api.anthropic.com/api/oauth/usage"),
        )?;
        let mut url = reqwest::Url::parse(&base)
            .map_err(|_| UsageError::parse("Invalid Claude usage URL"))?;
        url.query_pairs_mut().append_pair("cedar_ember", "1");
        let bearer = credential.bearer();
        let headers = [
            ("Authorization", bearer.as_str()),
            ("Accept", "application/json"),
            ("Content-Type", "application/json"),
            ("anthropic-beta", "oauth-2025-04-20"),
            ("User-Agent", "claude-cli/2.1.0 (external, cli)"),
        ];
        let mut response = transport.get(url.as_str(), &headers)?;
        if response.status == 400
            || (response.status == 403
                && !String::from_utf8_lossy(&response.body).contains("user:profile"))
        {
            let mut headers = headers;
            headers[4] = ("User-Agent", "claude-code/2.1.0");
            response = transport.get(&base, &headers)?;
        }
        let json = http::json_response(response, now)?;
        let mut usage = parse_at(&json, now)?;
        usage.plan = credential.plan.clone();
        Ok(usage)
    }
}

#[cfg(test)]
pub fn parse(json: &Value) -> Result<ProviderUsage, UsageError> {
    parse_at(json, OffsetDateTime::now_utc())
}

pub(crate) fn parse_at(json: &Value, now: OffsetDateTime) -> Result<ProviderUsage, UsageError> {
    if !json.is_object() {
        return Err(UsageError::parse("Invalid Claude usage envelope"));
    }
    let mut usage = ProviderUsage::new(ProviderId::Claude);
    usage.source = "OAuth API".into();
    for (key, id, label, duration) in [
        ("five_hour", "session", "5h", 18_000),
        ("seven_day", "weekly", "Weekly", 604_800),
        ("seven_day_oauth_apps", "weekly", "Weekly", 604_800),
        (
            "seven_day_sonnet",
            "sonnet-weekly",
            "Sonnet weekly",
            604_800,
        ),
        ("seven_day_opus", "opus-weekly", "Opus weekly", 604_800),
    ] {
        if usage.windows.iter().any(|window| window.id == id) {
            continue;
        }
        let Some(raw) = json.get(key).filter(|value| !value.is_null()) else {
            continue;
        };
        match quota_percent(raw.get("utilization")) {
            Ok(percent) => {
                let mut window = Window::new(id, label, percent);
                window.window_seconds = Some(duration);
                window.resets_at = reset(raw.get("resets_at"));
                usage.windows.push(window);
            }
            Err(_) => usage.partial = true,
        }
    }
    if let Some(limits) = json.get("limits").and_then(Value::as_array) {
        usage.partial |= limits.len() > 16;
        for limit in limits.iter().take(16) {
            if limit.get("kind").and_then(Value::as_str) != Some("weekly_scoped")
                || limit.get("group").and_then(Value::as_str) != Some("weekly")
            {
                continue;
            }
            let Some(percent) = finite(limit.get("percent")) else {
                usage.partial = true;
                continue;
            };
            let model = limit.pointer("/scope/model");
            let Some(name) = model.and_then(|model| text(model.get("display_name"))) else {
                usage.partial = true;
                continue;
            };
            let id = model
                .and_then(|model| text(model.get("id")))
                .unwrap_or_else(|| {
                    name.to_lowercase()
                        .chars()
                        .map(|ch| if ch.is_alphanumeric() { ch } else { '-' })
                        .collect()
                });
            if name.eq_ignore_ascii_case("all models")
                || id == "all-models"
                || id.ends_with("-all-models")
            {
                continue;
            }
            if usage
                .windows
                .iter()
                .any(|window| window.id == format!("{id}-weekly"))
            {
                continue;
            }
            let mut window = Window::new(format!("{id}-weekly"), format!("{name} weekly"), percent);
            window.window_seconds = Some(604_800);
            window.resets_at = reset(limit.get("resets_at"));
            usage.windows.push(window);
        }
    }
    if let Some(extra) = json
        .get("extra_usage")
        .filter(|extra| extra.get("is_enabled").and_then(Value::as_bool) == Some(true))
    {
        let used = finite(extra.get("used_credits")).map(|cents| cents / 100.0);
        let limit = finite(extra.get("monthly_limit")).map(|cents| cents / 100.0);
        if used.is_some() || limit.is_some() {
            usage.allowances.push(Allowance {
                label: "Extra usage".into(),
                used,
                limit,
                currency: text(extra.get("currency")).unwrap_or_else(|| "USD".into()),
                resets_at: None,
            });
        }
    }
    usage.reset_credits = reset_inventory(json, now);
    if !usage.has_data() {
        return Err(UsageError::parse(
            "Claude returned no recognizable quota or spend data",
        ));
    }
    usage.update_legacy_fields();
    Ok(usage)
}

pub(crate) fn reset_inventory(json: &Value, now: OffsetDateTime) -> Option<u32> {
    let inventory = json.get("cedar_ember")?.as_object()?;
    if inventory.get("eligible").and_then(Value::as_bool) != Some(true) {
        return None;
    }
    let grants = inventory.get("grants")?.as_array()?;
    if grants.len() > 200 {
        return None;
    }
    let mut total = 0u32;
    for grant in grants.iter().take(200) {
        if grant.get("paused").and_then(Value::as_bool) != Some(false) {
            continue;
        }
        let bound = |key: &str| -> Result<Option<OffsetDateTime>, ()> {
            grant
                .get(key)
                .filter(|value| !value.is_null())
                .map(|value| {
                    value.as_str().ok_or(()).and_then(|value| {
                        OffsetDateTime::parse(value, &time::format_description::well_known::Rfc3339)
                            .map_err(|_| ())
                    })
                })
                .transpose()
        };
        let (Ok(start), Ok(end)) = (bound("starts_at"), bound("ends_at")) else {
            continue;
        };
        if start.is_some_and(|start| start > now) || end.is_some_and(|end| end <= now) {
            continue;
        }
        if let Some(left) = grant.get("resets_left").and_then(Value::as_u64) {
            if let Some(value) = grant.get("resets_total").filter(|value| !value.is_null()) {
                if value.as_u64().is_none_or(|value| value < left) {
                    continue;
                }
            }
            if left > u64::from(50 - total) {
                return None;
            }
            total += left as u32;
        }
    }
    (total > 0).then_some(total)
}
