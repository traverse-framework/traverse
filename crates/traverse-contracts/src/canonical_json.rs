//! Canonical guest input bytes (spec `1722-canonical-guest-input-bytes`,
//! Decision 118).
//!
//! Every host feeds a capability guest the RFC 8785 JSON Canonicalization
//! Scheme (JCS) form of its JSON input, so the same WASM given the same
//! logical input produces the same output on every host. Input JCS cannot
//! represent exactly is rejected with [`INPUT_NOT_CANONICALIZABLE`] before
//! any guest runs (FR-003).

use serde::de::{self, Deserialize, Deserializer, MapAccess, SeqAccess, Visitor};
use serde_json::{Map, Number, Value};
use std::fmt::{self, Write as _};

/// Stable error code for input JCS cannot represent exactly (FR-003).
pub const INPUT_NOT_CANONICALIZABLE: &str = "input_not_canonicalizable";

/// Largest integer magnitude an IEEE double represents exactly (2^53 − 1).
pub const MAX_SAFE_INTEGER: u64 = 9_007_199_254_740_991;

/// [`MAX_SAFE_INTEGER`] as a double. Every double of greater magnitude is an
/// integer, so this one bound covers integer and float literals alike.
const MAX_SAFE_INTEGER_F64: f64 = 9_007_199_254_740_991.0;

/// Why input could not be canonicalized. Messages never echo the input.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CanonicalJsonError {
    /// The bytes are not JSON at all; hosts keep their existing error.
    InvalidJson,
    /// A number's magnitude exceeds [`MAX_SAFE_INTEGER`]. Any such number is
    /// an integer, including a double such as `1e21`.
    UnsafeInteger,
    /// An object repeats a member name.
    DuplicateKey,
    /// A string is not valid Unicode (invalid UTF-8 or a lone surrogate).
    InvalidUnicode,
}

impl CanonicalJsonError {
    /// The stable error code; `invalid_json` for bytes that are not JSON.
    #[must_use]
    pub fn code(self) -> &'static str {
        match self {
            Self::InvalidJson => "invalid_json",
            Self::UnsafeInteger | Self::DuplicateKey | Self::InvalidUnicode => {
                INPUT_NOT_CANONICALIZABLE
            }
        }
    }
}

impl fmt::Display for CanonicalJsonError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let detail = match self {
            Self::InvalidJson => "input is not valid JSON",
            Self::UnsafeInteger => "input contains an integer beyond ±(2^53 − 1)",
            Self::DuplicateKey => "input contains an object with a duplicate member name",
            Self::InvalidUnicode => "input contains a string that is not valid Unicode",
        };
        write!(f, "{}: {detail}", self.code())
    }
}

impl std::error::Error for CanonicalJsonError {}

/// Parses caller bytes, rejecting duplicate member names and invalid
/// Unicode, which a plain `serde_json` parse would silently accept or report
/// only as a syntax error.
///
/// # Errors
///
/// Returns [`CanonicalJsonError::InvalidJson`] for non-JSON bytes, or the
/// FR-003 variant for JSON that JCS cannot represent.
pub fn parse_canonicalizable(bytes: &[u8]) -> Result<Value, CanonicalJsonError> {
    if serde_json::from_slice::<Value>(bytes).is_err() {
        return Err(
            if std::str::from_utf8(bytes).is_err() || has_lone_surrogate_escape(bytes) {
                CanonicalJsonError::InvalidUnicode
            } else {
                CanonicalJsonError::InvalidJson
            },
        );
    }
    // The bytes are well-formed JSON, so the only way the strict parse can
    // still fail is a duplicate member name.
    serde_json::from_slice::<StrictValue>(bytes)
        .map(|strict| strict.0)
        .map_err(|_| CanonicalJsonError::DuplicateKey)
}

