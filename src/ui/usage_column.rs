use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, Paragraph};

use crate::app::state::UsageColumnEntry;
use crate::app::AppState;

fn utc_to_local_display(iso: &str) -> String {
    let trimmed = iso.trim_end_matches('Z');
    let without_frac = trimmed.split('.').next().unwrap_or(trimmed);
    let time_part = without_frac.split('T').nth(1).unwrap_or(without_frac);
    let parts: Vec<&str> = time_part.split(':').collect();
    if parts.len() < 2 {
        return time_part.to_string();
    }
    let utc_h: i32 = parts[0].parse().unwrap_or(0);
    let utc_m: i32 = parts[1].parse().unwrap_or(0);
    let offset = time::UtcOffset::current_local_offset().unwrap_or(time::UtcOffset::UTC);
    let h = ((utc_h + offset.whole_hours() as i32) % 24 + 24) % 24;
    let m = ((utc_m + (offset.minutes_past_hour() as i32)) % 60 + 60) % 60;
    format!("{h:02}:{m:02}")
}

pub fn load_usage_data() -> Vec<UsageColumnEntry> {
    let path = crate::plugin_paths::plugin_state_dir("usage-monitor").join("account-usage.json");
    let content = match std::fs::read_to_string(&path) {
        Ok(c) => c,
        Err(_) => return Vec::new(),
    };
    let json: serde_json::Value = match serde_json::from_str(&content) {
        Ok(v) => v,
        Err(_) => return Vec::new(),
    };
    let platforms = match json.get("platforms").and_then(|p| p.as_object()) {
        Some(p) => p,
        None => return Vec::new(),
    };

    platforms
        .iter()
        .map(|(name, v)| UsageColumnEntry {
            name: name.clone(),
            session_percent: v
                .get("session_percent")
                .and_then(|n| n.as_u64())
                .map(|n| n as u32),
            weekly_percent: v
                .get("weekly_percent")
                .and_then(|n| n.as_u64())
                .map(|n| n as u32),
            weekly_fable_percent: v
                .get("weekly_fable_percent")
                .and_then(|n| n.as_u64())
                .map(|n| n as u32),
            reset_credits: v
                .get("reset_credits")
                .and_then(|n| n.as_u64())
                .map(|n| n as u32),
            session_resets: v
                .get("session_resets")
                .and_then(|s| s.as_str())
                .map(String::from),
            weekly_resets: v
                .get("weekly_resets")
                .and_then(|s| s.as_str())
                .map(String::from),
            updated_at: v
                .get("updated_at")
                .and_then(|s| s.as_str())
                .map(String::from),
        })
        .collect()
}

fn percent_color(pct: u32) -> Color {
    if pct >= 80 {
        Color::Red
    } else if pct >= 50 {
        Color::Yellow
    } else {
        Color::Green
    }
}

fn bar(pct: u32, width: u16) -> String {
    let filled = ((pct as f32 / 100.0) * width as f32).round() as usize;
    let empty = (width as usize).saturating_sub(filled);
    format!("{}{}", "█".repeat(filled), "░".repeat(empty))
}

pub fn render_usage_column(app: &AppState, frame: &mut Frame, area: Rect) {
    if area.width == 0 || area.height == 0 {
        return;
    }

    let block = Block::default()
        .borders(Borders::LEFT)
        .border_style(Style::default().fg(Color::DarkGray))
        .title(" Usage ")
        .title_style(Style::default().fg(Color::Cyan));

    let inner = block.inner(area);
    frame.render_widget(block, area);

    let entries = &app.usage_column_data;
    let mut lines: Vec<Line> = Vec::new();

    if entries.is_empty() {
        lines.push(Line::from(Span::styled(
            "No data",
            Style::default().fg(Color::DarkGray),
        )));
    } else {
        let bar_width = inner.width.saturating_sub(8);
        for entry in entries {
            lines.push(Line::from(Span::styled(
                entry.name.to_uppercase(),
                Style::default()
                    .fg(Color::White)
                    .add_modifier(Modifier::BOLD),
            )));

            if let Some(ref resets) = entry.session_resets {
                lines.push(Line::from(Span::styled(
                    format!(" resets {resets}"),
                    Style::default().fg(Color::DarkGray),
                )));
            }

            if let Some(pct) = entry.session_percent {
                let color = percent_color(pct);
                lines.push(Line::from(vec![
                    Span::styled(" 5h ", Style::default().fg(Color::DarkGray)),
                    Span::styled(bar(pct, bar_width), Style::default().fg(color)),
                    Span::styled(format!(" {pct}%"), Style::default().fg(color)),
                ]));
            }

            if let Some(pct) = entry.weekly_percent {
                let color = percent_color(pct);
                lines.push(Line::from(vec![
                    Span::styled(" wk ", Style::default().fg(Color::DarkGray)),
                    Span::styled(bar(pct, bar_width), Style::default().fg(color)),
                    Span::styled(format!(" {pct}%"), Style::default().fg(color)),
                ]));
            }

            if let Some(pct) = entry.weekly_fable_percent {
                if pct > 0 {
                    let color = percent_color(pct);
                    lines.push(Line::from(vec![
                        Span::styled(" fb ", Style::default().fg(Color::DarkGray)),
                        Span::styled(bar(pct, bar_width), Style::default().fg(color)),
                        Span::styled(format!(" {pct}%"), Style::default().fg(color)),
                    ]));
                }
            }

            if let Some(credits) = entry.reset_credits {
                lines.push(Line::from(vec![
                    Span::styled(" rs ", Style::default().fg(Color::DarkGray)),
                    Span::styled(
                        format!("{credits} resets"),
                        Style::default().fg(if credits > 0 {
                            Color::Cyan
                        } else {
                            Color::DarkGray
                        }),
                    ),
                ]));
            }

            if let Some(ref ts) = entry.updated_at {
                let display = utc_to_local_display(ts);
                lines.push(Line::from(Span::styled(
                    format!(" Updated: {display}"),
                    Style::default().fg(Color::DarkGray),
                )));
            }

            lines.push(Line::from(""));
        }
    }

    let paragraph = Paragraph::new(lines);
    frame.render_widget(paragraph, inner);
}
