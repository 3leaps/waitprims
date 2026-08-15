//! Fail-closed RFC3339 profile used by `contract: agent-wait/v0`.
//!
//! Accepted: extended date/time with seconds and `Z`/`z` or a
//! colon-separated numeric offset. Leap seconds are rejected, not clamped.
//! Equivalent offsets compare as the same instant.

use std::cmp::Ordering;

use time::{Date, Month, OffsetDateTime, PrimitiveDateTime, Time, UtcOffset};

use crate::error::{NormativeReason, ValidationError};

/// Parse a fail-closed RFC3339 timestamp to a UTC instant.
pub fn parse(text: &str) -> Result<OffsetDateTime, ValidationError> {
    parse_inner(text).map_err(|_| {
        ValidationError::normative(
            "/timestamp",
            "rfc3339_profile",
            NormativeReason::UnparseableTimestamp,
        )
    })
}

/// Compare two timestamps after normalizing to UTC instants.
///
/// Returns -1, 0, or 1.
pub fn compare(left: &str, right: &str) -> Result<i8, ValidationError> {
    let a = parse(left)?;
    let b = parse(right)?;
    Ok(match a.cmp(&b) {
        Ordering::Less => -1,
        Ordering::Equal => 0,
        Ordering::Greater => 1,
    })
}

fn parse_inner(text: &str) -> Result<OffsetDateTime, ()> {
    if text.is_empty() || text != text.trim() {
        return Err(());
    }

    let bytes = text.as_bytes();
    // YYYY-MM-DDTHH:MM:SS + timezone, minimum 20 chars (…SSZ).
    if bytes.len() < 20 {
        return Err(());
    }

    let year = parse_n_digits(&bytes[0..4], 4)?;
    expect(bytes, 4, b'-')?;
    let month = parse_n_digits(&bytes[5..7], 2)?;
    expect(bytes, 7, b'-')?;
    let day = parse_n_digits(&bytes[8..10], 2)?;
    if bytes[10] != b'T' && bytes[10] != b't' {
        return Err(());
    }
    let hour = parse_n_digits(&bytes[11..13], 2)?;
    expect(bytes, 13, b':')?;
    let minute = parse_n_digits(&bytes[14..16], 2)?;
    expect(bytes, 16, b':')?;
    let second = parse_n_digits(&bytes[17..19], 2)?;

    if hour > 23 || minute > 59 {
        return Err(());
    }
    if second == 60 {
        // Leap second: reject, do not clamp.
        return Err(());
    }
    if second > 60 {
        return Err(());
    }

    let mut idx = 19;
    let mut nanosecond = 0u32;
    if idx < bytes.len() && bytes[idx] == b'.' {
        idx += 1;
        let start = idx;
        while idx < bytes.len() && bytes[idx].is_ascii_digit() {
            idx += 1;
        }
        if idx == start {
            return Err(());
        }
        let digits = &text[start..idx];
        let mut padded = digits.to_string();
        while padded.len() < 9 {
            padded.push('0');
        }
        if padded.len() > 9 {
            padded.truncate(9);
        }
        nanosecond = padded.parse().map_err(|_| ())?;
    }

    if idx >= bytes.len() {
        return Err(());
    }

    let offset = if bytes[idx] == b'Z' || bytes[idx] == b'z' {
        if idx + 1 != bytes.len() {
            return Err(());
        }
        UtcOffset::UTC
    } else if bytes[idx] == b'+' || bytes[idx] == b'-' {
        let sign = if bytes[idx] == b'+' { 1i8 } else { -1i8 };
        idx += 1;
        if idx + 5 != bytes.len() {
            return Err(());
        }
        let off_h = parse_n_digits(&bytes[idx..idx + 2], 2)?;
        if bytes[idx + 2] != b':' {
            return Err(());
        }
        let off_m = parse_n_digits(&bytes[idx + 3..idx + 5], 2)?;
        if off_h > 23 || off_m > 59 {
            return Err(());
        }
        UtcOffset::from_hms(sign * off_h as i8, sign * off_m as i8, 0).map_err(|_| ())?
    } else {
        return Err(());
    };

    let month = Month::try_from(month as u8).map_err(|_| ())?;
    let date = Date::from_calendar_date(year as i32, month, day as u8).map_err(|_| ())?;
    let time =
        Time::from_hms_nano(hour as u8, minute as u8, second as u8, nanosecond).map_err(|_| ())?;
    Ok(PrimitiveDateTime::new(date, time)
        .assume_offset(offset)
        .to_offset(UtcOffset::UTC))
}