/// Serializes `value` as RFC 8785 canonical JSON.
///
/// # Errors
///
/// Returns [`CanonicalJsonError::UnsafeInteger`] when a number's magnitude
/// exceeds [`MAX_SAFE_INTEGER`].
pub fn to_canonical_json(value: &Value) -> Result<String, CanonicalJsonError> {
    let mut out = String::new();
    write_value(&mut out, value)?;
    Ok(out)
}

/// Parses caller bytes and returns their canonical form (FR-001, FR-003).
///
/// # Errors
///
/// See [`parse_canonicalizable`] and [`to_canonical_json`].
pub fn canonicalize_json_bytes(bytes: &[u8]) -> Result<String, CanonicalJsonError> {
    to_canonical_json(&parse_canonicalizable(bytes)?)
}

fn write_value(out: &mut String, value: &Value) -> Result<(), CanonicalJsonError> {
    match value {
        Value::Null => out.push_str("null"),
        Value::Bool(flag) => out.push_str(if *flag { "true" } else { "false" }),
        Value::Number(number) => write_number(out, number)?,
        Value::String(text) => write_string(out, text),
        Value::Array(items) => {
            out.push('[');
            for (index, item) in items.iter().enumerate() {
                if index > 0 {
                    out.push(',');
                }
                write_value(out, item)?;
            }
            out.push(']');
        }
        Value::Object(members) => {
            // RFC 8785 §3.2.3: order members by their UTF-16 code units.
            let mut entries: Vec<(Vec<u16>, &String, &Value)> = members
                .iter()
                .map(|(key, member)| (key.encode_utf16().collect(), key, member))
                .collect();
            entries.sort_by(|left, right| left.0.cmp(&right.0));
            out.push('{');
            for (index, (_, key, member)) in entries.into_iter().enumerate() {
                if index > 0 {
                    out.push(',');
                }
                write_string(out, key);
                out.push(':');
                write_value(out, member)?;
            }
            out.push('}');
        }
    }
    Ok(())
}

fn write_number(out: &mut String, number: &Number) -> Result<(), CanonicalJsonError> {
    if let Some(unsigned) = number.as_u64() {
        if unsigned > MAX_SAFE_INTEGER {
            return Err(CanonicalJsonError::UnsafeInteger);
        }
        out.push_str(&unsigned.to_string());
    } else if let Some(signed) = number.as_i64() {
        if signed.unsigned_abs() > MAX_SAFE_INTEGER {
            return Err(CanonicalJsonError::UnsafeInteger);
        }
        out.push_str(&signed.to_string());
    } else {
        // A `serde_json::Number` that is neither u64 nor i64 is a finite f64;
        // integer literals beyond i64/u64 also land here.
        let float = number.as_f64().unwrap_or_default();
        if float.abs() > MAX_SAFE_INTEGER_F64 {
            return Err(CanonicalJsonError::UnsafeInteger);
        }
        out.push_str(&format_f64(float));
    }
    Ok(())
}

/// ECMAScript `Number::toString` (RFC 8785 §3.2.2.3) for a finite double of
/// magnitude at most [`MAX_SAFE_INTEGER`]. That bound keeps the decimal
/// exponent `n` at most 16, so ECMAScript's `n <= 21` limits always hold and
/// only small magnitudes take the exponential form, always with `e-`.
fn format_f64(value: f64) -> String {
    if value == 0.0 {
        return "0".to_string();
    }
    // Rust's `{:e}` yields the shortest round-trip digits, e.g. `-1.2345e-7`.
    let scientific = format!("{:e}", value.abs());
    let (mantissa, exponent) = scientific.split_once('e').unwrap_or((&scientific, "0"));
    let digits: String = mantissa.chars().filter(char::is_ascii_digit).collect();
    let exponent: i32 = exponent.parse().unwrap_or_default();
    let k = i32::try_from(digits.len()).unwrap_or(i32::MAX);
    let n = exponent + 1;
    let mut out = String::new();
    if value < 0.0 {
        out.push('-');
    }
    if k <= n {
        out.push_str(&digits);
        out.extend(std::iter::repeat_n(
            '0',
            usize::try_from(n - k).unwrap_or(0),
        ));
    } else if 0 < n {
        let (whole, fraction) = digits.split_at(usize::try_from(n).unwrap_or(0));
        out.push_str(whole);
        out.push('.');
        out.push_str(fraction);
    } else if -6 < n && n <= 0 {
        out.push_str("0.");
        out.extend(std::iter::repeat_n('0', usize::try_from(-n).unwrap_or(0)));
        out.push_str(&digits);
    } else {
        let (first, rest) = digits.split_at(1);
        out.push_str(first);
        if !rest.is_empty() {
            out.push('.');
            out.push_str(rest);
        }
        out.push_str("e-");
        out.push_str(&(1 - n).to_string());
    }
    out
}

