use super::*;

fn projection(revision: u64, name: &str) -> crate::protocol::endpoint::EndpointUsageProjection {
    let mut usage = herdr_usage::ProviderUsage::new(herdr_usage::ProviderId::Zai);
    usage.display_name = name.into();
    usage
        .windows
        .push(herdr_usage::Window::new("session", "5h", 12.5));
    let mut snapshot = herdr_usage::UsageSnapshot::default();
    snapshot.platforms.insert("zai".into(), usage);
    crate::protocol::endpoint::EndpointUsageProjection {
        boot_id: "boot-1".into(),
        revision,
        usage: snapshot,
    }
}

#[test]
fn usage_waits_for_matching_boot_and_generation_and_survives_unrelated_snapshots() {
    let mut state = ClientShellState::new(ClientShellConfig::from_config(&Config::default()));
    let id = ClientEndpointId::Local;
    assert!(state.set_endpoint_usage(&id, 7, projection(1, "Z.AI")));
    assert!(crate::client::shell::usage::snapshot(&state.endpoints[0]).is_none());
    state.set_endpoint_snapshot_for_generation(&id, 7, Box::new(snapshot()));
    assert!(crate::client::shell::usage::snapshot(&state.endpoints[0]).is_some());
    let mut updated = snapshot();
    updated.revision = 99;
    state.set_endpoint_snapshot_for_generation(&id, 7, Box::new(updated));
    assert!(crate::client::shell::usage::snapshot(&state.endpoints[0]).is_some());
    assert!(!state.set_endpoint_usage(&id, 6, projection(100, "old connection")));
    assert!(!state.set_endpoint_usage(&id, 7, projection(1, "old data")));
    state.set_endpoint_snapshot_for_generation(&id, 8, Box::new(snapshot()));
    assert!(crate::client::shell::usage::snapshot(&state.endpoints[0]).is_none());
}

#[test]
fn usage_column_keeps_toggle_geometry_and_renders_fractional_generic_provider_data() {
    let mut state = ClientShellState::new(ClientShellConfig::from_config(&Config::default()));
    state.usage_column_collapsed = false;
    let id = ClientEndpointId::Local;
    state.set_endpoint_snapshot_for_generation(&id, 1, Box::new(snapshot()));
    state.set_pane_surface(surface());
    state.set_endpoint_usage(&id, 1, projection(1, "Z.AI"));
    let frame = state.compose(120, 30).expect("usage column");
    let text = frame
        .cells
        .iter()
        .map(|cell| cell.symbol.as_str())
        .collect::<String>();
    assert!(text.contains("Z.AI"));
    assert!(text.contains("12.5%"));
    assert!(state.hits.usage_column_title.width > 0);
    let title = state.hits.usage_column_title;
    let outcome = state.handle_raw_events(vec![RawInputEvent::Mouse(MouseEvent {
        kind: MouseEventKind::Down(MouseButton::Left),
        column: title.x,
        row: title.y,
        modifiers: KeyModifiers::empty(),
    })]);
    assert!(state.usage_column_collapsed);
    assert!(outcome.resize && outcome.repaint);
}

#[test]
fn malformed_optional_usage_does_not_reject_the_endpoint_or_break_legacy_fallback() {
    use crate::client::endpoint::{decode_endpoint_control, EndpointControlMessage};
    assert!(matches!(
        decode_endpoint_control(crate::protocol::endpoint::USAGE_KIND, "broken").unwrap(),
        EndpointControlMessage::Ignored
    ));
    let mut value = projection(1, "Z.AI");
    value.usage.version = 999;
    assert!(matches!(
        decode_endpoint_control(
            crate::protocol::endpoint::USAGE_KIND,
            &serde_json::to_string(&value).unwrap()
        )
        .unwrap(),
        EndpointControlMessage::Ignored
    ));
    let mut state = ClientShellState::new(ClientShellConfig::from_config(&Config::default()));
    state.usage_column_collapsed = false;
    let mut old = snapshot();
    old.plugin_usage.push(crate::protocol::PluginUsageEntry {
        name: "legacy".into(),
        session_percent: Some(35),
        weekly_percent: None,
        weekly_fable_percent: None,
        reset_credits: None,
        session_resets: None,
        weekly_resets: None,
        updated_at: None,
    });
    state.set_snapshot(Box::new(old));
    state.set_pane_surface(surface());
    let frame = state.compose(120, 30).unwrap();
    assert!(frame
        .cells
        .iter()
        .map(|cell| cell.symbol.as_str())
        .collect::<String>()
        .contains("LEGACY"));
}

