use super::*;

#[tokio::test]
async fn quiet_usage_changes_reach_inactive_clients_without_a_pane_or_snapshot_change() {
    let mut server = test_headless_server();
    let sample = |percent| {
        let mut snapshot = herdr_usage::UsageSnapshot::default();
        let mut provider = herdr_usage::ProviderUsage::new(herdr_usage::ProviderId::Codex);
        provider
            .windows
            .push(herdr_usage::Window::new("session", "5h", percent));
        provider.update_legacy_fields();
        snapshot.platforms.insert("codex".into(), provider);
        Arc::new(snapshot)
    };
    server.app.state.usage = sample(17.1);
    server.app.state.usage_revision = 1;
    let mut controls = Vec::new();
    for client_id in [41, 42] {
        let (writer, control, render) = test_client_writer();
        assert!(
            server.handle_server_event(ServerEvent::ClientShellConnected {
                surface_reuse: false,
                surface_delta: false,
                surface_scroll: false,
                client_id,
                surface_cols: 80,
                surface_rows: 23,
                cell_width_px: 0,
                cell_height_px: 0,
                pixel_mouse: false,
                direct_graphics: false,
                endpoint_keybindings: false,
                mouse_capture: false,
                surface_active: false,
                writer,
            })
        );
        let initial = client_shell_snapshot(&control);
        assert_eq!(initial.plugin_usage[0].session_percent, Some(17));
        assert_eq!(server.clients[&client_id].shell_usage_revision, Some(1));
        assert!(render.try_recv().is_err());
        controls.push((client_id, initial.revision, control, render));
    }
    // Both fractions round to the same legacy value, so the shell snapshot is unchanged.
    server.app.state.usage = sample(17.2);
    server.app.state.usage_revision = 2;
    server.app.render_dirty.request_generic();
    server.render_and_stream();
    for (client_id, revision, control, render) in &controls {
        let ServerMessage::EndpointControl { kind, data } =
            read_server_message(control.recv_timeout(Duration::from_secs(1)).unwrap())
        else {
            panic!("expected account usage control");
        };
        assert_eq!(kind, protocol::endpoint::USAGE_KIND);
        let update: protocol::endpoint::EndpointUsageProjection =
            serde_json::from_str(&data).unwrap();
        assert_eq!(update.boot_id, server.client_shell_boot_id);
        assert_eq!(update.revision, 2);
        assert_eq!(
            update.usage.platforms["codex"].windows[0].used_percent,
            17.2
        );
        assert_eq!(
            server.clients[client_id].shell_projection_revision,
            *revision
        );
        assert_eq!(server.clients[client_id].shell_usage_revision, Some(2));
        assert!(control.try_recv().is_err());
        assert!(render.try_recv().is_err());
    }
    server.render_and_stream();
    for (_, _, control, render) in controls {
        assert!(control.try_recv().is_err());
        assert!(render.try_recv().is_err());
    }
}
