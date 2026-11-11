//! Translated direct-fetch strategies from CodexBar (MIT, Peter Steinberger).
pub(crate) mod claude;
pub(crate) mod codex;
pub(crate) mod zai;

pub use claude::Claude;
pub use codex::Codex;
pub use zai::Zai;

use crate::{model, UsageError, UsageProvider};
use serde_json::Value;

pub fn registry() -> [&'static dyn UsageProvider; 3] {
    [&Codex, &Claude, &Zai]
}

fn finite(value: Option<&Value>) -> Option<f64> {
    value.and_then(model::number)
}

fn quota_percent(value: Option<&Value>) -> Result<f64, UsageError> {
    value
        .and_then(Value::as_f64)
        .filter(|value| value.is_finite())
        .ok_or_else(|| UsageError::parse("Quota window has no finite utilization"))
}

fn text(value: Option<&Value>) -> Option<String> {
    value
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|text| !text.is_empty())
        .map(str::to_owned)
}

fn reset(value: Option<&Value>) -> Option<String> {
    let text = text(value)?;
    time::OffsetDateTime::parse(&text, &time::format_description::well_known::Rfc3339)
        .ok()
        .map(model::timestamp)
}