#[test]
fn retained_quota_displays_its_original_success_time() {
    let mut state = ClientShellState::new(ClientShellConfig::from_config(&Config::default()));
    state.usage_column_collapsed = false;
    state.set_endpoint_snapshot_for_generation(&ClientEndpointId::Local, 1, Box::new(snapshot()));
    state.set_pane_surface(surface());
    let mut value = projection(1, "Z.AI");
    let usage = value.usage.platforms.get_mut("zai").unwrap();
    usage.updated_at = Some("2020-01-02T03:04:00Z".into());
    usage.error = Some(herdr_usage::UsageError::new(
        herdr_usage::ErrorKind::Network,
        "offline",
    ));
    value.usage.updated_at = Some("2026-11-11T03:04:00Z".into());
    state.set_endpoint_usage(&ClientEndpointId::Local, 1, value);
    let frame = state.compose(120, 30).unwrap();
    let text = frame
        .cells
        .iter()
        .map(|cell| cell.symbol.as_str())
        .collect::<String>();
    assert!(text.contains("Stale: offline"));
    assert!(text.contains("Data from "));
    assert!(text.contains("Checked "));
}

#[test]
fn usage_is_scoped_to_the_selected_endpoint_and_survives_catalog_reload() {
    use crate::client::endpoint::{ClientEndpointStatus, ProfileId, SavedSshEndpoint};
    let mut state = ClientShellState::new(ClientShellConfig::from_config(&Config::default()));
    let profile = SavedSshEndpoint {
        id: ProfileId::parse("0123456789abcdef0123456789abcdef").unwrap(),
        label: "Remote".into(),
        target: "dev@host.example".into(),
        session: "work".into(),
        enabled: true,
    };
    let remote = ClientEndpointId::Ssh(profile.id.clone());
    state.set_endpoint_catalog(std::slice::from_ref(&profile));
    state.set_endpoint_status(&remote, ClientEndpointStatus::Online);
    state.set_endpoint_snapshot_for_generation(&ClientEndpointId::Local, 1, Box::new(snapshot()));
    state.set_endpoint_snapshot_for_generation(&remote, 4, Box::new(snapshot()));
    assert!(state.set_endpoint_usage(&ClientEndpointId::Local, 1, projection(1, "Local quota")));
    assert!(!state.set_endpoint_usage(&remote, 4, projection(1, "Remote quota")));
    fn active_name(state: &ClientShellState) -> &str {
        let endpoint = state
            .endpoints
            .iter()
            .find(|endpoint| endpoint.endpoint_id == state.active_endpoint_id)
            .unwrap();
        &crate::client::shell::usage::snapshot(endpoint)
            .unwrap()
            .0
            .platforms["zai"]
            .display_name
    }
    assert_eq!(active_name(&state), "Local quota");
    assert!(state.activate_endpoint_projection(&remote));
    assert_eq!(active_name(&state), "Remote quota");
    state.set_endpoint_catalog(std::slice::from_ref(&profile));
    assert_eq!(active_name(&state), "Remote quota");
    assert!(state.activate_endpoint_projection(&ClientEndpointId::Local));
    assert_eq!(active_name(&state), "Local quota");
    state.retire_endpoint(&remote);
    let endpoint = state
        .endpoints
        .iter()
        .find(|endpoint| endpoint.endpoint_id == remote)
        .unwrap();
    assert!(endpoint.usage.is_none());
}
