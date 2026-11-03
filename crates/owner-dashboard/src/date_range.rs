//! Inclusive calendar dates, explicitly labelled UTC in the custom-range UI.
use chrono::NaiveDate;
use crate::dto::period::UsagePeriod;

pub fn parse(start: &str, end: &str) -> Option<UsagePeriod> {
    fn date(text: &str) -> Option<NaiveDate> {
        if text.len() != 10 || text.as_bytes().get(4) != Some(&b'-') || text.as_bytes().get(7) != Some(&b'-') { return None; }
        NaiveDate::parse_from_str(text, "%Y-%m-%d").ok()
    }
    let start = date(start.trim())?;
    let end = date(end.trim())?.succ_opt()?;
    let days = end.signed_duration_since(start).num_days();
    if !(1..=366).contains(&days) { return None; }
    let ms = |date: NaiveDate| u64::try_from(date.and_hms_opt(0, 0, 0)?.and_utc().timestamp_millis()).ok();
    Some(UsagePeriod::Range { start_ms: ms(start)?, end_ms: ms(end)? })
}

pub fn label(ms: u64) -> String {
    i64::try_from(ms).ok().and_then(chrono::DateTime::from_timestamp_millis)
        .map(|date| date.format("%Y-%m-%d").to_string()).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn inclusive_dates_validate_leap_days_order_and_bounded_series() {
        assert!(parse("2024-02-29", "2024-02-29").is_some());
        for (a,b) in [("2025-02-29","2025-03-01"),("2026-11-02","2026-11-01"),("2020-01-01","2026-11-01"),("1969-12-31","1970-01-01"),("2026-1-1","2026-01-02")] {
            assert!(parse(a,b).is_none(), "{a} {b}");
        }
        let Some(UsagePeriod::Range {start_ms,end_ms}) = parse("2026-11-01","2026-11-01") else { panic!() };
        assert_eq!(end_ms-start_ms,86_400_000);
        assert_eq!(label(start_ms),"2026-11-01");
    }
}
