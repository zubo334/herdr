use crate::*;
use serde_json::{json, Value};
use std::{collections::BTreeMap, sync::Mutex};
use time::OffsetDateTime;

struct Mock {
    responses: Mutex<Vec<HttpResponse>>,
    requests: Mutex<Vec<(String, BTreeMap<String, String>)>>,
}
impl Mock {
    fn new(responses: Vec<(u16, Value)>) -> Self {
        Self {
            responses: Mutex::new(
                responses
                    .into_iter()
                    .rev()
                    .map(|(status, body)| HttpResponse {
                        status,
                        headers: BTreeMap::new(),
                        body: serde_json::to_vec(&body).unwrap(),
                    })
                    .collect(),
            ),
            requests: Mutex::new(Vec::new()),
        }
    }
}
impl HttpTransport for Mock {
    fn get(&self, url: &str, headers: &[(&str, &str)]) -> Result<HttpResponse, UsageError> {
        self.requests.lock().unwrap().push((
            url.into(),
            headers
                .iter()
                .map(|(k, v)| ((*k).into(), (*v).into()))
                .collect(),
        ));
        self.responses
            .lock()
            .unwrap()
            .pop()
            .ok_or_else(|| UsageError::new(ErrorKind::Network, "offline"))
    }
}
fn env() -> Environment {
    Environment {
        home: std::env::temp_dir().join("herdr-usage-no-real-home"),
        values: BTreeMap::new(),
        secure_store: None,
    }
}
fn config() -> Config {
    let mut config = Config::default();
    config.claude.enabled = false;
    config.zai.enabled = false;
    config.codex.endpoint = Some("https://usage.test/codex".into());
    config
}
fn now() -> OffsetDateTime {
    OffsetDateTime::from_unix_timestamp(1_800_000_000).unwrap()
}
fn codex_data() -> Value {
    json!({"plan_type":"future-plan", "rate_limit": {
        "primary_window":{"used_percent":12,"limit_window_seconds":18000,"reset_at":1800010000},
        "secondary_window":{"used_percent":50,"limit_window_seconds":604800}
    }})
}

#[test]
fn codex_durations_do_not_promote_weekly_usage_to_session() {
    let usage = providers::codex::parse(&json!({"rate_limit": {
        "primary_window":{"used_percent":0,"limit_window_seconds":604800}
    }}))
    .unwrap();
    assert_eq!(usage.session_percent, None);
    assert_eq!(usage.weekly_percent, Some(0));
    assert_eq!(usage.windows[0].label, "Weekly");
}

#[test]
fn codex_keeps_valid_siblings_credits_and_unknown_plans() {
    let usage = providers::codex::parse(&json!({
        "plan_type":"new-plan",
        "rate_limit":{"primary_window":{"used_percent":"broken"},
            "secondary_window":{"used_percent":120,"limit_window_seconds":604800}},
        "additional_rate_limits":[{"limit_name":"Spark","rate_limit":{
            "primary_window":{"used_percent":35,"limit_window_seconds":18000}}}],
        "credits":{"balance":"12.75","unlimited":false},
        "individual_limit":{"limit":100,"remaining_percent":25}
    }))
    .unwrap();
    assert!(usage.partial);
    assert_eq!(usage.plan.as_deref(), Some("new-plan"));
    assert_eq!(usage.windows[0].used_percent, 120.0);
    assert_eq!(usage.windows[1].label, "Spark 5h");
    assert_eq!(usage.credits.unwrap().balance, Some(12.75));
    assert_eq!(usage.allowances[0].used, Some(75.0));
}

#[test]
fn truncated_additional_codex_limits_are_marked_partial() {
    let extra = json!({"limit_name":"model", "rate_limit": {
        "primary_window":{"used_percent":15,"limit_window_seconds":18000}
    }});
    let usage = providers::codex::parse(&json!({
        "additional_rate_limits": vec![extra; 17],
    }))
    .unwrap();
    assert_eq!(usage.windows.len(), 16);
    assert!(usage.partial);
}

#[test]
fn claude_preserves_fractional_usage_and_spend_is_cents() {
    let usage = providers::claude::parse(&json!({
        "five_hour":{"utilization":12.25,"resets_at":"2027-01-15T10:00:00.123+09:00"},
        "seven_day":null, "seven_day_oauth_apps":{"utilization":0},
        "limits":[{"kind":"weekly_scoped","group":"weekly","is_active":false,"percent":32.5,
            "scope":{"model":{"id":"custom","display_name":"Custom"}}}],
        "extra_usage":{"is_enabled":true,"used_credits":1250,"monthly_limit":10000}
    }))
    .unwrap();
    assert_eq!(usage.windows[0].used_percent, 12.25);
    assert!(usage.windows[0].resets_at.as_ref().unwrap().ends_with('Z'));
    assert_eq!(usage.windows[2].label, "Custom weekly");
    assert_eq!(usage.allowances[0].used, Some(12.5));
    assert_eq!(usage.allowances[0].limit, Some(100.0));
    assert_eq!(usage.weekly_percent, Some(0));
}

