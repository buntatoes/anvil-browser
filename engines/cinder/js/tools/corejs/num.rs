// SPDX-License-Identifier: LicenseRef-PolyForm-Noncommercial-1.0.0-OR-Commercial
// Copyright (c) 2026 Buntato
//! Number to string and string to number, following the language spec
//! (Number::toString with radix 10, and StringToNumber).

/// `Number::toString(x)` for radix 10.
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
    if x < 0.0 {
        return format!("-{}", number_to_string(-x));
    }
    // The shortest digits that round-trip, as `d.ddde±x`.
    let sci = format!("{x:e}");
    let (mant, exp) = sci.split_once('e').unwrap_or((&sci, "0"));
    let digits: String = mant.chars().filter(|c| *c != '.').collect();
    let e: i32 = exp.parse().unwrap_or(0);
    let k = digits.len() as i32;
    let n = e + 1;
    if k <= n && n <= 21 {
        let mut s = digits;
        for _ in 0..(n - k) {
            s.push('0');
        }
        s
    } else if 0 < n && n <= 21 {
        format!("{}.{}", &digits[..n as usize], &digits[n as usize..])
    } else if -6 < n && n <= 0 {
        let mut s = "0.".to_string();
        for _ in 0..(-n) {
            s.push('0');
        }
        s.push_str(&digits);
        s
    } else {
        let sign = if n - 1 < 0 { '-' } else { '+' };
        let ex = (n - 1).abs();
        if k == 1 {
            format!("{digits}e{sign}{ex}")
        } else {
            format!("{}.{}e{sign}{ex}", &digits[..1], &digits[1..])
        }
    }
}

fn is_js_space(u: u16) -> bool {
    matches!(
        u,
        0x09 | 0x0A | 0x0B | 0x0C | 0x0D | 0x20 | 0xA0 | 0x1680 | 0x2000
            ..=0x200A | 0x2028 | 0x2029 | 0x202F | 0x205F | 0x3000 | 0xFEFF
    )
}

/// StringToNumber over UTF-16 code units.
pub fn string_to_number(units: &[u16]) -> f64 {
    let mut a = 0;
    let mut b = units.len();
    while a < b && is_js_space(units[a]) {
        a += 1;
    }
    while b > a && is_js_space(units[b - 1]) {
        b -= 1;
    }
    let s: String = match String::from_utf16(&units[a..b]) {
        Ok(s) => s,
        Err(_) => return f64::NAN,
    };
    if s.is_empty() {
        return 0.0;
    }
    let bytes = s.as_bytes();
    if bytes.len() > 2 && bytes[0] == b'0' {
        let radix = match bytes[1] {
            b'x' | b'X' => 16,
            b'o' | b'O' => 8,
            b'b' | b'B' => 2,
            _ => 0,
        };
        if radix != 0 {
            let mut v = 0f64;
            for c in s[2..].chars() {
                match c.to_digit(radix) {
                    Some(d) => v = v * radix as f64 + d as f64,
                    None => return f64::NAN,
                }
            }
            return v;
        }
    }
    let (neg, body) = match bytes[0] {
        b'+' => (false, &s[1..]),
        b'-' => (true, &s[1..]),
        _ => (false, &s[..]),
    };
    let v = if body == "Infinity" {
        f64::INFINITY
    } else {
        // StrUnsignedDecimalLiteral: digits [. digits] [e[+-]digits], with
        // at least one digit before or after the point.
        let b = body.as_bytes();
        let mut i = 0;
        let mut digits = 0;
        while i < b.len() && b[i].is_ascii_digit() {
            i += 1;
            digits += 1;
        }
        if i < b.len() && b[i] == b'.' {
            i += 1;
            while i < b.len() && b[i].is_ascii_digit() {
                i += 1;
                digits += 1;
            }
        }
        if digits == 0 {
            return f64::NAN;
        }
        if i < b.len() && (b[i] == b'e' || b[i] == b'E') {
            i += 1;
            if i < b.len() && (b[i] == b'+' || b[i] == b'-') {
                i += 1;
            }
            let start = i;
            while i < b.len() && b[i].is_ascii_digit() {
                i += 1;
            }
            if i == start {
                return f64::NAN;
            }
        }
        if i != b.len() {
            return f64::NAN;
        }
        let mut t = body.to_string();
        if t.starts_with('.') {
            t.insert(0, '0');
        }
        t = t.replace(".e", ".0e").replace(".E", ".0E");
        if t.ends_with('.') {
            t.push('0');
        }
        match t.parse::<f64>() {
            Ok(v) => v,
            Err(_) => return f64::NAN,
        }
    };
    if neg { -v } else { v }
}

#[cfg(test)]
mod num_tests {
    use super::*;

    #[test]
    fn to_string_follows_the_spec() {
        let cases = [
            (0.0, "0"),
            (-0.0, "0"),
            (1.0, "1"),
            (-1.5, "-1.5"),
            (0.1, "0.1"),
            (1e21, "1e+21"),
            (1e20, "100000000000000000000"),
            (123456789012345680000.0, "123456789012345680000"),
            (1e-6, "0.000001"),
            (1e-7, "1e-7"),
            (1.5e-7, "1.5e-7"),
            (0.1 + 0.2, "0.30000000000000004"),
            (5e-324, "5e-324"),
            (1.7976931348623157e308, "1.7976931348623157e+308"),
            (f64::NAN, "NaN"),
            (f64::NEG_INFINITY, "-Infinity"),
        ];
        for (v, s) in cases {
            assert_eq!(number_to_string(v), s, "{v}");
        }
    }

    #[test]
    fn from_string_follows_the_spec() {
        let u = |s: &str| s.encode_utf16().collect::<Vec<u16>>();
        assert_eq!(string_to_number(&u("  12  ")), 12.0);
        assert_eq!(string_to_number(&u("")), 0.0);
        assert_eq!(string_to_number(&u("0x1f")), 31.0);
        assert_eq!(string_to_number(&u("-Infinity")), f64::NEG_INFINITY);
        assert_eq!(string_to_number(&u(".5")), 0.5);
        assert_eq!(string_to_number(&u("5.")), 5.0);
        assert_eq!(string_to_number(&u("1e3")), 1000.0);
        assert!(string_to_number(&u("inf")).is_nan());
        assert!(string_to_number(&u("1_0")).is_nan());
        assert!(string_to_number(&u("-0x1")).is_nan());
        assert!(string_to_number(&u("1e")).is_nan());
        assert!(string_to_number(&u(".")).is_nan());
    }
}
