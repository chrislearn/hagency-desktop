//! Locale-aware number, time and quantity formatting. Pure functions; no
//! Makepad, clock or environment access. Unknown values are formatted by the
//! caller (as "Unknown"/"Not reported"), never as zero.
use crate::i18n::{Locale, Msg, tr};
use hagency_metering::dashboard::period::civil_from_days;

const MINUTE_MS: u64 = 60_000;
const HOUR_MS: u64 = 60 * MINUTE_MS;
const DAY_MS: u64 = 24 * HOUR_MS;

/// Wall clock and local UTC offset used for all relative/absolute times.
/// Injected so formatting and staleness are deterministic in tests.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Clock {
    pub now_ms: u64,
    pub utc_offset_minutes: i32,
}

impl Clock {
    pub fn fixed(now_ms: u64, utc_offset_minutes: i32) -> Self {
        Self { now_ms, utc_offset_minutes }
    }

    /// The system clock and the current local offset (DST aware).
    pub fn system() -> Self {
        let now_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0);
        Self { now_ms, utc_offset_minutes: local_offset_minutes(now_ms) }
    }
}

#[cfg(unix)]
fn local_offset_minutes(now_ms: u64) -> i32 {
    let seconds = (now_ms / 1000) as libc::time_t;
    // SAFETY: localtime_r writes into the provided zeroed struct only.
    unsafe {
        let mut tm: libc::tm = std::mem::zeroed();
        if libc::localtime_r(&seconds, &mut tm).is_null() {
            return 0;
        }
        (tm.tm_gmtoff / 60) as i32
    }
}
#[cfg(not(unix))]
fn local_offset_minutes(_now_ms: u64) -> i32 {
    0
}

/// `1234567` → `1,234,567`. Used for aligned numeric columns.
pub fn grouped(n: u64) -> String {
    let digits = n.to_string();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3);
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i) % 3 == 0 {
            out.push(',');
        }
        out.push(c);
    }
    out
}

/// Headline token counts: `12,345`, `123.5K`/`12.3万`, `1.23M`, `4.5亿`.
pub fn compact(n: u64, locale: Locale) -> String {
    let units: &[(u64, &str)] = match locale {
        Locale::En => &[(1_000_000_000_000, "T"), (1_000_000_000, "B"), (1_000_000, "M"), (1_000, "K")],
        Locale::ZhHans => &[(100_000_000, "亿"), (10_000, "万")],
    };
    let threshold = match locale {
        Locale::En => 100_000,
        Locale::ZhHans => 100_000,
    };
    if n < threshold {
        return grouped(n);
    }
    for &(unit, suffix) in units {
        if n >= unit {
            let value = n as f64 / unit as f64;
            let text = if value >= 100.0 {
                format!("{}", value.round() as u64)
            } else if value >= 10.0 {
                trim_zero(format!("{value:.1}"))
            } else {
                trim_zero(format!("{value:.2}"))
            };
            return format!("{text}{suffix}");
        }
    }
    grouped(n)
}

fn trim_zero(text: String) -> String {
    if text.contains('.') {
        text.trim_end_matches('0').trim_end_matches('.').to_owned()
    } else {
        text
    }
}

/// Provider-reported utilization. Values above 100 are shown as reported.
pub fn percent(value: f64) -> String {
    if !value.is_finite() {
        return "—".into();
    }
    if value > 0.0 && value < 1.0 {
        return "<1%".into();
    }
    format!("{}%", value.round() as i64)
}

/// Fraction for a meter: clamped 0..=1. `None` stays unknown.
pub fn meter_fraction(used_percent: Option<f64>) -> Option<f32> {
    used_percent.filter(|v| v.is_finite()).map(|v| (v / 100.0).clamp(0.0, 1.0) as f32)
}

