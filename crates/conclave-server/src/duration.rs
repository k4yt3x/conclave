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
        return Err(Error::BadRequest("empty duration string".into()));
    }

    let (num_str, multiplier) = match input.as_bytes().last() {
        Some(b's') => (&input[..input.len() - 1], 1i64),
        Some(b'h') => (&input[..input.len() - 1], 3600),
        Some(b'd') => (&input[..input.len() - 1], 86400),
        Some(b'w') => (&input[..input.len() - 1], 604800),
        Some(b'm') => (&input[..input.len() - 1], 2592000),
        Some(b'y') => (&input[..input.len() - 1], 31536000),
        _ => {
            return Err(Error::BadRequest(format!(
                "invalid duration '{input}': must end with s, h, d, w, m, or y"
            )));
        }
    };

    let number: i64 = num_str.parse().map_err(|_| {
        Error::BadRequest(format!(
            "invalid duration '{input}': numeric part '{num_str}' is not a valid integer"
        ))
    })?;

    if number <= 0 {
        return Err(Error::BadRequest(format!(
            "invalid duration '{input}': numeric part must be positive"
        )));
    }

    number
        .checked_mul(multiplier)
        .ok_or_else(|| Error::BadRequest(format!("invalid duration '{input}': value too large")))
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
    }

    #[test]
    fn test_parse_hours() {
        assert_eq!(parse_duration("1h").unwrap(), 3600);
        assert_eq!(parse_duration("2h").unwrap(), 7200);
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
    }

    #[test]
    fn test_parse_months() {
        assert_eq!(parse_duration("1m").unwrap(), 2592000);
    }

    #[test]
    fn test_parse_years() {
        assert_eq!(parse_duration("1y").unwrap(), 31536000);
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
}
