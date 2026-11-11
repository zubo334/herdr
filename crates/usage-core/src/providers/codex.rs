use super::{finite, text};
use crate::{
    credentials, http, model, Allowance, Config, Credential, Credits, Environment, HttpTransport,
    ProviderId, ProviderUsage, UsageError, UsageProvider, Window,
};
use serde_json::Value;
use time::OffsetDateTime;

pub struct Codex;

impl UsageProvider for Codex {
    fn id(&self) -> ProviderId {
        ProviderId::Codex
    }
    fn credentials(&self, config: &Config, env: &Environment) -> Result<Credential, UsageError> {
        credentials::codex(config, env)
    }
    fn fetch(
        &self,
        config: &Config,
        _env: &Environment,
        credential: &Credential,
        transport: &dyn HttpTransport,
        now: OffsetDateTime,
    ) -> Result<ProviderUsage, UsageError> {
        let url = http::checked_url(
            config
                .codex
                .endpoint
                .as_deref()
                .unwrap_or("https://chatgpt.com/backend-api/wham/usage"),
        )?;
        let bearer = credential.bearer();
        let mut headers = vec![
            ("Authorization", bearer.as_str()),
            ("Accept", "application/json"),
        ];
        if let Some(account) = credential.account_id.as_deref() {
            headers.push(("ChatGPT-Account-Id", account));
        }
        let json = http::json_response(transport.get(&url, &headers)?, now)?;
        let mut usage = parse(&json)?;
        usage.account = credential.account.clone();
        if usage.plan.is_none() {
            usage.plan = credential.plan.clone();
        }
        // Reset inventory is optional and must use the same in-memory account credential.
        // Explicit proxy overrides apply only to the requested quota endpoint.
        if config.codex.endpoint.is_none() {
            let mut extra_headers = headers.clone();
            extra_headers.extend([("OpenAI-Beta", "codex-1"), ("originator", "Codex Desktop")]);
            if let Ok(response) = transport.get(
                "https://chatgpt.com/backend-api/wham/rate-limit-reset-credits",
                &extra_headers,
            ) {
                if let Ok(value) = http::json_response(response, now) {
                    usage.reset_credits = reset_inventory(&value, now);
                }
            }
        }
        Ok(usage)
    }
}

pub fn parse(json: &Value) -> Result<ProviderUsage, UsageError> {
    if !json.is_object() {
        return Err(UsageError::parse("Invalid Codex usage envelope"));
    }
    let mut usage = ProviderUsage::new(ProviderId::Codex);
    usage.source = "OAuth API".into();
    usage.plan = text(json.get("plan_type"));
    if let Some(limits) = json.get("rate_limit").filter(|value| !value.is_null()) {
        parse_windows(limits, "", "", &mut usage);
    }
    if let Some(limits) = json
        .get("code_review_rate_limit")
        .filter(|value| !value.is_null())
    {
        parse_windows(limits, "review-", "Review ", &mut usage);
    }
    if let Some(extras) = json.get("additional_rate_limits") {
        if let Some(extras) = extras.as_array() {
            usage.partial |= extras.len() > 16;
            for (index, extra) in extras.iter().take(16).enumerate() {
                let name = text(extra.get("limit_name"))
                    .or_else(|| text(extra.get("metered_feature")))
                    .unwrap_or_else(|| format!("Extra {}", index + 1));
                if let Some(limit) = extra.get("rate_limit") {
                    parse_windows(
                        limit,
                        &format!("extra-{index}-"),
                        &format!("{name} "),
                        &mut usage,
                    );
                } else {
                    usage.partial = true;
                }
            }
        } else if !extras.is_null() {
            usage.partial = true;
        }
    }
    if let Some(credits) = json.get("credits").filter(|value| value.is_object()) {
        let balance = finite(credits.get("balance"));
        let unlimited = credits
            .get("unlimited")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        if balance.is_some() || unlimited {
            usage.credits = Some(Credits {
                balance,
                unlimited,
                currency: None,
            });
        }
    }
    let individual = [
        "/individual_limit",
        "/individualLimit",
        "/rate_limit/individual_limit",
        "/rate_limit/individualLimit",
        "/spend_control/individual_limit",
        "/spend_control/individualLimit",
        "/spendControl/individual_limit",
        "/spendControl/individualLimit",
    ]
    .iter()
    .find_map(|path| {
        let value = json.pointer(path)?.as_object()?;
        finite(value.get("limit"))
            .filter(|limit| *limit > 0.0)
            .map(|cap| (value, cap))
    });
    if let Some((limit, cap)) = individual {
        let used = finite(limit.get("used"))
            .filter(|used| *used >= 0.0)
            .or_else(|| {
                let remaining = finite(
                    limit
                        .get("remaining_percent")
                        .or_else(|| limit.get("remainingPercent")),
                )?;
                Some(cap * (100.0 - remaining.clamp(0.0, 100.0)) / 100.0)
            });
        let reset = limit
            .get("resets_at")
            .or_else(|| limit.get("resetsAt"))
            .or_else(|| limit.get("reset_at"));
        let resets_at = super::reset(reset).or_else(|| {
            reset
                .and_then(Value::as_i64)
                .and_then(model::unix_timestamp)
        });
        usage.allowances.push(Allowance {
            label: "Monthly credits".into(),
            used,
            limit: Some(cap),
            currency: "credits".into(),
            resets_at,
        });
    }
    if !usage.has_data() {
        return Err(UsageError::parse(
            "Codex returned no recognizable quota or credit data",
        ));
    }
    if usage.windows.len() > 32 {
        usage.windows.truncate(32);
        usage.partial = true;
    }
    usage.update_legacy_fields();
    Ok(usage)
}

