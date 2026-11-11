use clap::{Arg, ArgAction, Command};
use std::{
    io,
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

pub(super) fn command() -> Command {
    let path = |name: &'static str, help: &'static str| {
        Arg::new(name).long(name).value_name("PATH").help(help)
    };
    Command::new("usage")
        .about("Read account quotas directly, without opening agent panes")
        .after_help("Provider implementation adapted from CodexBar (MIT). See: herdr usage license")
        .subcommand(
            Command::new("fetch")
                .about("Fetch quotas through provider OAuth/API credentials and print JSON")
                .arg(path("config", "Read a usage.toml configuration"))
                .arg(path("cache", "Use this account-usage.json path"))
                .arg(
                    Arg::new("provider")
                        .long("provider")
                        .value_parser(["all", "codex", "claude", "zai"]),
                )
                .arg(
                    Arg::new("write-cache")
                        .long("write-cache")
                        .action(ArgAction::SetTrue),
                )
                .arg(Arg::new("json").long("json").action(ArgAction::SetTrue)),
        )
        .subcommand(
            Command::new("get")
                .about("Read the running server's cached account quotas")
                .arg(path(
                    "cache",
                    "Read an explicit cache file instead of contacting the server",
                ))
                .arg(Arg::new("json").long("json").action(ArgAction::SetTrue)),
        )
        .subcommand(
            Command::new("dashboard")
                .about("Show a cached account quota dashboard")
                .arg(path("cache", "Read this account-usage.json file"))
                .arg(
                    Arg::new("once")
                        .long("once")
                        .action(ArgAction::SetTrue)
                        .help("Print the dashboard once without entering terminal mode"),
                ),
        )
        .subcommand(Command::new("license").about("Print the CodexBar provider port's MIT notice"))
}

pub(super) fn run(args: &[String]) -> io::Result<i32> {
    let matches = match command()
        .try_get_matches_from(std::iter::once("usage").chain(args.iter().map(String::as_str)))
    {
        Ok(matches) => matches,
        Err(error) => {
            let code = if error.use_stderr() { 2 } else { 0 };
            error.print()?;
            return Ok(code);
        }
    };
    let Some((subcommand, args)) = matches.subcommand() else {
        command().print_long_help()?;
        return Ok(2);
    };
    if subcommand == "license" {
        println!(
            "{}",
            include_str!("../../crates/usage-core/LICENSE-CodexBar")
        );
        return Ok(0);
    }
    let cache = args
        .get_one::<String>("cache")
        .map(PathBuf::from)
        .unwrap_or_else(crate::usage::cache_path);
    match subcommand {
        "fetch" => {
            let path = args
                .get_one::<String>("config")
                .map(PathBuf::from)
                .unwrap_or_else(crate::usage::config_path);
            let mut config = crate::usage::read_config(&path).map_err(io::Error::other)?;
            let original_config = config.clone();
            let _lock = args
                .get_flag("write-cache")
                .then(|| crate::usage::cache_lock(&cache))
                .transpose()?;
            if let Some(provider) = args
                .get_one::<String>("provider")
                .filter(|provider| provider.as_str() != "all")
            {
                config.codex.enabled = provider == "codex";
                config.claude.enabled = provider == "claude";
                config.zai.enabled = provider == "zai";
            }
            let previous = crate::usage::read_cache(&cache).unwrap_or_default();
            let mut snapshot = if config.enabled {
                crate::usage::fetch(&config, &previous).map_err(io::Error::other)?
            } else {
                herdr_usage::UsageSnapshot::default()
            };
            let requested_failed = snapshot
                .platforms
                .values()
                .any(|usage| usage.error.is_some());
            if args.get_flag("write-cache") {
                if config.enabled
                    && args
                        .get_one::<String>("provider")
                        .is_some_and(|provider| provider != "all")
                {
                    let env = crate::usage::environment()?;
                    let owned = herdr_usage::owned_snapshot(&original_config, &env, &previous);
                    for (name, usage) in owned.platforms {
                        snapshot.platforms.entry(name).or_insert(usage);
                    }
                }
                crate::usage::write_cache(&cache, &snapshot)?;
            }
            print_json(&snapshot)?;
            Ok(i32::from(requested_failed))
        }
        "get" => {
            if args.get_one::<String>("cache").is_some() {
                print_json(&crate::usage::read_cache(&cache)?)?;
                return Ok(0);
            }
            super::print_response(&super::send_request(&crate::api::schema::Request {
                id: "cli:usage:get".into(),
                method: crate::api::schema::Method::UsageGet(
                    crate::api::schema::EmptyParams::default(),
                ),
            })?)
        }
        "dashboard" if args.get_flag("once") => {
            let snapshot =
                read_dashboard_snapshot(args.get_one::<String>("cache").map(|_| cache.as_path()))?;
            for line in dashboard_lines(&snapshot, 80) {
                println!("{line}");
            }
            Ok(0)
        }
        "dashboard" => dashboard(args.get_one::<String>("cache").map(|_| cache.as_path())),
        _ => Ok(2),
    }
}

fn print_json(snapshot: &herdr_usage::UsageSnapshot) -> io::Result<()> {
    use io::Write;
    crate::platform::begin_cli_output();
    let mut out = io::stdout().lock();
    serde_json::to_writer_pretty(&mut out, snapshot).map_err(io::Error::other)?;
    writeln!(out)
}

fn dashboard_lines(snapshot: &herdr_usage::UsageSnapshot, width: u16) -> Vec<String> {
    use crate::usage::presentation::{details, local_time, UsageRowLayout};
    let offset = time::UtcOffset::current_local_offset().unwrap_or(time::UtcOffset::UTC);
    let quota_layout = UsageRowLayout::new(width.into(), 2, 24, 40);
    let mut lines = vec![
        "Account Usage".into(),
        "q / Esc closes · ↑ / ↓ scrolls · cached data".into(),
        String::new(),
    ];
    if snapshot.platforms.is_empty() {
        lines.push("No account usage available.".into());
    }
    for usage in snapshot.platforms.values() {
        lines.push(usage.display_name.clone());
        for window in &usage.windows {
            lines.push(quota_layout.format(window));
            if let Some(reset) = &window.resets_at {
                lines.push(format!("    Resets {}", local_time(reset, offset)));
            }
        }
        lines.extend(
            details(usage, offset)
                .into_iter()
                .map(|line| format!("  {line}")),
        );
        lines.push(String::new());
    }
    lines
}

fn read_dashboard_snapshot(path: Option<&Path>) -> io::Result<herdr_usage::UsageSnapshot> {
    if let Some(path) = path {
        return crate::usage::read_cache(path);
    }
    let response = crate::api::client::ApiClient::local()
        .request_value_with_deadline(
            &crate::api::schema::Request {
                id: "cli:usage:dashboard".into(),
                method: crate::api::schema::Method::UsageGet(
                    crate::api::schema::EmptyParams::default(),
                ),
            },
            Duration::from_secs(2),
        )
        .map_err(io::Error::other)?;
    serde_json::from_value(response["result"]["usage"].clone()).map_err(io::Error::other)
}

fn dashboard(path: Option<&Path>) -> io::Result<i32> {
    use crossterm::{
        cursor::{Hide, Show},
        event::{self, Event, KeyCode, KeyEventKind},
        execute,
        terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
    };
    use ratatui::{
        backend::CrosstermBackend,
        widgets::{Paragraph, Wrap},
        Terminal,
    };
    struct Guard;
    impl Drop for Guard {
        fn drop(&mut self) {
            let _ = disable_raw_mode();
            let _ = execute!(io::stdout(), LeaveAlternateScreen, Show);
        }
    }
    enable_raw_mode()?;
    let _guard = Guard;
    execute!(io::stdout(), EnterAlternateScreen, Hide)?;
    let mut terminal = Terminal::new(CrosstermBackend::new(io::stdout()))?;
    let initial = read_dashboard_snapshot(path);
    let mut source_error = initial.is_err();
    let mut snapshot = initial.unwrap_or_default();
    let mut next_refresh = Instant::now() + Duration::from_secs(2);
    let mut scroll = 0u16;
    loop {
        if Instant::now() >= next_refresh {
            match read_dashboard_snapshot(path) {
                Ok(next) => {
                    snapshot = next;
                    source_error = false;
                }
                Err(_) => source_error = true,
            }
            next_refresh = Instant::now() + Duration::from_secs(2);
        }
        terminal.draw(|frame| {
            let mut lines = dashboard_lines(&snapshot, frame.area().width);
            if source_error {
                lines.insert(
                    2,
                    "Usage refresh unavailable; showing the last received snapshot.".into(),
                );
            }
            let paragraph = Paragraph::new(lines.join("\n")).wrap(Wrap { trim: false });
            scroll = scroll.min(scroll_limit(
                &paragraph,
                frame.area().width,
                frame.area().height,
            ));
            frame.render_widget(paragraph.scroll((scroll, 0)), frame.area());
        })?;
        if event::poll(Duration::from_millis(200))? {
            if let Event::Key(key) = event::read()? {
                if key.kind == KeyEventKind::Release {
                    continue;
                }
                match key.code {
                    KeyCode::Char('q') | KeyCode::Esc => break,
                    KeyCode::Char('c') if key.modifiers.contains(event::KeyModifiers::CONTROL) => {
                        break
                    }
                    KeyCode::Down | KeyCode::Char('j') => scroll = scroll.saturating_add(1),
                    KeyCode::Up | KeyCode::Char('k') => scroll = scroll.saturating_sub(1),
                    KeyCode::PageDown => scroll = scroll.saturating_add(10),
                    KeyCode::PageUp => scroll = scroll.saturating_sub(10),
                    _ => {}
                }
            }
        }
    }
    Ok(0)
}

fn scroll_limit(paragraph: &ratatui::widgets::Paragraph<'_>, width: u16, height: u16) -> u16 {
    paragraph
        .line_count(width.max(1))
        .saturating_sub(height.into())
        .min(u16::MAX as usize) as u16
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn popup_bars_align_across_labels_and_percent_widths() {
        use unicode_width::UnicodeWidthStr;
        let mut snapshot = herdr_usage::UsageSnapshot::default();
        for (id, label, value) in [
            (herdr_usage::ProviderId::Codex, "5h", 9.0),
            (herdr_usage::ProviderId::Claude, "Weekly", 12.5),
            (herdr_usage::ProviderId::Zai, "週次 👩‍💻", 100.0),
        ] {
            let mut provider = herdr_usage::ProviderUsage::new(id);
            provider
                .windows
                .push(herdr_usage::Window::new("window", label, value));
            snapshot.platforms.insert(id.as_str().into(), provider);
        }
        let ranges: Vec<_> = dashboard_lines(&snapshot, 80)
            .into_iter()
            .filter_map(|line| {
                let start = line.find(['█', '░'])?;
                let prefix = UnicodeWidthStr::width(&line[..start]);
                let count = line.chars().filter(|ch| matches!(ch, '█' | '░')).count();
                Some((prefix, count))
            })
            .collect();
        assert_eq!(ranges.len(), 3);
        assert!(ranges.windows(2).all(|pair| pair[0] == pair[1]));
    }
    #[test]
    fn narrow_dashboard_can_scroll_to_the_last_provider_after_wrapped_errors() {
        use ratatui::{
            buffer::Buffer,
            layout::Rect,
            widgets::{Paragraph, Widget, Wrap},
        };
        let mut snapshot = herdr_usage::UsageSnapshot::default();
        let mut first = herdr_usage::ProviderUsage::new(herdr_usage::ProviderId::Claude);
        first.error = Some(herdr_usage::UsageError::new(
            herdr_usage::ErrorKind::Network,
            "long network detail ".repeat(30),
        ));
        snapshot.platforms.insert("claude".into(), first);
        let mut last = herdr_usage::ProviderUsage::new(herdr_usage::ProviderId::Zai);
        last.display_name = "LAST PROVIDER".into();
        snapshot.platforms.insert("zai".into(), last);
        let lines = dashboard_lines(&snapshot, 20);
        let paragraph = Paragraph::new(lines.join("\n")).wrap(Wrap { trim: false });
        assert!(paragraph.line_count(20) > lines.len());
        let area = Rect::new(0, 0, 20, 8);
        let mut buffer = Buffer::empty(area);
        let scroll = scroll_limit(&paragraph, area.width, area.height);
        paragraph.scroll((scroll, 0)).render(area, &mut buffer);
        let rendered = buffer
            .content
            .iter()
            .map(|cell| cell.symbol())
            .collect::<String>();
        assert!(rendered.contains("LAST PROVIDER"));
    }
}
