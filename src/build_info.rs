//! Build identity helpers.

pub const BASE_VERSION: &str = env!("CARGO_PKG_VERSION");

pub fn channel() -> &'static str {
    non_empty(option_env!("HERDR_BUILD_CHANNEL")).unwrap_or("stable")
}

pub fn build_id() -> Option<&'static str> {
    non_empty(option_env!("HERDR_BUILD_ID"))
}

pub fn commit() -> Option<&'static str> {
    non_empty(option_env!("HERDR_BUILD_COMMIT"))
}

pub fn version() -> String {
    match channel() {
        "stable" => BASE_VERSION.to_string(),
        "preview" => build_id()
            .map(|id| preview_version(BASE_VERSION, id))
            .unwrap_or_else(|| format!("{BASE_VERSION}-preview")),
        channel => match build_id() {
            Some(build_id) => format!("{BASE_VERSION}-{channel}.{build_id}"),
            None => format!("{BASE_VERSION}-{channel}"),
        },
    }
}

/// Fork tags are already complete public versions. Other preview IDs retain
/// the upstream BASE-preview.ID representation.
pub(crate) fn preview_version(base_version: &str, build_id: &str) -> String {
    if build_id.starts_with("fork-v") {
        build_id.to_string()
    } else {
        format!(
            "{}-preview.{build_id}",
            base_version.trim_start_matches('v')
        )
    }
}

pub fn is_preview() -> bool {
    channel() == "preview"
}

fn non_empty(value: Option<&'static str>) -> Option<&'static str> {
    value.and_then(|value| {
        let trimmed = value.trim();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed)
        }
    })
}

#[cfg(test)]
mod tests {
    #[test]
    fn fork_preview_version_is_the_exact_release_tag() {
        let tag = "fork-v0.9.5-preview.2026-11-11-086fa74f";
        assert_eq!(super::preview_version("0.9.5", tag), tag);
        assert_eq!(
            super::preview_version("v0.9.5", "2026-11-11-086fa74f638c"),
            "0.9.5-preview.2026-11-11-086fa74f638c"
        );
    }
    #[test]
    fn stable_version_defaults_to_cargo_version() {
        assert!(!super::version().is_empty());
    }
}
