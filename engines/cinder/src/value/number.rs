// SPDX-License-Identifier: LicenseRef-PolyForm-Noncommercial-1.0.0-OR-Commercial
// Copyright (c) 2026 Buntato
//! Number to string and string to number, following the language rules.
//!
//! The shortest digits that read back to the same double come from std's
//! `{:e}` formatting, which already gives the shortest round-trip form. This
//! module lays those digits out the way `Number.prototype.toString()` says
//! to (plain up to 21 digits, exponent form past that, and so on).

/// `ToString` for a number, in base 10.
pub fn number_to_string(x: f64) -> String {
    if x.is_nan() {
        return "NaN".to_string();
    }
    if x == 0.0 {
        return "0".to_string();
    }
    if x.is_infinite() {
        return if x > 0.0 { "Infinity" } else { "-Infinity" }.to_string();
    }
    let mut out = String::new();
    if x < 0.0 {
        out.push('-');
    }
    let (digits, n) = shortest_digits(x.abs());
    let k = digits.len() as i32;
    if k <= n && n <= 21 {
        out.push_str(&digits);
        for _ in 0..(n - k) {
            out.push('0');
        }
    } else if 0 < n && n <= 21 {
        let (int, frac) = digits.split_at(n as usize);
        out.push_str(int);
        out.push('.');
        out.push_str(frac);
    } else if -6 < n && n <= 0 {
        out.push_str("0.");
        for _ in 0..(-n) {
            out.push('0');
        }
        out.push_str(&digits);
    } else {
        let e = n - 1;
        let (first, rest) = digits.split_at(1);
        out.push_str(first);
        if !rest.is_empty() {
            out.push('.');
            out.push_str(rest);
        }
        out.push('e');
        out.push(if e >= 0 { '+' } else { '-' });
        out.push_str(&e.unsigned_abs().to_string());
    }
    out
}

/// The shortest decimal digits of a finite positive `x`, and `n`, the
/// position of the decimal point: `x = 0.digits * 10^n`.
fn shortest_digits(x: f64) -> (String, i32) {
    let formatted = format!("{x:e}");
    let (mantissa, exp) = match formatted.split_once('e') {
        Some(parts) => parts,
        None => (formatted.as_str(), "0"),
    };
    let exp: i32 = exp.parse().unwrap_or(0);
    let mut digits: String = mantissa.chars().filter(|c| c.is_ascii_digit()).collect();
    while digits.len() > 1 && digits.ends_with('0') {
        digits.pop();
    }
    (digits, exp + 1)
}

/// True for the characters the language counts as white space or line
/// terminators when trimming a string.
pub fn is_js_whitespace(unit: u16) -> bool {
    matches!(
        unit,
        0x0009 | 0x000A | 0x000B | 0x000C | 0x000D | 0x0020 | 0x00A0 | 0x1680 | 0x2000
            ..=0x200A | 0x2028 | 0x2029 | 0x202F | 0x205F | 0x3000 | 0xFEFF
    )
}

/// `ToNumber` for a string, given as UTF-16 code units. Anything that isn't
/// a valid numeric string gives NaN.
pub fn string_to_number(units: &[u16]) -> f64 {
    let start = units.iter().position(|&u| !is_js_whitespace(u));
    let Some(start) = start else {
        return 0.0;
    };
    let end = units
        .iter()
        .rposition(|&u| !is_js_whitespace(u))
        .map_or(start, |e| e + 1);
    let trimmed = &units[start..end];
    // Everything left must be ASCII to be a number.
    let mut text = String::with_capacity(trimmed.len());
    for &u in trimmed {
        if u >= 0x80 {
            return f64::NAN;
        }
        text.push(u as u8 as char);
    }
    parse_numeric_text(&text)
}

fn parse_numeric_text(text: &str) -> f64 {
    let bytes = text.as_bytes();
    if bytes.len() > 2 && bytes[0] == b'0' {
        let radix = match bytes[1] {
            b'x' | b'X' => Some(16),
            b'o' | b'O' => Some(8),
            b'b' | b'B' => Some(2),
            _ => None,
        };
        if let Some(radix) = radix {
            return parse_power_of_two_radix(&text[2..], radix);
        }
    }
    let (sign, body) = match bytes.first() {
        Some(b'+') => (1.0, &text[1..]),
        Some(b'-') => (-1.0, &text[1..]),
        _ => (1.0, text),
    };
    if body == "Infinity" {
        return sign * f64::INFINITY;
    }
    if !is_decimal_literal(body.as_bytes()) {
        return f64::NAN;
    }
    match body.parse::<f64>() {
        Ok(v) => sign * v,
        Err(_) => f64::NAN,
    }
}

