//! Formatting shared by the client column and the standalone popup.
use herdr_usage::{ProviderUsage, Window};

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
}