#[test]
fn no_quota_is_distinct_from_zero_and_spend_only_is_valid() {
    assert!(providers::claude::parse(&json!({})).is_err());
    assert!(providers::codex::parse(&json!({"credits":{"has_credits":true}})).is_err());
    let usage =
        providers::claude::parse(&json!({"extra_usage":{"is_enabled":true,"used_credits":2450}}))
            .unwrap();
    assert!(usage.windows.is_empty());
    assert_eq!(usage.allowances[0].used, Some(24.5));
}

#[test]
fn zai_uses_counts_sorts_windows_and_keeps_mcp_separate() {
    let usage = providers::zai::parse(&json!({"success":true,"code":200,"data":{"planName":"Pro","limits":[
        {"type":"TOKENS_LIMIT","unit":6,"number":1,"percentage":99,"usage":1000,"currentValue":250,"remaining":800},
        {"type":"TIME_LIMIT","unit":5,"number":1,"percentage":8},
        {"type":"CREDIT_LIMIT","unit":3,"number":5,"percentage":25,"nextResetTime":1800099999000i64},
        {"type":"NEW_LIMIT"}
    ]}}), now()).unwrap();
    assert_eq!(usage.windows[0].id, "session");
    assert_eq!(usage.windows[0].resets_at, None);
    assert_eq!(usage.windows[1].id, "weekly");
    assert_eq!(usage.windows[1].used_percent, 25.0);
    assert_eq!(usage.windows[2].label, "MCP");
    assert_eq!(usage.windows[2].window_seconds, Some(2_592_000));
    assert!(usage.partial);
    assert!(providers::zai::parse(
        &json!({"success":true,"code":200,"data":{"limits":[]}}),
        now()
    )
    .is_err());
}

#[test]
fn claude_fallback_is_bounded_and_preserves_required_headers() {
    let mut config = Config::default();
    config.claude.endpoint = Some("https://usage.test/claude".into());
    let mut env = env();
    env.values
        .insert("HERDR_CLAUDE_OAUTH_TOKEN".into(), "test-secret".into());
    let credential = providers::Claude.credentials(&config, &env).unwrap();
    let mock = Mock::new(vec![
        (400, json!({})),
        (200, json!({"five_hour":{"utilization":12}})),
    ]);
    let usage = providers::Claude
        .fetch(&config, &env, &credential, &mock, now())
        .unwrap();
    assert_eq!(usage.windows[0].used_percent, 12.0);
    let requests = mock.requests.lock().unwrap();
    assert!(requests[0].0.ends_with("?cedar_ember=1"));
    assert_eq!(requests[1].0, "https://usage.test/claude");
    assert_eq!(requests[0].1["anthropic-beta"], "oauth-2025-04-20");
    assert_eq!(requests[1].1["Authorization"], "Bearer test-secret");
}

#[test]
fn permission_error_does_not_trigger_cli_or_alternate_account_fallback() {
    let config = Config::default();
    let mut env = env();
    env.values
        .insert("HERDR_CLAUDE_OAUTH_TOKEN".into(), "test-secret".into());
    let credential = providers::Claude.credentials(&config, &env).unwrap();
    let mock = Mock::new(vec![(403, json!({"error":"user:profile required"}))]);
    assert_eq!(
        providers::Claude
            .fetch(&config, &env, &credential, &mock, now())
            .unwrap_err()
            .kind,
        ErrorKind::Permission
    );
    assert_eq!(mock.requests.lock().unwrap().len(), 1);
}

