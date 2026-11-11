//! Formatting shared by the client column and the standalone popup.
use herdr_usage::{ProviderUsage, Window};
use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;

/// One layout per surface, shared by every provider and quota window.
pub(crate) struct UsageRowLayout {
    indent: usize,
    label: usize,
    label_gap: usize,
    bar: usize,
    percent_gap: usize,
    percent: usize,
}

impl UsageRowLayout {
    pub(crate) fn new(width: usize, indent: usize, label: usize, max_bar: usize) -> Self {
        let indent = indent.min(width);
        let remaining = width.saturating_sub(indent);
        let percent = remaining.min(6);
        let left = remaining.saturating_sub(percent);
        let percent_gap = usize::from(left > 0 && percent > 0);
        let left = left.saturating_sub(percent_gap);
        let label = label.min(left.saturating_sub(2));
        let label_gap = usize::from(label > 0);
        let bar = left.saturating_sub(label + label_gap).min(max_bar).min(120);
        Self {
            indent,
            label,
            label_gap,
            bar,
            percent_gap,
            percent,
        }
    }

    pub(crate) fn format(&self, window: &Window) -> String {
        let label = clipped_text(&window.label, self.label);
        let mut value = percent(window.used_percent);
        if value.len() > 6 {
            value = format!("{:.0e}%", window.used_percent);
            if value.len() > 6 {
                value = if window.used_percent.is_sign_negative() {
                    "<-99%"
                } else {
                    ">999%"
                }
                .into();
            }
        }
        let value = clipped_text(&value, self.percent);
        format!(
            "{}{}{}{}{}{}{}{}",
            " ".repeat(self.indent),
            label,
            " ".repeat(
                self.label
                    .saturating_sub(UnicodeWidthStr::width(label.as_str()))
            ),
            " ".repeat(self.label_gap),
            bar(window, self.bar),
            " ".repeat(self.percent_gap),
            " ".repeat(self.percent.saturating_sub(value.len())),
            value,
        )
    }
}

fn clipped_text(text: &str, cells: usize) -> String {
    let mut used = 0;
    text.graphemes(true)
        .filter(|grapheme| !grapheme.chars().any(char::is_control))
        .take_while(|grapheme| {
            used += UnicodeWidthStr::width(*grapheme);
            used <= cells
        })
        .collect()
}

pub(crate) fn local_time(iso: &str, offset: time::UtcOffset) -> String {
    time::OffsetDateTime::parse(iso, &time::format_description::well_known::Rfc3339)
        .ok()
        .map(|value| {
            let value = value.to_offset(offset);
            format!(
                "{:02}/{:02} {:02}:{:02}",
                value.month() as u8,
                value.day(),
                value.hour(),
                value.minute()
            )
        })
        .unwrap_or_else(|| iso.chars().take(24).collect())
}

pub(crate) fn percent(value: f64) -> String {
    if (value - value.round()).abs() < 0.05 {
        format!("{value:.0}%")
    } else {
        format!("{value:.1}%")
    }
}

pub(crate) fn bar(window: &Window, width: usize) -> String {
    let width = width.min(120);
    let filled = ((window.used_percent.clamp(0.0, 100.0) / 100.0) * width as f64).round() as usize;
    format!(
        "{}{}",
        "█".repeat(filled),
        "░".repeat(width.saturating_sub(filled))
    )
}

pub(crate) fn details(usage: &ProviderUsage, offset: time::UtcOffset) -> Vec<String> {
    let mut lines = Vec::new();
    if let Some(account) = &usage.account {
        lines.push(account.clone());
    }
    if let Some(plan) = &usage.plan {
        lines.push(format!("{plan} · {}", usage.source));
    } else if !usage.source.is_empty() {
        lines.push(usage.source.clone());
    }
    if let Some(credits) = &usage.credits {
        if credits.unlimited {
            lines.push("Credits: unlimited".into());
        } else if let Some(balance) = credits.balance {
            lines.push(format!(
                "Credits: {balance:.2} {}",
                credits.currency.as_deref().unwrap_or("")
            ));
        }
    }
    for allowance in &usage.allowances {
        let used = allowance
            .used
            .map(|value| format!("{value:.2}"))
            .unwrap_or_else(|| "—".into());
        let limit = allowance
            .limit
            .map(|value| format!(" / {value:.2}"))
            .unwrap_or_default();
        lines.push(format!(
            "{}: {used}{limit} {}",
            allowance.label, allowance.currency
        ));
    }
    if let Some(credits) = usage.reset_credits {
        lines.push(format!("Reset credits: {credits}"));
    }
    if let Some(error) = &usage.error {
        lines.push(format!(
            "{}{}",
            if usage.has_data() { "Stale: " } else { "" },
            error.message
        ));
    }
    if usage.partial {
        lines.push("Some provider limits unavailable".into());
    }
    if let Some(updated) = &usage.updated_at {
        lines.push(format!("Updated {}", local_time(updated, offset)));
    }
    lines
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn reset_dates_handle_half_and_quarter_hour_offsets_with_day_carry() {
        let offset = time::UtcOffset::from_hms(5, 45, 0).unwrap();
        assert_eq!(local_time("2027-01-01T23:30:00Z", offset), "01/02 05:15");
    }
    #[test]
    fn untrusted_percentages_never_allocate_bars_wider_than_the_panel() {
        let window = Window::new("test", "test", f64::MAX);
        assert_eq!(bar(&window, 20).chars().count(), 20);
        assert_eq!(bar(&window, usize::MAX).chars().count(), 120);
    }

    #[test]
    fn quota_rows_align_display_cells_for_unicode_labels_and_fractional_values() {
        let layout = UsageRowLayout::new(28, 1, 8, 120);
        let mut positions = Vec::new();
        for (label, value) in [
            ("5h", 0.0),
            ("Weekly", 9.0),
            ("週次", 12.5),
            ("e\u{301}", 100.0),
            ("👩‍💻 quota", 125.5),
        ] {
            let row = layout.format(&Window::new("test", label, value));
            let start = row.find(['█', '░']).unwrap();
            let cells = UnicodeWidthStr::width(&row[..start]);
            let length = row.chars().filter(|c| matches!(c, '█' | '░')).count();
            positions.push((cells, length));
            assert_eq!(UnicodeWidthStr::width(row.as_str()), 28);
        }
        assert!(positions.windows(2).all(|pair| pair[0] == pair[1]));
    }

    #[test]
    fn quota_rows_fit_narrow_surfaces_and_extreme_values() {
        for width in 0..=80 {
            let layout = UsageRowLayout::new(width, 2, 24, 40);
            for value in [0.0, 12.5, 100.0, 125.5, f64::MAX, -f64::MAX] {
                let row = layout.format(&Window::new("test", "長い 👩‍💻 label", value));
                assert!(
                    UnicodeWidthStr::width(row.as_str()) <= width,
                    "{width}: {row:?}"
                );
            }
        }
    }
}
