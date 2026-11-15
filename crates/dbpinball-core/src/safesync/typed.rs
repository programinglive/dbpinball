//! Typed range-filter value parsing and canonicalization.
//!
//! Every `gte` / `lte` predicate is validated against the column's catalogue
//! type before any SQL is constructed. Decimals are normalized without floating
//! point, timestamps are checked without locale or server-timezone ambiguity,
//! and unsupported or unordered types fail closed with `invalid_filter`.

use sqlx::types::chrono::{DateTime, FixedOffset, NaiveDate, NaiveDateTime, NaiveTime, Utc};

use crate::domain::error::{CoreError, ErrorCode};
use crate::safesync::spec::FilterValue;

const MAX_NUMERIC_FRACTION_DIGITS: usize = 1_000;

/// A validated, canonicalized range-filter bound.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TypedValue {
    Integer { value: i64, pg_type: &'static str },
    Numeric(String),
    Date(String),
    Timestamp(String),
    TimestampTz(String),
}

impl TypedValue {
    /// Canonical string representation used both for the SQL bind and the
    /// keyed filter digest.
    pub fn canonical(&self) -> String {
        match self {
            TypedValue::Integer { value, .. } => value.to_string(),
            TypedValue::Numeric(value)
            | TypedValue::Date(value)
            | TypedValue::Timestamp(value)
            | TypedValue::TimestampTz(value) => value.clone(),
        }
    }

    /// Explicit PostgreSQL cast for the bound parameter (`$n::<cast>`).
    pub fn cast(&self) -> &'static str {
        match self {
            TypedValue::Integer { pg_type, .. } => pg_type,
            TypedValue::Numeric(_) => "numeric",
            TypedValue::Date(_) => "date",
            TypedValue::Timestamp(_) => "timestamp",
            TypedValue::TimestampTz(_) => "timestamptz",
        }
    }
}

/// Parse and canonicalize a filter bound against a PostgreSQL column type.
pub fn parse_typed_value(type_name: &str, value: &FilterValue) -> Result<TypedValue, CoreError> {
    match type_name {
        "smallint" => parse_int(value, i64::from(i16::MIN), i64::from(i16::MAX), "smallint"),
        "integer" => parse_int(value, i64::from(i32::MIN), i64::from(i32::MAX), "integer"),
        "bigint" => parse_int(value, i64::MIN, i64::MAX, "bigint"),
        "numeric" | "decimal" => parse_numeric(value),
        "date" => parse_date(value),
        "timestamp without time zone" => parse_timestamp(value),
        "timestamp with time zone" => parse_timestamptz(value),
        other => Err(invalid_filter(format!(
            "range filters are not supported on column type '{other}'"
        ))),
    }
}

fn parse_int(
    value: &FilterValue,
    min: i64,
    max: i64,
    pg_type: &'static str,
) -> Result<TypedValue, CoreError> {
    let parsed = match value {
        FilterValue::Int(n) => *n,
        FilterValue::Text(raw) => {
            if raw.is_empty() || raw.starts_with('+') {
                return Err(invalid_filter(format!(
                    "invalid {pg_type} literal: signs with '+' or empty values are not allowed"
                )));
            }
            let digits = raw.strip_prefix('-').unwrap_or(raw);
            if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
                return Err(invalid_filter(format!(
                    "invalid {pg_type} literal: expected decimal digits"
                )));
            }
            if (digits.len() > 1 && digits.starts_with('0'))
                || (raw.starts_with('-') && digits == "0")
            {
                return Err(invalid_filter(format!(
                    "invalid {pg_type} literal: leading zeros and -0 are not allowed"
                )));
            }
            raw.parse::<i64>().map_err(|_| {
                invalid_filter(format!("integer literal out of range for {pg_type}"))
            })?
        }
        FilterValue::Bool(_) | FilterValue::List(_) => {
            return Err(invalid_filter(format!(
                "invalid {pg_type} literal: expected integer or decimal string"
            )));
        }
    };

    if parsed < min || parsed > max {
        return Err(invalid_filter(format!(
            "integer literal is out of range for {pg_type}"
        )));
    }

    Ok(TypedValue::Integer {
        value: parsed,
        pg_type,
    })
}