#[test]
fn retained_data_and_rate_limit_cooldowns_are_account_scoped() {
    let config = config();
    let mut env = env();
    env.values
        .insert("HERDR_CODEX_OAUTH_TOKEN".into(), "account-one".into());
    let good = collect(
        &config,
        &env,
        &UsageSnapshot::default(),
        &Mock::new(vec![(200, codex_data())]),
        now(),
    );
    let limited = collect(
        &config,
        &env,
        &good,
        &Mock::new(vec![(429, json!({}))]),
        now(),
    );
    assert_eq!(
        limited.platforms["codex"].windows,
        good.platforms["codex"].windows
    );
    assert_eq!(
        limited.platforms["codex"].updated_at,
        good.platforms["codex"].updated_at
    );
    let mock = Mock::new(Vec::new());
    let _ = collect(&config, &env, &limited, &mock, now());
    assert!(mock.requests.lock().unwrap().is_empty());
    env.values
        .insert("HERDR_CODEX_OAUTH_TOKEN".into(), "account-two".into());
    let switched = collect(&config, &env, &limited, &mock, now());
    assert!(switched.platforms["codex"].windows.is_empty());
    assert_eq!(mock.requests.lock().unwrap().len(), 1);
    let json = serde_json::to_string(&switched).unwrap();
    assert!(!json.contains("account-one") && !json.contains("account-two"));
}

#[test]
fn credential_paths_are_literal_and_secure_storage_empty_means_default() {
    let mut env = env();
    env.values
        .insert("CLAUDE_CONFIG_DIR".into(), "literal path;not-a-list".into());
    assert_eq!(
        env.claude_home(),
        std::path::PathBuf::from("literal path;not-a-list")
    );
    env.values
        .insert("CLAUDE_SECURESTORAGE_CONFIG_DIR".into(), String::new());
    assert_eq!(env.claude_home(), env.home.join(".claude"));
    env.values
        .insert("CODEX_HOME".into(), "Z:\\login profiles\\one".into());
    assert_eq!(
        env.codex_home(),
        std::path::PathBuf::from("Z:\\login profiles\\one")
    );
}

#[test]
fn tokens_remain_redacted_and_insecure_endpoints_are_rejected() {
    let mut env = env();
    env.values.insert(
        "HERDR_CODEX_OAUTH_TOKEN".into(),
        "secret-oauth-value".into(),
    );
    let credential = providers::Codex.credentials(&config(), &env).unwrap();
    assert!(!format!("{credential:?}").contains("secret-oauth-value"));
    assert!(crate::http::checked_url("http://example.com/usage").is_err());
    assert!(crate::http::checked_url("https://user:password@example.com").is_err());
    assert!(crate::http::checked_url("https://example.com/usage").is_ok());
}

#[test]
fn real_reset_inventory_shapes_filter_expired_paused_and_future_grants() {
    let codex = json!({"available_count":99,"credits":[
        {"status":"available"},{"status":"used"},
        {"status":"available","expires_at":"2000-01-01T00:00:00Z"},
        {"status":"available","expires_at":"2100-01-01T00:00:00Z"},
        {"status":"available","expires_at":"malformed"}
    ]});
    assert_eq!(providers::codex::reset_inventory(&codex, now()), Some(2));
    let claude = json!({"cedar_ember":{"eligible":true,"grants":[
        {"resets_left":2,"paused":false,"usable_now":false},
        {"resets_left":9,"paused":true},
        {"resets_left":9,"paused":false,"starts_at":"2100-01-01T00:00:00Z"},
        {"resets_left":9,"paused":false,"ends_at":"2000-01-01T00:00:00Z"},
        {"resets_left":9,"paused":false,"ends_at":"bad"},
        {"resets_left":9}
    ]}});
    assert_eq!(providers::claude::reset_inventory(&claude, now()), Some(2));
    let bounded = json!({"cedar_ember":{"eligible":true,"grants":[{"resets_left":999999999999u64,"paused":false}]}});
    assert_eq!(providers::claude::reset_inventory(&bounded, now()), None);
}

#[test]
fn retry_after_is_bounded_and_accepts_http_dates() {
    let stamp = now().unix_timestamp();
    assert_eq!(
        crate::http::retry_after(Some("9223372036854775807"), now()),
        stamp + 86400
    );
    assert_eq!(crate::http::retry_after(Some("-1"), now()), stamp + 300);
    assert_eq!(crate::http::retry_after(Some("bad"), now()), stamp + 300);
    assert_eq!(crate::http::retry_after(None, now()), stamp + 300);
    assert_eq!(crate::http::retry_after(Some("30"), now()), stamp + 30);
    let date = time::OffsetDateTime::from_unix_timestamp(stamp + 120)
        .unwrap()
        .format(&time::format_description::well_known::Rfc2822)
        .unwrap()
        .replace("+0000", "GMT");
    assert_eq!(crate::http::retry_after(Some(&date), now()), stamp + 120);
}

#[test]
fn nullable_monthly_credit_cap_falls_back_and_clamps_derived_spend() {
    let usage = providers::codex::parse(&json!({
        "individual_limit":null,"rate_limit":{"individualLimit":{
            "limit":100,"remainingPercent":150,"resetsAt":"2027-02-01T00:00:00Z"}}
    }))
    .unwrap();
    assert_eq!(usage.allowances[0].used, Some(0.0));
    assert_eq!(
        usage.allowances[0].resets_at.as_deref(),
        Some("2027-02-01T00:00:00Z")
    );
}