/// RFC 8785 §3.2.2.2 string serialization.
fn write_string(out: &mut String, text: &str) {
    out.push('"');
    for character in text.chars() {
        match character {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\u{8}' => out.push_str("\\b"),
            '\t' => out.push_str("\\t"),
            '\n' => out.push_str("\\n"),
            '\u{c}' => out.push_str("\\f"),
            '\r' => out.push_str("\\r"),
            control if u32::from(control) < 0x20 => {
                // Writing to a `String` cannot fail.
                let _ = write!(out, "\\u{:04x}", u32::from(control));
            }
            other => out.push(other),
        }
    }
    out.push('"');
}

/// Whether a JSON string literal contains a `\u` escape for a surrogate that
/// is not part of a high/low pair. Only called on bytes `serde_json` rejected.
fn has_lone_surrogate_escape(bytes: &[u8]) -> bool {
    let mut index = 0;
    let mut in_string = false;
    let mut pending_high = false;
    while index < bytes.len() {
        let byte = bytes[index];
        if !in_string {
            in_string = byte == b'"';
            index += 1;
            continue;
        }
        if byte == b'\\' && bytes.get(index + 1) == Some(&b'u') {
            let unit = bytes
                .get(index + 2..index + 6)
                .and_then(|hex| std::str::from_utf8(hex).ok())
                .and_then(|hex| u16::from_str_radix(hex, 16).ok());
            match unit {
                Some(0xD800..=0xDBFF) if !pending_high => pending_high = true,
                Some(0xDC00..=0xDFFF) if pending_high => pending_high = false,
                Some(0xD800..=0xDFFF) => return true,
                _ if pending_high => return true,
                _ => {}
            }
            index += 6;
            continue;
        }
        if pending_high {
            return true;
        }
        if byte == b'\\' {
            index += 2;
            continue;
        }
        in_string = byte != b'"';
        index += 1;
    }
    pending_high
}

/// A JSON value whose objects must not repeat a member name.
struct StrictValue(Value);

impl<'de> Deserialize<'de> for StrictValue {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        deserializer.deserialize_any(StrictVisitor).map(StrictValue)
    }
}

struct StrictVisitor;

