use chrono::{DateTime, Utc};

pub(crate) fn seconds(value: &str, now: DateTime<Utc>) -> Option<u64> {
    let value = value.trim();
    if !value.is_empty() && value.bytes().all(|byte| byte.is_ascii_digit()) {
        return value.parse().ok();
    }
    let date = DateTime::parse_from_rfc2822(value).ok()?;
    let millis = date.signed_duration_since(now).num_milliseconds().max(0);
    u64::try_from(millis).ok().map(|value| value.div_ceil(1000))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_delta_seconds_and_http_date_without_guessing_malformed_values() {
        let now = DateTime::parse_from_rfc2822("Sun, 06 Sep 2026 12:00:00 GMT")
            .unwrap()
            .to_utc();
        assert_eq!(seconds(" 12 ", now), Some(12));
        assert_eq!(seconds("Sun, 06 Sep 2026 12:00:09 GMT", now), Some(9));
        assert_eq!(seconds("Sun, 06 Sep 2026 11:59:00 GMT", now), Some(0));
        for value in ["", "-1", "1.5", "secret-url", "18446744073709551616"] {
            assert_eq!(seconds(value, now), None);
        }
    }
}
