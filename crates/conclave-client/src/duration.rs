use crate::error::{Error, Result};

/// Parse a human-readable duration string into seconds.
///
/// Accepts:
/// - `-1` (disabled)
/// - `0` (delete after all members fetch)
/// - `<number><unit>` where unit is `s`/`h`/`d`/`w`/`m`/`y`
///
/// Multipliers: s=1, h=3600, d=86400, w=604800, m=2592000 (30d), y=31536000 (365d).
pub fn parse_duration(input: &str) -> Result<i64> {
    let input = input.trim();

    if input == "-1" {
        return Ok(-1);
    }

    if input == "0" {
        return Ok(0);
    }

    if input.is_empty() {
        return Err(Error::Other("empty duration string".into()));
    }

    let (num_str, multiplier) = match input.as_bytes().last() {
        Some(b's') => (&input[..input.len() - 1], 1i64),
        Some(b'h') => (&input[..input.len() - 1], 3600),
        Some(b'd') => (&input[..input.len() - 1], 86400),
        Some(b'w') => (&input[..input.len() - 1], 604800),
        Some(b'm') => (&input[..input.len() - 1], 2592000),
        Some(b'y') => (&input[..input.len() - 1], 31536000),
        _ => {
            return Err(Error::Other(format!(
                "invalid duration '{input}': must end with s, h, d, w, m, or y"
            )));
        }
    };

    let number: i64 = num_str.parse().map_err(|_| {
        Error::Other(format!(
            "invalid duration '{input}': numeric part '{num_str}' is not a valid integer"
        ))
    })?;

    if number <= 0 {
        return Err(Error::Other(format!(
            "invalid duration '{input}': numeric part must be positive"
        )));
    }

    number
        .checked_mul(multiplier)
        .ok_or_else(|| Error::Other(format!("invalid duration '{input}': value too large")))
}

/// Format seconds back into the most natural human-readable duration string.
///
/// Special values:
/// - `-1` -> "disabled"
/// - `0` -> "fetch-then-delete"
///
/// For positive values, picks the largest unit that divides evenly.
pub fn format_duration(seconds: i64) -> String {
    if seconds == -1 {
        return "disabled".to_string();
    }

    if seconds == 0 {
        return "fetch-then-delete".to_string();
    }

    if seconds < 0 {
        return format!("{seconds}s");
    }

    let units: &[(i64, &str)] = &[
        (31536000, "y"),
        (2592000, "m"),
        (604800, "w"),
        (86400, "d"),
        (3600, "h"),
        (1, "s"),
    ];

    for &(divisor, suffix) in units {
        if seconds % divisor == 0 {
            return format!("{}{suffix}", seconds / divisor);
        }
    }

    format!("{seconds}s")
}

/// Compute the effective expiry from server retention and group expiry.
///
/// Rules:
/// - If both are disabled (-1), effective is disabled (-1).
/// - If one is disabled, the other applies.
/// - If both are positive, the stricter (smaller) one applies.
/// - `0` (fetch-then-delete) is treated as stricter than any positive value.
pub fn compute_effective(server_retention_seconds: i64, group_expiry_seconds: i64) -> i64 {
    match (server_retention_seconds, group_expiry_seconds) {
        (-1, -1) => -1,
        (-1, group) => group,
        (server, -1) => server,
        (0, _) | (_, 0) => 0,
        (server, group) => server.min(group),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_special_values() {
        assert_eq!(parse_duration("-1").unwrap(), -1);
        assert_eq!(parse_duration("0").unwrap(), 0);
    }

    #[test]
    fn test_parse_seconds() {
        assert_eq!(parse_duration("15s").unwrap(), 15);
        assert_eq!(parse_duration("1s").unwrap(), 1);
        assert_eq!(parse_duration("100s").unwrap(), 100);
    }

    #[test]
    fn test_parse_hours() {
        assert_eq!(parse_duration("1h").unwrap(), 3600);
        assert_eq!(parse_duration("2h").unwrap(), 7200);
        assert_eq!(parse_duration("24h").unwrap(), 86400);
    }

    #[test]
    fn test_parse_days() {
        assert_eq!(parse_duration("1d").unwrap(), 86400);
        assert_eq!(parse_duration("7d").unwrap(), 604800);
        assert_eq!(parse_duration("30d").unwrap(), 2592000);
    }

    #[test]
    fn test_parse_weeks() {
        assert_eq!(parse_duration("1w").unwrap(), 604800);
        assert_eq!(parse_duration("4w").unwrap(), 2419200);
    }

    #[test]
    fn test_parse_months() {
        assert_eq!(parse_duration("1m").unwrap(), 2592000);
        assert_eq!(parse_duration("6m").unwrap(), 15552000);
    }

    #[test]
    fn test_parse_years() {
        assert_eq!(parse_duration("1y").unwrap(), 31536000);
        assert_eq!(parse_duration("2y").unwrap(), 63072000);
    }

    #[test]
    fn test_parse_whitespace() {
        assert_eq!(parse_duration("  7d  ").unwrap(), 604800);
    }

    #[test]
    fn test_parse_invalid() {
        assert!(parse_duration("").is_err());
        assert!(parse_duration("abc").is_err());
        assert!(parse_duration("10x").is_err());
        assert!(parse_duration("d").is_err());
        assert!(parse_duration("-2d").is_err());
        assert!(parse_duration("0d").is_err());
    }

    #[test]
    fn test_parse_overflow() {
        assert!(parse_duration("999999999999y").is_err());
    }

    #[test]
    fn test_format_special_values() {
        assert_eq!(format_duration(-1), "disabled");
        assert_eq!(format_duration(0), "fetch-then-delete");
    }

    #[test]
    fn test_format_exact_units() {
        assert_eq!(format_duration(1), "1s");
        assert_eq!(format_duration(15), "15s");
        assert_eq!(format_duration(3600), "1h");
        assert_eq!(format_duration(7200), "2h");
        assert_eq!(format_duration(86400), "1d");
        assert_eq!(format_duration(604800), "1w");
        assert_eq!(format_duration(2592000), "1m");
        assert_eq!(format_duration(31536000), "1y");
    }

    #[test]
    fn test_format_non_exact_falls_to_smaller_unit() {
        assert_eq!(format_duration(90), "90s");
        assert_eq!(format_duration(5400), "5400s");
    }

    #[test]
    fn test_format_roundtrip() {
        // Only positive durations roundtrip; special values (-1, 0) format to
        // human-readable strings ("disabled", "fetch-then-delete") that are not
        // valid parse inputs.
        for input in &["15s", "2h", "7d", "4w", "1m", "1y"] {
            let seconds = parse_duration(input).unwrap();
            let formatted = format_duration(seconds);
            let re_parsed = parse_duration(&formatted).unwrap();
            assert_eq!(
                seconds, re_parsed,
                "roundtrip failed for {input}: {seconds} -> {formatted} -> {re_parsed}"
            );
        }
    }

    #[test]
    fn test_compute_effective() {
        assert_eq!(compute_effective(-1, -1), -1);
        assert_eq!(compute_effective(-1, 3600), 3600);
        assert_eq!(compute_effective(86400, -1), 86400);
        assert_eq!(compute_effective(86400, 3600), 3600);
        assert_eq!(compute_effective(3600, 86400), 3600);
        assert_eq!(compute_effective(0, 3600), 0);
        assert_eq!(compute_effective(3600, 0), 0);
        assert_eq!(compute_effective(0, 0), 0);
        assert_eq!(compute_effective(0, -1), 0);
        assert_eq!(compute_effective(-1, 0), 0);
    }
}