fn parse_numeric(value: &FilterValue) -> Result<TypedValue, CoreError> {
    let raw = match value {
        FilterValue::Int(n) => return Ok(TypedValue::Numeric(n.to_string())),
        FilterValue::Text(s) => s.as_str(),
        FilterValue::Bool(_) | FilterValue::List(_) => {
            return Err(invalid_filter(
                "invalid numeric literal: expected decimal string or integer",
            ));
        }
    };

    if raw.is_empty() {
        return Err(invalid_filter("invalid numeric literal: empty string"));
    }

    let (negative, unsigned) = if let Some(rest) = raw.strip_prefix('-') {
        (true, rest)
    } else if let Some(rest) = raw.strip_prefix('+') {
        (false, rest)
    } else {
        (false, raw)
    };

    if unsigned.is_empty() {
        return Err(invalid_filter("invalid numeric literal: missing digits"));
    }

    let (int_raw, frac_raw) = match unsigned.split_once('.') {
        Some((int_part, frac_part)) => {
            if frac_part.contains('.') || int_part.is_empty() || frac_part.is_empty() {
                return Err(invalid_filter("invalid numeric literal: malformed decimal"));
            }
            if frac_part.len() > MAX_NUMERIC_FRACTION_DIGITS {
                return Err(invalid_filter(
                    "invalid numeric literal: exceeds 1000 fractional digits",
                ));
            }
            (int_part, frac_part)
        }
        None => (unsigned, ""),
    };

    if !int_raw.bytes().all(|b| b.is_ascii_digit()) || !frac_raw.bytes().all(|b| b.is_ascii_digit())
    {
        return Err(invalid_filter(
            "invalid numeric literal: only decimal digits and optional '.' are allowed",
        ));
    }

    let int_trimmed = int_raw.trim_start_matches('0');
    let int_part = if int_trimmed.is_empty() {
        "0"
    } else {
        int_trimmed
    };
    let frac_part = frac_raw.trim_end_matches('0');

    let canonical = if int_part == "0" && frac_part.is_empty() {
        "0".to_string()
    } else {
        let sign = if negative { "-" } else { "" };
        if frac_part.is_empty() {
            format!("{sign}{int_part}")
        } else {
            format!("{sign}{int_part}.{frac_part}")
        }
    };

    Ok(TypedValue::Numeric(canonical))
}

fn parse_date(value: &FilterValue) -> Result<TypedValue, CoreError> {
    let raw = expect_text(value, "date")?;
    let date = parse_strict_date(raw)?;
    Ok(TypedValue::Date(date.format("%Y-%m-%d").to_string()))
}

fn parse_timestamp(value: &FilterValue) -> Result<TypedValue, CoreError> {
    let raw = expect_text(value, "timestamp")?;
    let (date_str, time_str) = split_date_time(raw, "timestamp")?;
    let date = parse_strict_date(date_str)?;

    if time_str.contains(['Z', 'z', '+', '-']) {
        return Err(invalid_filter(
            "timestamp without time zone literal must not include an offset or Z",
        ));
    }

    let time = parse_strict_time(time_str)?;
    let dt = NaiveDateTime::new(date, time);
    Ok(TypedValue::Timestamp(format_naive_datetime(dt)))
}

fn parse_timestamptz(value: &FilterValue) -> Result<TypedValue, CoreError> {
    let raw = expect_text(value, "timestamptz")?;
    let (date_str, time_and_tz) = split_date_time(raw, "timestamptz")?;
    let date = parse_strict_date(date_str)?;

    let (time_str, offset) = split_time_and_offset(time_and_tz)?;
    let time = parse_strict_time(time_str)?;
    let naive = NaiveDateTime::new(date, time);
    let local = naive
        .and_local_timezone(offset)
        .single()
        .ok_or_else(|| invalid_filter("invalid timestamptz offset"))?;
    let utc: DateTime<Utc> = local.with_timezone(&Utc);
    Ok(TypedValue::TimestampTz(format_utc_datetime(utc)))
}

fn expect_text<'a>(value: &'a FilterValue, label: &str) -> Result<&'a str, CoreError> {
    match value {
        FilterValue::Text(s) => Ok(s.as_str()),
        FilterValue::Bool(_) | FilterValue::Int(_) | FilterValue::List(_) => Err(invalid_filter(
            format!("invalid {label} literal: expected string"),
        )),
    }
}

