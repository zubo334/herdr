use crate::{ErrorKind, UsageError};
use std::{collections::BTreeMap, io::Read, time::Duration};

pub struct HttpResponse {
    pub status: u16,
    pub headers: BTreeMap<String, String>,
    pub body: Vec<u8>,
}

pub trait HttpTransport: Send + Sync {
    fn get(&self, url: &str, headers: &[(&str, &str)]) -> Result<HttpResponse, UsageError>;
}

pub struct ReqwestTransport {
    client: reqwest::blocking::Client,
}

impl ReqwestTransport {
    pub fn new(timeout_seconds: u64) -> Result<Self, UsageError> {
        let policy = reqwest::redirect::Policy::custom(|attempt| {
            let current = attempt.url();
            let original = attempt.previous().first();
            if attempt.previous().len() < 3
                && original.is_some_and(|original| {
                    original.scheme() == "https"
                        && current.scheme() == "https"
                        && original.host_str() == current.host_str()
                        && original.port_or_known_default() == current.port_or_known_default()
                        && current.username().is_empty()
                        && current.password().is_none()
                })
            {
                attempt.follow()
            } else {
                attempt.stop()
            }
        });
        let client = reqwest::blocking::Client::builder()
            .timeout(Duration::from_secs(timeout_seconds.clamp(1, 60)))
            .redirect(policy)
            .user_agent("herdr-usage (CodexBar port)")
            .build()
            .map_err(|_| {
                UsageError::new(ErrorKind::Network, "Could not initialize HTTPS transport")
            })?;
        Ok(Self { client })
    }
}

pub fn checked_url(value: &str) -> Result<String, UsageError> {
    let url = reqwest::Url::parse(value)
        .map_err(|_| UsageError::new(ErrorKind::Configuration, "Invalid usage endpoint URL"))?;
    if url.scheme() != "https"
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
    {
        return Err(UsageError::new(
            ErrorKind::Configuration,
            "Usage endpoints must use HTTPS without URL credentials",
        ));
    }
    Ok(url.to_string())
}

impl HttpTransport for ReqwestTransport {
    fn get(&self, url: &str, headers: &[(&str, &str)]) -> Result<HttpResponse, UsageError> {
        let url = checked_url(url)?;
        let mut request = self.client.get(url);
        for (name, value) in headers {
            request = request.header(*name, *value);
        }
        let response = request.send().map_err(|_| {
            UsageError::new(ErrorKind::Network, "Usage request failed or timed out")
        })?;
        let status = response.status().as_u16();
        let headers = response
            .headers()
            .iter()
            .filter_map(|(key, value)| {
                value
                    .to_str()
                    .ok()
                    .map(|value| (key.as_str().to_owned(), value.to_owned()))
            })
            .collect();
        const LIMIT: u64 = 1024 * 1024;
        let mut body = Vec::new();
        response
            .take(LIMIT + 1)
            .read_to_end(&mut body)
            .map_err(|_| UsageError::new(ErrorKind::Network, "Could not read usage response"))?;
        if body.len() as u64 > LIMIT {
            return Err(UsageError::parse("Usage response exceeds 1 MiB"));
        }
        Ok(HttpResponse {
            status,
            headers,
            body,
        })
    }
}

pub(crate) fn json_response(
    response: HttpResponse,
    now: time::OffsetDateTime,
) -> Result<serde_json::Value, UsageError> {
    if !(200..300).contains(&response.status) {
        let (kind, message) = match response.status {
            401 => (
                ErrorKind::Authentication,
                "Login has expired; refresh the provider CLI login",
            ),
            403 => (
                ErrorKind::Permission,
                "Usage access was denied for this account",
            ),
            429 => (
                ErrorKind::RateLimited,
                "Usage API rate limit; retry after the cooldown",
            ),
            _ => (
                ErrorKind::Api,
                "Usage API returned an unsuccessful response",
            ),
        };
        let mut error = UsageError::new(kind, format!("{message} (HTTP {})", response.status));
        if response.status == 429 {
            error.retry_at = Some(retry_after(
                response.headers.get("retry-after").map(String::as_str),
                now,
            ));
        }
        return Err(error);
    }
    serde_json::from_slice(&response.body)
        .map_err(|_| UsageError::parse("Invalid JSON from usage API"))
}

pub(crate) fn retry_after(value: Option<&str>, now: time::OffsetDateTime) -> i64 {
    let delay = value.and_then(|value| {
        value.trim().parse::<i64>().ok().filter(|value| *value >= 0).or_else(|| {
            let format = time::format_description::parse_borrowed::<2>(
                "[weekday repr:short], [day] [month repr:short] [year] [hour]:[minute]:[second] GMT").ok()?;
            let date = time::PrimitiveDateTime::parse(value.trim(), &format).ok()?.assume_utc();
            Some((date - now).whole_seconds().max(0))
        })
    }).unwrap_or(300).clamp(0, 86_400);
    now.unix_timestamp().saturating_add(delay)
}
