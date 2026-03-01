# Duration Format

Several configuration fields and commands use a human-readable duration string format.

## Syntax

```
<number><unit>
```

Where `<number>` is a positive integer and `<unit>` is one of the supported time units.

## Special Values

| Value | Meaning |
|-------|---------|
| `"-1"` | Disabled / indefinite (no limit) |
| `"0"` | Immediate / delete-after-fetch |

## Time Units

| Unit | Multiplier (seconds) | Example | Equivalent |
|------|---------------------|---------|------------|
| `s` | 1 | `"15s"` | 15 seconds |
| `h` | 3,600 | `"2h"` | 7,200 seconds |
| `d` | 86,400 | `"7d"` | 604,800 seconds |
| `w` | 604,800 | `"4w"` | 2,419,200 seconds |
| `m` | 2,592,000 (30 days) | `"1m"` | 2,592,000 seconds |
| `y` | 31,536,000 (365 days) | `"1y"` | 31,536,000 seconds |

## Usage

This format is used in:

- **Server configuration**: `message_retention` and `cleanup_interval` fields.
- **Client commands**: `/expire` command for setting per-group message expiry.
- **API responses**: Durations are represented as integer seconds (`i64`) in protobuf messages, not as duration strings. The string format is used only in configuration files and user-facing commands.

## Examples

| Duration String | Seconds |
|----------------|---------|
| `"15s"` | 15 |
| `"2h"` | 7,200 |
| `"7d"` | 604,800 |
| `"4w"` | 2,419,200 |
| `"1m"` | 2,592,000 |
| `"1y"` | 31,536,000 |
| `"-1"` | -1 (disabled) |
| `"0"` | 0 (immediate) |

## Error Conditions

Parsing fails for:

- Empty strings.
- Missing unit suffix (e.g., `"30"`).
- Non-positive numbers (e.g., `"-5d"`, except the special value `"-1"`).
- Unknown unit characters (e.g., `"5x"`).
- Non-numeric prefixes (e.g., `"abcd"`).