pub(crate) fn reset_inventory(json: &Value, now: OffsetDateTime) -> Option<u32> {
    let records = json.get("credits")?.as_array()?;
    Some(
        records
            .iter()
            .take(200)
            .filter(|record| {
                record.get("status").and_then(Value::as_str) == Some("available")
                    && match record.get("expires_at").filter(|value| !value.is_null()) {
                        Some(value) => value
                            .as_str()
                            .and_then(|value| {
                                OffsetDateTime::parse(
                                    value,
                                    &time::format_description::well_known::Rfc3339,
                                )
                                .ok()
                            })
                            .is_some_and(|expiry| expiry > now),
                        None => true,
                    }
            })
            .count() as u32,
    )
}

fn parse_windows(limits: &Value, prefix: &str, label_prefix: &str, usage: &mut ProviderUsage) {
    for (slot, fallback) in [
        ("primary_window", "primary"),
        ("secondary_window", "secondary"),
    ] {
        let Some(raw) = limits.get(slot).filter(|value| !value.is_null()) else {
            continue;
        };
        let Some(percent) = raw
            .get("used_percent")
            .and_then(Value::as_f64)
            .filter(|value| value.is_finite())
        else {
            usage.partial = true;
            continue;
        };
        let duration = raw.get("limit_window_seconds").and_then(Value::as_u64);
        let (id, label) = match duration {
            Some(18_000) => ("session".to_owned(), "5h".to_owned()),
            Some(604_800) => ("weekly".to_owned(), "Weekly".to_owned()),
            Some(seconds) if seconds > 0 => (fallback.to_owned(), format_duration(seconds)),
            _ => (
                fallback.to_owned(),
                format!(
                    "{} window",
                    if slot == "primary_window" {
                        "Primary"
                    } else {
                        "Secondary"
                    }
                ),
            ),
        };
        let mut window = Window::new(
            format!("{prefix}{id}"),
            format!("{label_prefix}{label}"),
            percent,
        );
        window.window_seconds = duration;
        window.resets_at = raw
            .get("reset_at")
            .and_then(Value::as_i64)
            .and_then(model::unix_timestamp);
        usage.windows.push(window);
    }
}

pub(super) fn format_duration(seconds: u64) -> String {
    if seconds.is_multiple_of(604_800) {
        format!("{}wk", seconds / 604_800)
    } else if seconds.is_multiple_of(86_400) {
        format!("{}d", seconds / 86_400)
    } else if seconds.is_multiple_of(3_600) {
        format!("{}h", seconds / 3_600)
    } else {
        format!("{}m", seconds / 60)
    }
}