fn expect(bytes: &[u8], idx: usize, want: u8) -> Result<(), ()> {
    if idx < bytes.len() && bytes[idx] == want {
        Ok(())
    } else {
        Err(())
    }
}

fn parse_n_digits(slice: &[u8], n: usize) -> Result<u32, ()> {
    if slice.len() != n || !slice.iter().all(|b| b.is_ascii_digit()) {
        return Err(());
    }
    let mut value = 0u32;
    for b in slice {
        value = value * 10 + u32::from(b - b'0');
    }
    Ok(value)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn reject(label: &str, text: &str) {
        assert!(
            parse(text).is_err(),
            "{label}: expected reject of timestamp"
        );
    }

    fn accept(label: &str, text: &str) {
        assert!(parse(text).is_ok(), "{label}: expected accept of timestamp");
    }

    #[test]
    fn profile_self_test() {
        reject("empty", "");
        reject("whitespace", "   ");
        reject("garbage", "not-a-timestamp");
        reject("date-only", "2026-08-15");
        reject("naive", "2026-08-15T17:00:00");
        reject("space-separator", "2026-08-15 17:00:00Z");
        reject("basic-date-time", "20260815T170000Z");
        reject("week-date", "2026-W33-6T17:00:00Z");
        reject("ordinal-date", "2026-227T17:00:00Z");
        reject("missing-seconds", "2026-08-15T17:00Z");
        reject("offset-without-colon", "2026-08-15T17:00:00+0000");
        reject("comma-fraction", "2026-08-15T17:00:00,123Z");
        reject("invalid-calendar", "2026-02-30T17:00:00Z");
        reject("hour-24", "2026-08-15T24:00:00Z");
        reject("leap-second-utc", "2016-12-31T23:59:60Z");
        reject("leap-second-offset", "2016-12-31T23:59:60+00:00");

        accept("zulu", "2026-08-15T17:00:00Z");
        accept("lowercase", "2026-08-15t17:00:00z");
        accept("numeric-offset", "2026-08-15T17:00:00+00:00");
        accept("negative-offset", "2026-08-15T13:00:00-04:00");
        accept("fractional-seconds", "2026-08-15T17:00:00.123Z");

        assert_eq!(
            compare("1970-01-01T00:00:00Z", "1970-01-01T00:00:00+00:00").unwrap(),
            0
        );
        assert_eq!(
            compare("1970-01-01T01:00:00+01:00", "1970-01-01T00:00:00Z").unwrap(),
            0
        );
        assert_eq!(
            compare("2026-08-15T17:00:00+00:00", "2026-08-15T17:00:00Z").unwrap(),
            0
        );
        assert_eq!(
            compare("1970-01-01T00:00:00Z", "1970-01-01T00:00:01Z").unwrap(),
            -1
        );
        assert_eq!(
            compare("1970-01-01T00:00:01Z", "1970-01-01T00:00:00Z").unwrap(),
            1
        );
        assert_eq!(
            compare("2026-08-15T17:00:00.100Z", "2026-08-15T17:00:00Z").unwrap(),
            1
        );
        assert_eq!(
            compare("2026-08-15T16:00:10Z", "2026-08-15T17:00:00+01:00").unwrap(),
            1
        );
    }
}
