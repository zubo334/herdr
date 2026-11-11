use super::App;
use std::{
    sync::Arc,
    time::{Duration, Instant},
};

pub(crate) struct UsageRuntime {
    pub(crate) next_refresh: Option<Instant>,
    generation: u64,
    in_flight: bool,
}
impl UsageRuntime {
    pub(crate) fn new(enabled: bool) -> Self {
        let enabled = enabled && std::env::var("HERDR_USAGE_DISABLED").as_deref() != Ok("1");
        Self {
            next_refresh: enabled.then(Instant::now),
            generation: 0,
            in_flight: false,
        }
    }
}

impl App {
    pub(crate) fn start_usage_refresh_if_due(&mut self, now: Instant) {
        if self.usage_runtime.in_flight
            || self
                .usage_runtime
                .next_refresh
                .is_none_or(|deadline| now < deadline)
        {
            return;
        }
        self.usage_runtime.in_flight = true;
        self.usage_runtime.next_refresh = None;
        self.usage_runtime.generation = self.usage_runtime.generation.wrapping_add(1);
        let generation = self.usage_runtime.generation;
        let previous = Arc::clone(&self.state.usage);
        let tx = self.event_tx.clone();
        if let Err(error) = crate::thread_spawn::spawn_named("herdr-usage", move || {
            let update = crate::usage::collect_refresh(generation, previous);
            let _ = tx.blocking_send(crate::events::AppEvent::UsageRefreshed(Box::new(update)));
        }) {
            tracing::warn!(%error, "could not start usage collection");
            self.usage_runtime.in_flight = false;
            self.usage_runtime.next_refresh = Some(now + Duration::from_secs(60));
        }
    }

    pub(crate) fn handle_usage_refreshed(&mut self, update: crate::usage::Refresh) -> bool {
        if !self.usage_runtime.in_flight || update.generation != self.usage_runtime.generation {
            return false;
        }
        self.usage_runtime.in_flight = false;
        self.usage_runtime.next_refresh =
            Some(Instant::now() + Duration::from_secs(update.interval_seconds.max(5)));
        if self.state.usage == update.snapshot {
            return false;
        }
        self.state.usage = update.snapshot;
        self.state.usage_revision = self.state.usage_revision.saturating_add(1);
        self.render_dirty.request_generic();
        self.render_notify.notify_one();
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn quiet_usage_changes_request_render_but_unchanged_or_stale_results_do_not() {
        let (_tx, rx) = tokio::sync::mpsc::unbounded_channel();
        let mut app = App::new(
            &crate::config::Config::default(),
            super::super::AppPolicy::TEST,
            None,
            rx,
            crate::api::EventHub::default(),
        );
        let _ = app.render_dirty.take();
        assert!(app.usage_runtime.next_refresh.is_none());
        app.usage_runtime.in_flight = true;
        app.usage_runtime.generation = 4;
        let mut snapshot = herdr_usage::UsageSnapshot::default();
        snapshot.platforms.insert(
            "codex".into(),
            herdr_usage::ProviderUsage::new(herdr_usage::ProviderId::Codex),
        );
        let snapshot = Arc::new(snapshot);
        let update = |generation, snapshot| crate::usage::Refresh {
            generation,
            snapshot,
            interval_seconds: 60,
        };
        assert!(!app.handle_usage_refreshed(update(3, Arc::clone(&snapshot))));
        assert!(app.usage_runtime.in_flight);
        assert!(app.handle_usage_refreshed(update(4, Arc::clone(&snapshot))));
        assert_eq!(app.state.usage_revision, 1);
        assert!(app.render_dirty.take().generic);
        app.usage_runtime.in_flight = true;
        assert!(!app.handle_usage_refreshed(update(4, snapshot)));
        assert_eq!(app.state.usage_revision, 1);
        assert!(!app.usage_runtime.in_flight);
        assert!(!app.render_dirty.is_pending());
    }

    #[test]
    fn usage_api_reads_cached_state_without_starting_collection() {
        let (_tx, rx) = tokio::sync::mpsc::unbounded_channel();
        let mut app = App::new(
            &crate::config::Config::default(),
            crate::app::AppPolicy::TEST,
            None,
            rx,
            crate::api::EventHub::default(),
        );
        let mut snapshot = herdr_usage::UsageSnapshot::default();
        let mut provider = herdr_usage::ProviderUsage::new(herdr_usage::ProviderId::Codex);
        provider
            .windows
            .push(herdr_usage::Window::new("session", "5h", 37.5));
        snapshot.platforms.insert("codex".into(), provider);
        app.state.usage = Arc::new(snapshot);
        let value: serde_json::Value = serde_json::from_str(
            &app.handle_api_request_after_internal_events_drained(crate::api::schema::Request {
                id: "usage-read".into(),
                method: crate::api::schema::Method::UsageGet(
                    crate::api::schema::EmptyParams::default(),
                ),
            }),
        )
        .unwrap();
        assert_eq!(
            value["result"]["usage"]["platforms"]["codex"]["windows"][0]["used_percent"],
            37.5
        );
        assert!(!app.usage_runtime.in_flight);
        assert!(app.usage_runtime.next_refresh.is_none());
    }
}