fn split_date_time<'a>(raw: &'a str, label: &str) -> Result<(&'a str, &'a str), CoreError> {
    let bytes = raw.as_bytes();
    if bytes.len() < 16 {
        return Err(invalid_filter(format!(
            "invalid {label} literal: expected YYYY-MM-DD HH:MM[:SS[.ffffff]]"
        )));
    }
    let sep = bytes[10];
    if sep != b' ' && sep != b'T' {
        return Err(invalid_filter(format!(
            "invalid {label} literal: expected space or 'T' between date and time"
        )));
    }
    Ok((&raw[..10], &raw[11..]))
}

fn parse_strict_date(raw: &str) -> Result<NaiveDate, CoreError> {
    let b = raw.as_bytes();
    if b.len() != 10
        || b[4] != b'-'
        || b[7] != b'-'
        || !b[..4].iter().all(u8::is_ascii_digit)
        || !b[5..7].iter().all(u8::is_ascii_digit)
        || !b[8..10].iter().all(u8::is_ascii_digit)
    {
        return Err(invalid_filter("invalid date literal: expected YYYY-MM-DD"));
    }
    NaiveDate::parse_from_str(raw, "%Y-%m-%d").map_err(|_| invalid_filter("invalid calendar date"))
}

fn parse_strict_time(raw: &str) -> Result<NaiveTime, CoreError> {
    let (hms, frac) = match raw.split_once('.') {
        Some((main, frac)) => {
            if frac.is_empty() || frac.len() > 6 || !frac.bytes().all(|b| b.is_ascii_digit()) {
                return Err(invalid_filter(
                    "invalid time literal: fractional seconds must be 1 to 6 digits",
                ));
            }
            (main, Some(frac))
        }
        None => (raw, None),
    };

    let parts: Vec<&str> = hms.split(':').collect();
    let (hh_str, mm_str, ss_str) = match parts.as_slice() {
        [hh, mm] => {
            if frac.is_some() {
                return Err(invalid_filter(
                    "invalid time literal: fractional seconds require seconds",
                ));
            }
            (*hh, *mm, "00")
        }
        [hh, mm, ss] => (*hh, *mm, *ss),
        _ => {
            return Err(invalid_filter(
                "invalid time literal: expected HH:MM or HH:MM:SS[.ffffff]",
            ));
        }
    };

    if hh_str.len() != 2
        || mm_str.len() != 2
        || ss_str.len() != 2
        || !hh_str.bytes().all(|b| b.is_ascii_digit())
        || !mm_str.bytes().all(|b| b.is_ascii_digit())
        || !ss_str.bytes().all(|b| b.is_ascii_digit())
    {
        return Err(invalid_filter(
            "invalid time literal: expected two-digit time components",
        ));
    }

    let hh: u32 = hh_str.parse().map_err(|_| invalid_filter("invalid hour"))?;
    let mm: u32 = mm_str
        .parse()
        .map_err(|_| invalid_filter("invalid minute"))?;
    let ss: u32 = ss_str
        .parse()
        .map_err(|_| invalid_filter("invalid second"))?;

    let micros: u32 = match frac {
        Some(f) => {
            let mut padded = f.to_string();
            while padded.len() < 6 {
                padded.push('0');
            }
            padded
                .parse()
                .map_err(|_| invalid_filter("invalid fractional seconds"))?
        }
        None => 0,
    };

    NaiveTime::from_hms_micro_opt(hh, mm, ss, micros)
        .ok_or_else(|| invalid_filter("invalid time value"))
}

