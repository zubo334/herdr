//! Distribution endpoints for this fork, shared by updates and SSH bootstrap.

pub(crate) const PREVIEW_MANIFEST_URL: &str =
    "https://github.com/zubo334/herdr/releases/latest/download/preview.json";
pub(crate) const INSTALL_MANIFEST_URL: &str =
    "https://github.com/zubo334/herdr/releases/latest/download/latest.json";
const RELEASE_DOWNLOAD_URL: &str = "https://github.com/zubo334/herdr/releases/download";

fn resolve_url(explicit: Option<String>, default: &str) -> String {
    explicit
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| default.to_owned())
}

pub(crate) fn preview_manifest_url() -> String {
    resolve_url(
        std::env::var("HERDR_PREVIEW_MANIFEST_URL").ok(),
        PREVIEW_MANIFEST_URL,
    )
}

pub(crate) fn install_manifest_url() -> String {
    resolve_url(
        std::env::var("HERDR_UPDATE_MANIFEST_URL").ok(),
        INSTALL_MANIFEST_URL,
    )
}

pub(crate) fn preview_manifest_for_build(build_id: &str) -> String {
    resolve_preview_build_url(std::env::var("HERDR_PREVIEW_MANIFEST_URL").ok(), build_id)
}

fn resolve_preview_build_url(explicit: Option<String>, build_id: &str) -> String {
    let default = if build_id.starts_with("fork-v") {
        format!("{RELEASE_DOWNLOAD_URL}/{build_id}/preview.json")
    } else {
        PREVIEW_MANIFEST_URL.to_owned()
    };
    resolve_url(explicit, &default)
}

pub(crate) fn channel_rejection(channel: &str) -> Option<&'static str> {
    (channel == "stable")
        .then_some("This fork distributes preview releases only; run `herdr channel set preview`.")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_or_empty_overrides_keep_fork_distribution() {
        for value in [None, Some(String::new()), Some("  ".into())] {
            assert_eq!(
                resolve_url(value, PREVIEW_MANIFEST_URL),
                PREVIEW_MANIFEST_URL
            );
        }
        assert_eq!(
            resolve_url(
                Some("http://127.0.0.1/test.json".into()),
                PREVIEW_MANIFEST_URL
            ),
            "http://127.0.0.1/test.json"
        );
        assert!(channel_rejection("stable").is_some());
        assert!(channel_rejection("preview").is_none());
    }

    #[test]
    fn ssh_downloads_the_immutable_build_even_after_latest_advances() {
        let tag = "fork-v0.9.5-preview.2026-11-11-162dd3cb";
        assert_eq!(
            resolve_preview_build_url(None, tag),
            format!("{RELEASE_DOWNLOAD_URL}/{tag}/preview.json")
        );
        assert_eq!(
            resolve_preview_build_url(Some("http://127.0.0.1/override.json".into()), tag),
            "http://127.0.0.1/override.json"
        );
    }
}