impl<'de> Visitor<'de> for StrictVisitor {
    type Value = Value;

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("a JSON value")
    }

    fn visit_bool<E>(self, value: bool) -> Result<Value, E> {
        Ok(Value::Bool(value))
    }

    fn visit_i64<E>(self, value: i64) -> Result<Value, E> {
        Ok(Value::from(value))
    }

    fn visit_u64<E>(self, value: u64) -> Result<Value, E> {
        Ok(Value::from(value))
    }

    fn visit_f64<E>(self, value: f64) -> Result<Value, E> {
        Ok(Value::from(value))
    }

    fn visit_str<E>(self, value: &str) -> Result<Value, E> {
        Ok(Value::String(value.to_string()))
    }

    fn visit_unit<E>(self) -> Result<Value, E> {
        Ok(Value::Null)
    }

    fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Value, A::Error> {
        let mut items = Vec::new();
        while let Some(StrictValue(item)) = seq.next_element()? {
            items.push(item);
        }
        Ok(Value::Array(items))
    }

    fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Value, A::Error> {
        let mut members = Map::new();
        while let Some(key) = map.next_key::<String>()? {
            let StrictValue(member) = map.next_value()?;
            if members.insert(key, member).is_some() {
                return Err(de::Error::custom("duplicate member name"));
            }
        }
        Ok(Value::Object(members))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn canonical(bytes: &str) -> Result<String, CanonicalJsonError> {
        canonicalize_json_bytes(bytes.as_bytes())
    }

    fn double(bits: u64) -> String {
        to_canonical_json(&json!(f64::from_bits(bits))).unwrap_or_default()
    }

    #[test]
    fn rfc8785_number_vectors_within_the_safe_range() {
        assert_eq!(double(0x0000_0000_0000_0000), "0");
        assert_eq!(double(0x8000_0000_0000_0000), "0");
        assert_eq!(double(0x0000_0000_0000_0001), "5e-324");
        assert_eq!(double(0x8000_0000_0000_0001), "-5e-324");
        assert_eq!(double(0x433f_ffff_ffff_ffff), "9007199254740991");
        assert_eq!(double(0x3eb0_c6f7_a0b5_ed8d), "0.000001");
        assert_eq!(double(0x3eb0_c6f7_a0b5_ed8c), "9.999999999999997e-7");
        assert_eq!(double(0x41b3_de43_5555_5553), "333333333.3333332");
        assert_eq!(double(0x41b3_de43_5555_5556), "333333333.3333334");
    }

    #[test]
    fn numbers_follow_ecmascript_layout() {
        assert_eq!(
            canonical("[1.0,1e2,-12.5,0.5,1.5e-6,1e-7,-4.5e-7,123.456]"),
            Ok("[1,100,-12.5,0.5,0.0000015,1e-7,-4.5e-7,123.456]".to_string())
        );
        assert_eq!(
            canonical("[-9007199254740991,9007199254740991]"),
            Ok("[-9007199254740991,9007199254740991]".to_string())
        );
    }

    #[test]
    fn rfc8785_sample_without_out_of_range_numbers() {
        let input = r#"{
            "numbers": [333333333.33333329, 4.50, 2e-3, 0.000000000000000000000000001],
            "string": "\u20ac$\u000F\u000aA'\u0042\u0022\u005c\\\"\/",
            "literals": [null, true, false]
        }"#;
        assert_eq!(
            canonical(input),
            Ok(concat!(
                r#"{"literals":[null,true,false],"numbers":[333333333.3333333,4.5,0.002,1e-27],"#,
                r#""string":"€$\u000f\nA'B\"\\\\\"/"}"#
            )
            .to_string())
        );
    }

    #[test]
    fn members_sort_by_utf16_code_units() {
        let input = r#"{"\u20ac":"Euro","\r":"CR","\ufb33":"Hebrew","1":"One","\ud83d\ude00":"Smiley","\u0080":"Ctl","\u00f6":"Latin"}"#;
        // U+1F600 encodes as the surrogate pair D83D DE00, so it sorts before
        // U+FB33 in UTF-16 order although it follows it in code-point order.
        let expected = "{\"\\r\":\"CR\",\"1\":\"One\",\"\u{80}\":\"Ctl\",\"\u{f6}\":\"Latin\",\"\u{20ac}\":\"Euro\",\"\u{1f600}\":\"Smiley\",\"\u{fb33}\":\"Hebrew\"}";
        assert_eq!(canonical(input), Ok(expected.to_string()));
    }

    #[test]
    fn whitespace_and_key_order_do_not_change_the_bytes() {
        assert_eq!(
            canonical(r#"{"b":1,"a":2}"#),
            canonical(r#"{ "a" : 2, "b" : 1 }"#)
        );
        assert_eq!(
            canonical(r#"{"b":1,"a":2}"#),
            Ok(r#"{"a":2,"b":1}"#.to_string())
        );
    }

    #[test]
    fn control_characters_use_short_or_lowercase_escapes() {
        let value = json!("\u{8}\t\n\u{c}\r\u{1}\u{1f}\u{7f}");
        assert_eq!(
            to_canonical_json(&value),
            Ok("\"\\b\\t\\n\\f\\r\\u0001\\u001f\u{7f}\"".to_string())
        );
    }

    #[test]
    fn unsafe_integers_are_rejected() {
        for input in [
            "9007199254740992",
            "-9007199254740992",
            "18446744073709551615",
            "100000000000000000000000",
            "1e21",
            "[1.7976931348623157e308]",
            r#"{"nested":{"id":9007199254740993}}"#,
        ] {
            assert_eq!(
                canonical(input),
                Err(CanonicalJsonError::UnsafeInteger),
                "{input}"
            );
        }
        assert_eq!(
            to_canonical_json(&json!(i64::MIN)),
            Err(CanonicalJsonError::UnsafeInteger)
        );
    }

    #[test]
    fn duplicate_member_names_are_rejected_at_any_depth() {
        assert_eq!(
            canonical(r#"{"a":1,"a":2}"#),
            Err(CanonicalJsonError::DuplicateKey)
        );
        assert_eq!(
            canonical(r#"[{"x":{"a":1,"b":2,"a":3}}]"#),
            Err(CanonicalJsonError::DuplicateKey)
        );
    }

    #[test]
    fn invalid_unicode_is_rejected() {
        for input in [
            r#""\ud800""#,
            r#""\udc00""#,
            r#""\ud800x""#,
            r#""\ud800\u0041""#,
            r#""\ud800\ud800""#,
            r#""\ud800\"#,
        ] {
            assert_eq!(
                canonical(input),
                Err(CanonicalJsonError::InvalidUnicode),
                "{input}"
            );
        }
        assert_eq!(
            canonicalize_json_bytes(b"\"\xff\""),
            Err(CanonicalJsonError::InvalidUnicode)
        );
        assert_eq!(canonical(r#""\ud83d\ude00""#), Ok("\"😀\"".to_string()));
    }

    #[test]
    fn non_json_keeps_its_own_error() {
        for input in ["", "{", r#""\uZZZZ""#, r#"{"a":"\n" x}"#, r#"["\\", ]"#] {
            assert_eq!(
                canonical(input),
                Err(CanonicalJsonError::InvalidJson),
                "{input}"
            );
        }
    }

    #[test]
    fn error_codes_and_messages_are_stable_and_value_free() {
        assert_eq!(CanonicalJsonError::InvalidJson.code(), "invalid_json");
        for error in [
            CanonicalJsonError::UnsafeInteger,
            CanonicalJsonError::DuplicateKey,
            CanonicalJsonError::InvalidUnicode,
        ] {
            assert_eq!(error.code(), INPUT_NOT_CANONICALIZABLE);
            assert!(error.to_string().starts_with("input_not_canonicalizable: "));
        }
        assert_eq!(
            CanonicalJsonError::InvalidJson.to_string(),
            "invalid_json: input is not valid JSON"
        );
    }

    #[test]
    fn strict_visitor_describes_what_it_expects() {
        let expected: &dyn de::Expected = &StrictVisitor;
        assert_eq!(expected.to_string(), "a JSON value");
    }

    #[test]
    fn nested_values_round_trip_canonically() {
        let input = r#"{"z":[true,{"b":null,"a":"x"}],"a":-0.0,"m":[]}"#;
        assert_eq!(
            canonical(input),
            Ok(r#"{"a":0,"m":[],"z":[true,{"a":"x","b":null}]}"#.to_string())
        );
    }
}