#[test]
fn zai_preserves_reset_milliseconds_and_rejects_the_precise_skew_boundary() {
    let fixture = |reset| {
        json!({"success":true,"code":200,"data":{"limits":[
            {"type":"TOKENS_LIMIT","unit":3,"number":5,"percentage":10,"nextResetTime":reset}
        ]}})
    };
    let accepted = now().unix_timestamp() * 1000 + 18_060_000;
    assert!(providers::zai::parse(&fixture(accepted), now())
        .unwrap()
        .windows[0]
        .resets_at
        .is_some());
    assert_eq!(
        providers::zai::parse(&fixture(accepted + 1), now())
            .unwrap()
            .windows[0]
            .resets_at,
        None
    );
    assert!(providers::zai::parse(&fixture(accepted - 999), now())
        .unwrap()
        .windows[0]
        .resets_at
        .as_ref()
        .unwrap()
        .contains(".001"));
}

#[test]
fn zai_team_mode_and_endpoint_changes_have_distinct_account_ownership() {
    let mut config = Config::default();
    let mut env = env();
    env.values.insert("Z_AI_API_KEY".into(), "test-key".into());
    env.values
        .insert("Z_AI_BIGMODEL_ORGANIZATION".into(), "org".into());
    env.values
        .insert("Z_AI_BIGMODEL_PROJECT".into(), "project".into());
    let personal = providers::Zai.credentials(&config, &env).unwrap();
    config.zai.team = true;
    let team = providers::Zai.credentials(&config, &env).unwrap();
    assert_ne!(personal.owner_key, team.owner_key);
    config.zai.endpoint = Some("https://proxy.test/quota".into());
    assert_ne!(
        team.owner_key,
        providers::Zai.credentials(&config, &env).unwrap().owner_key
    );
}

#[test]
fn zai_endpoint_infers_region_preserves_paths_and_replaces_team_type() {
    let mut config = Config::default();
    config.zai.team = true;
    let mut env = env();
    env.values.extend([
        ("Z_AI_API_KEY".into(), "test-key".into()),
        ("Z_AI_BIGMODEL_ORGANIZATION".into(), "org".into()),
        ("Z_AI_BIGMODEL_PROJECT".into(), "project".into()),
        (
            "Z_AI_QUOTA_URL".into(),
            "https://open.bigmodel.cn/custom/quota?type=1&other=yes".into(),
        ),
    ]);
    assert!(env.zai_cn(&config));
    let credentials = providers::Zai.credentials(&config, &env).unwrap();
    let mock = Mock::new(vec![(
        200,
        json!({"success":true,"code":200,"data":{"limits":[
        {"type":"TOKENS_LIMIT","unit":3,"number":5,"percentage":0}]}}),
    )]);
    providers::Zai
        .fetch(&config, &env, &credentials, &mock, now())
        .unwrap();
    let requests = mock.requests.lock().unwrap();
    assert_eq!(
        requests[0].0,
        "https://open.bigmodel.cn/custom/quota?other=yes&type=2"
    );
    assert_eq!(requests[0].1["Bigmodel-Organization"], "org");
}

#[test]
fn scoped_weekly_model_names_are_stable_and_all_model_duplicates_are_omitted() {
    let usage = providers::claude::parse(&json!({
        "limits":[
            {"kind":"weekly_scoped","group":"weekly","percent":12,"scope":{"model":{"id":"claude-all-models","display_name":"All"}}},
            {"kind":"weekly_scoped","group":"weekly","percent":34,"scope":{"model":{"display_name":"Custom Model"}}},
            {"kind":"weekly_scoped","group":"weekly","percent":56,"scope":{"model":{"id":"missing-name"}}}
        ]
    })).unwrap();
    assert_eq!(usage.windows.len(), 1);
    assert_eq!(usage.windows[0].id, "custom-model-weekly");
}