/// `digits [. digits] [e [+-] digits]` or `. digits [...]`, nothing else.
fn is_decimal_literal(b: &[u8]) -> bool {
    let mut i = 0;
    let int_start = i;
    while i < b.len() && b[i].is_ascii_digit() {
        i += 1;
    }
    let int_digits = i - int_start;
    let mut frac_digits = 0;
    if i < b.len() && b[i] == b'.' {
        i += 1;
        let frac_start = i;
        while i < b.len() && b[i].is_ascii_digit() {
            i += 1;
        }
        frac_digits = i - frac_start;
    }
    if int_digits == 0 && frac_digits == 0 {
        return false;
    }
    if i < b.len() && (b[i] == b'e' || b[i] == b'E') {
        i += 1;
        if i < b.len() && (b[i] == b'+' || b[i] == b'-') {
            i += 1;
        }
        let exp_start = i;
        while i < b.len() && b[i].is_ascii_digit() {
            i += 1;
        }
        if i == exp_start {
            return false;
        }
    }
    i == b.len()
}

/// Parse digits in base 2, 8 or 16, rounding to nearest (ties to even)
/// exactly, however many digits there are.
fn parse_power_of_two_radix(digits: &str, radix: u32) -> f64 {
    if digits.is_empty() {
        return f64::NAN;
    }
    let bits_per_digit = radix.trailing_zeros();
    let mut bits: Vec<bool> = Vec::with_capacity(digits.len() * bits_per_digit as usize);
    for ch in digits.chars() {
        let Some(d) = ch.to_digit(radix) else {
            return f64::NAN;
        };
        for shift in (0..bits_per_digit).rev() {
            let bit = (d >> shift) & 1 == 1;
            if bit || !bits.is_empty() {
                bits.push(bit);
            }
        }
    }
    if bits.is_empty() {
        return 0.0;
    }
    if bits.len() <= 53 {
        let mut m: u64 = 0;
        for &b in &bits {
            m = (m << 1) | u64::from(b);
        }
        return m as f64;
    }
    let mut m: u64 = 0;
    for &b in &bits[..53] {
        m = (m << 1) | u64::from(b);
    }
    let round = bits[53];
    let sticky = bits[54..].iter().any(|&b| b);
    if round && (sticky || m & 1 == 1) {
        m += 1;
    }
    let shift = (bits.len() - 53) as i32;
    // m * 2^shift; powi of two is exact, and overflow gives Infinity.
    if shift > 1100 {
        return f64::INFINITY;
    }
    (m as f64) * 2f64.powi(shift)
}

/// `ToIntegerOrInfinity`: truncate toward zero, NaN becomes 0.
pub fn to_integer_or_infinity(x: f64) -> f64 {
    if x.is_nan() {
        0.0
    } else if x.is_infinite() {
        x
    } else {
        let t = x.trunc();
        if t == 0.0 { 0.0 } else { t }
    }
}

/// `ToUint32`: wrap modulo 2^32.
pub fn to_uint32(x: f64) -> u32 {
    if !x.is_finite() {
        return 0;
    }
    let t = x.trunc();
    let m = t.rem_euclid(4_294_967_296.0);
    m as u32
}

/// `ToInt32`: wrap modulo 2^32 into the signed range.
pub fn to_int32(x: f64) -> i32 {
    to_uint32(x) as i32
}

/// `ToUint16`: wrap modulo 2^16.
pub fn to_uint16(x: f64) -> u16 {
    to_uint32(x) as u16
}

#[cfg(test)]
mod tests {
    use super::*;

    fn units(s: &str) -> Vec<u16> {
        s.encode_utf16().collect()
    }