/// Remaining time: `2h 14m`, `3d 4h`, `45m`, `<1m` / `2小时14分`, `3天4小时`.
pub fn duration(ms: u64, locale: Locale) -> String {
    let days = ms / DAY_MS;
    let hours = (ms % DAY_MS) / HOUR_MS;
    let minutes = (ms % HOUR_MS) / MINUTE_MS;
    match locale {
        Locale::En => match (days, hours, minutes) {
            (0, 0, 0) => "<1m".into(),
            (0, 0, m) => format!("{m}m"),
            (0, h, 0) => format!("{h}h"),
            (0, h, m) => format!("{h}h {m}m"),
            (d, 0, _) => format!("{d}d"),
            (d, h, _) => format!("{d}d {h}h"),
        },
        Locale::ZhHans => match (days, hours, minutes) {
            (0, 0, 0) => "不到1分钟".into(),
            (0, 0, m) => format!("{m}分钟"),
            (0, h, 0) => format!("{h}小时"),
            (0, h, m) => format!("{h}小时{m}分"),
            (d, 0, _) => format!("{d}天"),
            (d, h, _) => format!("{d}天{h}小时"),
        },
    }
}

/// Age of an observation: `just now`, `5 min ago`, `2 h ago`, `3 days ago`.
/// A timestamp in the future (clock skew) reads as just now.
pub fn age(clock: Clock, at_ms: u64, locale: Locale) -> String {
    let elapsed = clock.now_ms.saturating_sub(at_ms);
    if elapsed < MINUTE_MS {
        return tr(locale, Msg::JustNow).into();
    }
    let (n, unit_en, unit_zh) = if elapsed < HOUR_MS {
        (elapsed / MINUTE_MS, "min", "分钟")
    } else if elapsed < DAY_MS {
        (elapsed / HOUR_MS, "h", "小时")
    } else {
        (elapsed / DAY_MS, if elapsed / DAY_MS == 1 { "day" } else { "days" }, "天")
    };
    match locale {
        Locale::En => format!("{n} {unit_en} ago"),
        Locale::ZhHans => format!("{n}{unit_zh}前"),
    }
}

/// `UTC+8`, `UTC−3:30`, `UTC`.
pub fn utc_offset(minutes: i32) -> String {
    if minutes == 0 {
        return "UTC".into();
    }
    let sign = if minutes > 0 { '+' } else { '−' };
    let abs = minutes.unsigned_abs();
    if abs % 60 == 0 {
        format!("UTC{sign}{}", abs / 60)
    } else {
        format!("UTC{sign}{}:{:02}", abs / 60, abs % 60)
    }
}

/// Local wall time with an explicit timezone: `Today 15:30 (UTC+8)`,
/// `Tomorrow 09:00 (UTC+8)`, `Nov 3, 09:00 (UTC+8)` / `11月3日 09:00（UTC+8）`.
pub fn local_time(clock: Clock, at_ms: u64, locale: Locale) -> String {
    let offset_ms = i64::from(clock.utc_offset_minutes) * MINUTE_MS as i64;
    let local = at_ms as i64 + offset_ms;
    let day = local.div_euclid(DAY_MS as i64);
    let in_day = local.rem_euclid(DAY_MS as i64) as u64;
    let today = (clock.now_ms as i64 + offset_ms).div_euclid(DAY_MS as i64);
    let hm = format!("{:02}:{:02}", in_day / HOUR_MS, (in_day % HOUR_MS) / MINUTE_MS);
    let zone = utc_offset(clock.utc_offset_minutes);
    let date = if day == today {
        tr(locale, Msg::Today).to_owned()
    } else if day == today + 1 {
        tr(locale, Msg::Tomorrow).to_owned()
    } else {
        let (year, month, d) = civil_from_days(day);
        let (now_year, _, _) = civil_from_days(today);
        match locale {
            Locale::En => {
                const MONTHS: [&str; 12] =
                    ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];
                let m = MONTHS[(month as usize).saturating_sub(1).min(11)];
                if year == now_year { format!("{m} {d},") } else { format!("{m} {d}, {year},") }
            }
            Locale::ZhHans => {
                if year == now_year { format!("{month}月{d}日") } else { format!("{year}年{month}月{d}日") }
            }
        }
    };
    match locale {
        Locale::En => format!("{date} {hm} ({zone})"),
        Locale::ZhHans => format!("{date} {hm}（{zone}）"),
    }
}