#[test]
fn a_different_claude_login_does_not_discard_codex_last_good_cache() {
    let mut config = config();
    config.claude.enabled = true;
    let mut env = env();
    env.values
        .insert("HERDR_CODEX_OAUTH_TOKEN".into(), "codex-a".into());
    env.values
        .insert("HERDR_CLAUDE_OAUTH_TOKEN".into(), "claude-b".into());
    let old = collect(
        &config,
        &env,
        &UsageSnapshot::default(),
        &Mock::new(vec![
            (200, codex_data()),
            (200, json!({"five_hour":{"utilization":25}})),
        ]),
        now(),
    );
    env.values
        .insert("HERDR_CLAUDE_OAUTH_TOKEN".into(), "claude-c".into());
    let owned = owned_snapshot(&config, &env, &old);
    assert!(owned.platforms.contains_key("codex"));
    assert!(!owned.platforms.contains_key("claude"));
    let failed = collect(&config, &env, &owned, &Mock::new(Vec::new()), now());
    assert_eq!(
        failed.platforms["codex"].windows,
        old.platforms["codex"].windows
    );
    assert!(failed.platforms["codex"].error.is_some());
    assert!(failed.platforms["claude"].windows.is_empty());
}

#[test]
fn malformed_claude_reset_grants_do_not_fabricate_inventory_or_hide_quota() {
    let json = json!({"five_hour":{"utilization":25},"cedar_ember":{"eligible":true,"grants":[
        {"resets_left":2,"resets_total":1,"paused":false},
        {"resets_left":2,"resets_total":-1,"paused":false},
        {"resets_left":2,"resets_total":"two","paused":false},
        {"resets_left":1,"resets_total":1,"paused":false}
    ]}});
    assert_eq!(providers::claude::reset_inventory(&json, now()), Some(1));
    let mut excessive = json.clone();
    excessive["cedar_ember"]["grants"] =
        Value::Array(vec![json!({"resets_left":1,"paused":false}); 201]);
    let usage = providers::claude::parse_at(&excessive, now()).unwrap();
    assert_eq!(usage.reset_credits, None);
    assert_eq!(usage.windows[0].used_percent, 25.0);
}

struct Temp(std::path::PathBuf);
impl Temp {
    fn new() -> Self {
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!("herdr-usage-{}-{stamp}", std::process::id()));
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
fn codex_native_credentials_use_access_token_identity_without_modifying_the_file() {
    use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
    let root = Temp::new();
    let token = format!(
        "test.{}.signature",
        URL_SAFE_NO_PAD.encode(
            serde_json::to_vec(&json!({
                "exp":now().unix_timestamp()+3600,
                "organizations":[{"id":"workspace-from-access-token"}],
                "https://api.openai.com/profile":{"email":"person@example.test"}
            }))
            .unwrap()
        )
    );
    let path = root.0.join("auth.json");
    let contents = serde_json::to_vec(&json!({"tokens":{
        "access_token":"","accessToken":token,"refresh_token":"private-refresh-token"
    }}))
    .unwrap();
    std::fs::write(&path, &contents).unwrap();
    let mut config = config();
    config.codex.credential_file = Some(path.clone());
    let credentials = providers::Codex.credentials(&config, &env()).unwrap();
    assert!(credentials.check_expiry(now()).is_ok());
    assert_eq!(
        credentials.account_id.as_deref(),
        Some("workspace-from-access-token")
    );
    assert_eq!(credentials.account.as_deref(), Some("person@example.test"));
    let mock = Mock::new(vec![(200, codex_data())]);
    providers::Codex
        .fetch(&config, &env(), &credentials, &mock, now())
        .unwrap();
    assert_eq!(
        mock.requests.lock().unwrap()[0].1["ChatGPT-Account-Id"],
        "workspace-from-access-token"
    );
    assert_eq!(std::fs::read(path).unwrap(), contents);
}

#[test]
fn claude_credentials_respect_millisecond_expiry_and_never_query_keychain_for_custom_profiles() {
    let root = Temp::new();
    let path = root.0.join(".credentials.json");
    std::fs::write(
        &path,
        serde_json::to_vec(&json!({"claudeAiOauth":{
            "accessToken":"fake-access","refreshToken":"fake-refresh",
            "expiresAt":now().unix_timestamp() as f64 * 1000.0 + 30_000.5,"scopes":["user:profile"]
        }}))
        .unwrap(),
    )
    .unwrap();
    let mut config = Config::default();
    config.claude.credential_file = Some(path);
    let credentials = providers::Claude.credentials(&config, &env()).unwrap();
    assert!(
        credentials.check_expiry(now()).is_ok(),
        "Claude stays usable during its final five minutes"
    );
    let mut env = env();
    env.values.insert(
        "CLAUDE_CONFIG_DIR".into(),
        root.0
            .join("missing custom profile")
            .to_string_lossy()
            .into(),
    );
    env.secure_store =
        Some(|_| panic!("custom profiles must not read the default account Keychain"));
    config.claude.credential_file = None;
    assert!(providers::Claude.credentials(&config, &env).is_err());
}