    #[test]
    fn formats_like_the_language() {
        let cases: &[(f64, &str)] = &[
            (0.0, "0"),
            (-0.0, "0"),
            (1.0, "1"),
            (-1.0, "-1"),
            (123.456, "123.456"),
            (0.1 + 0.2, "0.30000000000000004"),
            (1e21, "1e+21"),
            (1e20, "100000000000000000000"),
            (123e18, "123000000000000000000"),
            (1.5e21, "1.5e+21"),
            (0.000001, "0.000001"),
            (0.0000001, "1e-7"),
            (1.25e-7, "1.25e-7"),
            (9007199254740992.0, "9007199254740992"),
            (5e-324, "5e-324"),
            (f64::MAX, "1.7976931348623157e+308"),
            (100.0 / 3.0, "33.333333333333336"),
            (f64::NAN, "NaN"),
            (f64::INFINITY, "Infinity"),
            (f64::NEG_INFINITY, "-Infinity"),
            (0.5, "0.5"),
            (-0.0005, "-0.0005"),
        ];
        for &(x, want) in cases {
            assert_eq!(number_to_string(x), want, "formatting {x:e}");
        }
    }

    #[test]
    fn formatted_numbers_read_back() {
        let mut state = 0x1234_5678_9abc_def0u64;
        for _ in 0..20_000 {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            let x = f64::from_bits(state);
            if !x.is_finite() {
                continue;
            }
            let s = number_to_string(x);
            let back = string_to_number(&units(&s));
            assert_eq!(
                back.to_bits(),
                if x == 0.0 {
                    0.0f64.to_bits()
                } else {
                    x.to_bits()
                },
                "{s}"
            );
        }
    }

    #[test]
    fn parses_like_the_language() {
        let cases: &[(&str, f64)] = &[
            ("", 0.0),
            ("   ", 0.0),
            ("42", 42.0),
            ("  42  ", 42.0),
            ("\u{00A0}\u{FEFF}7\u{2028}", 7.0),
            ("-1.5", -1.5),
            ("+.5", 0.5),
            ("5.", 5.0),
            ("1e3", 1000.0),
            ("1E-2", 0.01),
            ("0x1F", 31.0),
            ("0XfF", 255.0),
            ("0o17", 15.0),
            ("0b101", 5.0),
            ("Infinity", f64::INFINITY),
            ("-Infinity", f64::NEG_INFINITY),
            ("00012", 12.0),
        ];
        for &(s, want) in cases {
            assert_eq!(string_to_number(&units(s)), want, "parsing {s:?}");
        }
        for bad in [
            "abc", "1a", "0x", "-0x10", "inf", "nan", "infinity", "1e", ".", "e5", "1..2", "0b2",
            "١",
        ] {
            assert!(
                string_to_number(&units(bad)).is_nan(),
                "{bad:?} should be NaN"
            );
        }
        assert!(string_to_number(&units("-0")).is_sign_negative());
    }

    #[test]
    fn long_hex_rounds_to_nearest_even() {
        // 2^53 + 1 is a tie between 2^53 and 2^53 + 2: rounds to even (2^53).
        assert_eq!(
            string_to_number(&units("0x20000000000001")),
            9007199254740992.0
        );
        // 2^53 + 3 is a tie between 2^53 + 2 and 2^53 + 4: rounds to even (+4).
        assert_eq!(
            string_to_number(&units("0x20000000000003")),
            9007199254740996.0
        );
        // 2^57 + 17: the dropped bits are past the halfway point, so it rounds up.
        assert_eq!(
            string_to_number(&units("0x200000000000011")),
            144115188075855904.0
        );
        let huge = format!("0x{}", "f".repeat(300));
        assert_eq!(string_to_number(&units(&huge)), f64::INFINITY);
    }

    #[test]
    fn integer_conversions() {
        assert_eq!(to_int32(4294967296.0 + 5.0), 5);
        assert_eq!(to_int32(2147483648.0), i32::MIN);
        assert_eq!(to_int32(-1.0), -1);
        assert_eq!(to_int32(f64::NAN), 0);
        assert_eq!(to_int32(f64::INFINITY), 0);
        assert_eq!(to_int32(-3.7), -3);
        assert_eq!(to_uint32(-1.0), u32::MAX);
        assert_eq!(to_uint16(65537.0), 1);
        assert_eq!(to_integer_or_infinity(-0.5), 0.0);
        assert!(to_integer_or_infinity(-0.5).is_sign_positive());
        assert_eq!(to_integer_or_infinity(f64::NAN), 0.0);
        assert_eq!(to_integer_or_infinity(-2.9), -2.0);
        // 100/3|0 from the probe list.
        assert_eq!(to_int32(100.0 / 3.0), 33);
    }
}