/// Provider-reported window length: `5-hour window`, `7-day window`.
pub fn window_length(minutes: u64, locale: Locale) -> String {
    let (n, en, zh) = if minutes >= 1440 && minutes % 1440 == 0 {
        (minutes / 1440, "day", "天")
    } else if minutes >= 60 && minutes % 60 == 0 {
        (minutes / 60, "hour", "小时")
    } else {
        (minutes, "minute", "分钟")
    };
    match locale {
        Locale::En => format!("{n}-{en} window"),
        Locale::ZhHans => format!("{n}{zh}窗口"),
    }
}

/// `$1,234.56`; sub-cent positive values read `<$0.01`.
pub fn usd(value: f64) -> String {
    if !value.is_finite() || value < 0.0 {
        return "—".into();
    }
    if value > 0.0 && value < 0.01 {
        return "<$0.01".into();
    }
    let cents = (value * 100.0).round() as u64;
    format!("${}.{:02}", grouped(cents / 100), cents % 100)
}

/// A token quantity that may be unknown.
pub fn tokens_or_unknown(value: Option<u64>, locale: Locale) -> String {
    match value {
        Some(n) => grouped(n),
        None => tr(locale, Msg::Unknown).into(),
    }
}

/// Bounded single-line text for daemon-provided strings shown in the UI.
pub fn bounded(text: &str, max_chars: usize) -> String {
    let clean: String = text.chars().map(|c| if c.is_control() { ' ' } else { c }).collect();
    let clean = clean.trim();
    if clean.chars().count() <= max_chars {
        clean.to_owned()
    } else {
        let mut out: String = clean.chars().take(max_chars.saturating_sub(1)).collect();
        out.push('…');
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    const NOV_1_2026_0730_UTC: u64 = 1_793_518_200_000;

    #[test]
    fn grouped_and_compact_counts() {
        assert_eq!(grouped(0), "0");
        assert_eq!(grouped(999), "999");
        assert_eq!(grouped(1_000), "1,000");
        assert_eq!(grouped(1_234_567), "1,234,567");
        assert_eq!(grouped(u64::MAX), "18,446,744,073,709,551,615");
        assert_eq!(compact(99_999, Locale::En), "99,999");
        assert_eq!(compact(123_456, Locale::En), "123K");
        assert_eq!(compact(1_234_567, Locale::En), "1.23M");
        assert_eq!(compact(12_000_000, Locale::En), "12M");
        assert_eq!(compact(2_500_000_000, Locale::En), "2.5B");
        assert_eq!(compact(123_456, Locale::ZhHans), "12.3万");
        assert_eq!(compact(1_234_567, Locale::ZhHans), "123万");
        assert_eq!(compact(450_000_000, Locale::ZhHans), "4.5亿");
        assert!(!compact(u64::MAX, Locale::En).is_empty());
    }

    #[test]
    fn percent_and_meter_never_invent_values() {
        assert_eq!(percent(0.0), "0%");
        assert_eq!(percent(0.4), "<1%");
        assert_eq!(percent(61.6), "62%");
        assert_eq!(percent(104.0), "104%");
        assert_eq!(percent(f64::NAN), "—");
        assert_eq!(meter_fraction(None), None);
        assert_eq!(meter_fraction(Some(f64::NAN)), None);
        assert_eq!(meter_fraction(Some(150.0)), Some(1.0));
        assert_eq!(meter_fraction(Some(-5.0)), Some(0.0));
        assert_eq!(meter_fraction(Some(25.0)), Some(0.25));
    }

    #[test]
    fn durations_in_both_locales() {
        assert_eq!(duration(0, Locale::En), "<1m");
        assert_eq!(duration(45 * MINUTE_MS, Locale::En), "45m");
        assert_eq!(duration(2 * HOUR_MS + 14 * MINUTE_MS, Locale::En), "2h 14m");
        assert_eq!(duration(3 * HOUR_MS, Locale::En), "3h");
        assert_eq!(duration(3 * DAY_MS + 4 * HOUR_MS + 5 * MINUTE_MS, Locale::En), "3d 4h");
        assert_eq!(duration(2 * HOUR_MS + 14 * MINUTE_MS, Locale::ZhHans), "2小时14分");
        assert_eq!(duration(7 * DAY_MS, Locale::ZhHans), "7天");
        assert_eq!(duration(30_000, Locale::ZhHans), "不到1分钟");
    }

    #[test]
    fn ages_handle_skew_and_units() {
        let clock = Clock::fixed(10 * DAY_MS, 0);
        assert_eq!(age(clock, clock.now_ms + 5_000, Locale::En), "just now");
        assert_eq!(age(clock, clock.now_ms - 5 * MINUTE_MS, Locale::En), "5 min ago");
        assert_eq!(age(clock, clock.now_ms - 2 * HOUR_MS, Locale::En), "2 h ago");
        assert_eq!(age(clock, clock.now_ms - DAY_MS, Locale::En), "1 day ago");
        assert_eq!(age(clock, clock.now_ms - 3 * DAY_MS, Locale::En), "3 days ago");
        assert_eq!(age(clock, clock.now_ms - 5 * MINUTE_MS, Locale::ZhHans), "5分钟前");
    }

    #[test]
    fn local_time_states_the_timezone() {
        let clock = Clock::fixed(NOV_1_2026_0730_UTC, 8 * 60);
        assert_eq!(local_time(clock, NOV_1_2026_0730_UTC, Locale::En), "Today 15:30 (UTC+8)");
        assert_eq!(
            local_time(clock, NOV_1_2026_0730_UTC + 18 * HOUR_MS, Locale::En),
            "Tomorrow 09:30 (UTC+8)"
        );
        assert_eq!(
            local_time(clock, NOV_1_2026_0730_UTC + 3 * DAY_MS, Locale::En),
            "Nov 4, 15:30 (UTC+8)"
        );
        assert_eq!(
            local_time(clock, NOV_1_2026_0730_UTC + 3 * DAY_MS, Locale::ZhHans),
            "11月4日 15:30（UTC+8）"
        );
        assert_eq!(
            local_time(clock, NOV_1_2026_0730_UTC + 70 * DAY_MS, Locale::En),
            "Jan 10, 2027, 15:30 (UTC+8)"
        );
        let west = Clock::fixed(NOV_1_2026_0730_UTC, -(3 * 60 + 30));
        assert_eq!(local_time(west, NOV_1_2026_0730_UTC, Locale::En), "Today 04:00 (UTC−3:30)");
        assert_eq!(utc_offset(0), "UTC");
    }

    #[test]
    fn window_lengths_use_reported_minutes_only() {
        assert_eq!(window_length(300, Locale::En), "5-hour window");
        assert_eq!(window_length(10_080, Locale::En), "7-day window");
        assert_eq!(window_length(90, Locale::En), "90-minute window");
        assert_eq!(window_length(300, Locale::ZhHans), "5小时窗口");
    }

    #[test]
    fn usd_and_unknown_tokens() {
        assert_eq!(usd(1234.567), "$1,234.57");
        assert_eq!(usd(0.0), "$0.00");
        assert_eq!(usd(0.004), "<$0.01");
        assert_eq!(usd(-1.0), "—");
        assert_eq!(tokens_or_unknown(None, Locale::En), "Unknown");
        assert_eq!(tokens_or_unknown(Some(0), Locale::En), "0");
    }

    #[test]
    fn bounded_strips_control_characters() {
        assert_eq!(bounded("a\nb\tc", 10), "a b c");
        assert_eq!(bounded("abcdefghijk", 5), "abcd…");
    }
}