fn split_time_and_offset(raw: &str) -> Result<(&str, FixedOffset), CoreError> {
    if let Some(time_str) = raw.strip_suffix('Z').or_else(|| raw.strip_suffix('z')) {
        let offset = FixedOffset::east_opt(0).ok_or_else(|| invalid_filter("invalid offset"))?;
        return Ok((time_str, offset));
    }

    let Some(pos) = raw.rfind(['+', '-']) else {
        return Err(invalid_filter(
            "timestamptz literal must include an explicit numeric offset or Z",
        ));
    };

    let time_str = &raw[..pos];
    let offset_str = &raw[pos..];
    let sign = if offset_str.starts_with('+') { 1 } else { -1 };
    let rest = &offset_str[1..];
    let Some((oh_str, om_str)) = rest.split_once(':') else {
        return Err(invalid_filter(
            "timestamptz offset must be Z, +HH:MM, or -HH:MM",
        ));
    };

    if oh_str.len() != 2
        || om_str.len() != 2
        || !oh_str.bytes().all(|b| b.is_ascii_digit())
        || !om_str.bytes().all(|b| b.is_ascii_digit())
    {
        return Err(invalid_filter(
            "timestamptz offset must be +HH:MM or -HH:MM",
        ));
    }

    let oh: i32 = oh_str
        .parse()
        .map_err(|_| invalid_filter("invalid offset hour"))?;
    let om: i32 = om_str
        .parse()
        .map_err(|_| invalid_filter("invalid offset minute"))?;

    if oh > 23 || om > 59 {
        return Err(invalid_filter("timestamptz offset out of range"));
    }

    let secs = sign * (oh * 3600 + om * 60);
    let offset =
        FixedOffset::east_opt(secs).ok_or_else(|| invalid_filter("invalid timestamptz offset"))?;
    Ok((time_str, offset))
}

fn format_naive_datetime(dt: NaiveDateTime) -> String {
    let micros = dt.and_utc().timestamp_subsec_micros();
    if micros == 0 {
        dt.format("%Y-%m-%d %H:%M:%S").to_string()
    } else {
        let frac = format!("{micros:06}");
        let trimmed = frac.trim_end_matches('0');
        format!("{}.{trimmed}", dt.format("%Y-%m-%d %H:%M:%S"))
    }
}

fn format_utc_datetime(dt: DateTime<Utc>) -> String {
    let micros = dt.timestamp_subsec_micros();
    if micros == 0 {
        dt.format("%Y-%m-%dT%H:%M:%SZ").to_string()
    } else {
        let frac = format!("{micros:06}");
        let trimmed = frac.trim_end_matches('0');
        format!(
            "{}T{}.{trimmed}Z",
            dt.format("%Y-%m-%d"),
            dt.format("%H:%M:%S")
        )
    }
}

fn invalid_filter(message: impl Into<String>) -> CoreError {
    CoreError::with_detail(ErrorCode::Usage, message, "invalid_filter")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn integers_validate_range_and_format() {
        let ok = parse_typed_value("smallint", &FilterValue::Int(32767)).unwrap();
        assert_eq!(ok.canonical(), "32767");
        assert_eq!(ok.cast(), "smallint");

        let ok_str = parse_typed_value("integer", &FilterValue::Text("-42".into())).unwrap();
        assert_eq!(ok_str.canonical(), "-42");
        assert_eq!(ok_str.cast(), "integer");

        let ok_zero = parse_typed_value("bigint", &FilterValue::Text("0".into())).unwrap();
        assert_eq!(ok_zero.canonical(), "0");
        assert_eq!(ok_zero.cast(), "bigint");

        for bad in [
            FilterValue::Int(32768),
            FilterValue::Int(-32769),
            FilterValue::Text("+42".into()),
            FilterValue::Text("007".into()),
            FilterValue::Text("-007".into()),
            FilterValue::Text("-0".into()),
            FilterValue::Text("1.0".into()),
            FilterValue::Bool(true),
        ] {
            let err = parse_typed_value("smallint", &bad).unwrap_err();
            assert_eq!(err.detail.as_deref(), Some("invalid_filter"), "bad={bad:?}");
        }
    }

    #[test]
    fn numeric_canonicalizes_without_floats() {
        let a = parse_typed_value("numeric", &FilterValue::Text("1.10".into())).unwrap();
        let b = parse_typed_value("decimal", &FilterValue::Text("1.1".into())).unwrap();
        assert_eq!(a.canonical(), "1.1");
        assert_eq!(a.canonical(), b.canonical());
        assert_eq!(a.cast(), "numeric");

        let neg_zero = parse_typed_value("numeric", &FilterValue::Text("-0.000".into())).unwrap();
        assert_eq!(neg_zero.canonical(), "0");

        let high_prec =
            parse_typed_value("numeric", &FilterValue::Text("1.10000000000000001".into())).unwrap();
        assert_eq!(high_prec.canonical(), "1.10000000000000001");
        assert_ne!(high_prec.canonical(), "1.1");

        let too_many_frac = format!("1.{}", "1".repeat(1001));
        for bad in [
            "1e5",
            "NaN",
            "Infinity",
            "-Infinity",
            "",
            "1.",
            ".5",
            &too_many_frac,
        ] {
            let err = parse_typed_value("numeric", &FilterValue::Text(bad.into())).unwrap_err();
            assert_eq!(err.detail.as_deref(), Some("invalid_filter"), "bad={bad}");
        }
    }

    #[test]
    fn date_accepts_strict_iso_and_rejects_time_or_invalid_calendar() {
        let leap = parse_typed_value("date", &FilterValue::Text("2024-02-29".into())).unwrap();
        assert_eq!(leap.canonical(), "2024-02-29");
        assert_eq!(leap.cast(), "date");

        for bad in [
            "2025-02-29",
            "2026-13-01",
            "2026-01-01T00:00:00",
            "2026-01-01Z",
            "2026-1-1",
        ] {
            let err = parse_typed_value("date", &FilterValue::Text(bad.into())).unwrap_err();
            assert_eq!(err.detail.as_deref(), Some("invalid_filter"), "bad={bad}");
        }
    }

    #[test]
    fn timestamp_without_tz_rejects_offsets_and_canonicalizes_space() {
        let t = parse_typed_value(
            "timestamp without time zone",
            &FilterValue::Text("2026-01-01T07:00:00.500000".into()),
        )
        .unwrap();
        assert_eq!(t.canonical(), "2026-01-01 07:00:00.5");
        assert_eq!(t.cast(), "timestamp");

        let short = parse_typed_value(
            "timestamp without time zone",
            &FilterValue::Text("2026-01-01 07:00".into()),
        )
        .unwrap();
        assert_eq!(short.canonical(), "2026-01-01 07:00:00");

        for bad in [
            "2026-01-01T07:00:00Z",
            "2026-01-01 07:00:00+07:00",
            "2026-01-01 07:00:00.1234567",
            "2026-01-01 25:00:00",
        ] {
            let err = parse_typed_value(
                "timestamp without time zone",
                &FilterValue::Text(bad.into()),
            )
            .unwrap_err();
            assert_eq!(err.detail.as_deref(), Some("invalid_filter"), "bad={bad}");
        }
    }

    #[test]
    fn timestamptz_requires_offset_and_normalizes_to_utc() {
        let plus7 = parse_typed_value(
            "timestamp with time zone",
            &FilterValue::Text("2026-01-01T07:00:00+07:00".into()),
        )
        .unwrap();
        let utc = parse_typed_value(
            "timestamp with time zone",
            &FilterValue::Text("2026-01-01T00:00:00Z".into()),
        )
        .unwrap();
        assert_eq!(plus7.canonical(), "2026-01-01T00:00:00Z");
        assert_eq!(plus7.canonical(), utc.canonical());
        assert_eq!(plus7.cast(), "timestamptz");

        let frac = parse_typed_value(
            "timestamp with time zone",
            &FilterValue::Text("2026-01-01 00:00:00.120000Z".into()),
        )
        .unwrap();
        assert_eq!(frac.canonical(), "2026-01-01T00:00:00.12Z");

        for bad in [
            "2026-01-01T00:00:00",
            "2026-01-01T00:00:00 Europe/Paris",
            "2026-01-01T00:00:00+25:00",
            "2026-01-01T00:00:00.1234567Z",
        ] {
            let err = parse_typed_value("timestamp with time zone", &FilterValue::Text(bad.into()))
                .unwrap_err();
            assert_eq!(err.detail.as_deref(), Some("invalid_filter"), "bad={bad}");
        }
    }

    #[test]
    fn unordered_and_unsupported_types_are_refused() {
        for type_name in ["boolean", "text", "uuid", "jsonb", "text[]", "custom_enum"] {
            let err = parse_typed_value(type_name, &FilterValue::Text("1".into())).unwrap_err();
            assert_eq!(
                err.detail.as_deref(),
                Some("invalid_filter"),
                "type_name={type_name}"
            );
        }
    }
}
