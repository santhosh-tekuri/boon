use std::{
    collections::HashMap,
    net::{Ipv4Addr, Ipv6Addr},
};

use chrono::NaiveDate;
use once_cell::sync::Lazy;
use regex::Regex;
use serde_json::Value;

use crate::Format;

pub(crate) static FORMATS: Lazy<HashMap<&'static str, Format>> = Lazy::new(|| {
    let mut m = HashMap::<&'static str, Format>::new();
    m.insert("regex", is_regex);
    m.insert("ipv4", is_ipv4);
    m.insert("ipv6", is_ipv6);
    m.insert("hostname", is_hostname_value);
    m.insert("email", is_email);
    m.insert("date", is_date);
    m.insert("time", is_time_value);
    m.insert("duration", is_duration);
    m.insert("json-pointer", is_json_pointer_value);
    m.insert("relative-json-pointer", is_relative_json_pointer);
    m.insert("uuid", is_uuid);
    m
});

fn is_regex(v: &Value) -> bool {
    let Value::String(s) = v else {
        return true;
    };
    Regex::new(s).is_ok()
}

fn is_ipv4(v: &Value) -> bool {
    let Value::String(s) = v else {
        return true;
    };
    s.parse::<Ipv4Addr>().is_ok()
}

fn is_ipv6(v: &Value) -> bool {
    let Value::String(s) = v else {
        return true;
    };
    s.parse::<Ipv6Addr>().is_ok()
}

// see https://datatracker.ietf.org/doc/html/rfc3339#section-5.6
fn is_date(v: &Value) -> bool {
    let Value::String(s) = v else {
        return true;
    };
    let Ok(d) = NaiveDate::parse_from_str(s, "%Y-%m-%d") else {
        return false;
    };

    // to ensure zero padded
    &d.format("%Y-%m-%d").to_string() == s
}

fn is_time_value(v: &Value) -> bool {
    let Value::String(s) = v else {
        return true;
    };
    is_time(s)
}

fn is_time(mut str: &str) -> bool {
    if str.len() < 9 {
        return false;
    }

    // parse hh:mm:ss
    if !str.is_char_boundary(8) {
        return false;
    }
    let mut hms = (&str[..8])
        .splitn(3, ':')
        .map(|t| t.parse::<usize>().ok())
        .flatten();
    let (Some(mut h), Some(mut m), Some(s)) = (hms.next(), hms.next(), hms.next()) else {
        return false;
    };
    if h > 23 || m > 59 || s > 60 {
        return false;
    }
    str = &str[8..];

    // parse sec-frac if present
    if let Some(rem) = str.strip_prefix('.') {
        let n_digits = rem.chars().take_while(|c| c.is_ascii_digit()).count();
        if n_digits == 0 {
            return false;
        }
        str = &rem[n_digits..];
    }

    if str != "z" && str != "Z" {
        // parse time-numoffset
        if str.len() != 6 {
            return false;
        }
        let sign: isize = match str.chars().next() {
            Some('+') => -1,
            Some('-') => 1,
            _ => return false,
        };
        let mut zhm = str[1..]
            .splitn(2, ':')
            .map(|t| t.parse::<usize>().ok())
            .flatten();
        let (Some(zh), Some(zm)) = (zhm.next(), zhm.next()) else {
            return false;
        };
        if zh > 23 || zm > 59 {
            return false;
        }

        // apply timezone
        let mut hm = (h * 60 + m) as isize + sign * (zh * 60 + zm) as isize;
        if hm < 0 {
            hm += 24 * 60;
            debug_assert!(hm >= 0);
        }
        let hm = hm as usize;
        (h, m) = (hm / 60, hm % 60);
    }

    // check leapsecond
    return s < 60 || h == 23 && m == 59;
}

// see https://datatracker.ietf.org/doc/html/rfc3339#appendix-A
fn is_duration(v: &Value) -> bool {
    let Value::String(s) = v else {
        return true;
    };

    // must start with 'P'
    let Some(s) = s.strip_prefix('P') else {
        return false;
    };
    if s.is_empty() {
        return false;
    }

    // dur-week
    if let Some(s) = s.strip_suffix('W') {
        return s.chars().all(|c| c.is_ascii_digit());
    }

    static UNITS: [&str; 2] = ["YMD", "HMS"];
    let mut i = 0;
    for s in s.split('T') {
        let mut s = s;
        if i != 0 && s.is_empty() {
            return false;
        }
        if i > UNITS.len() {
            return false;
        }
        let mut units = UNITS[i];
        i += 1;
        while !s.is_empty() {
            let digit_count = s.chars().take_while(|c| c.is_ascii_digit()).count();
            if digit_count == 0 {
                return false;
            }
            s = &s[digit_count..];
            let Some(unit) = s.chars().next() else {
                return false;
            };
            let Some(i) = units.find(unit) else {
                return false;
            };
            units = &units[i + 1..];
            s = &s[1..];
        }
    }

    true
}

fn is_hostname_value(v: &Value) -> bool {
    let Value::String(s) = v else {
        return true;
    };
    is_hostname(s)
}

// see https://en.wikipedia.org/wiki/Hostname#Restrictions_on_valid_host_names
fn is_hostname(mut s: &str) -> bool {
    // entire hostname (including the delimiting dots but not a trailing dot) has a maximum of 253 ASCII characters
    s = s.strip_suffix('.').unwrap_or(s);
    if s.len() > 253 {
        return false;
    }

    // Hostnames are composed of series of labels concatenated with dots, as are all domain names
    for label in s.split('.') {
        // Each label must be from 1 to 63 characters long
        if !matches!(label.len(), 1..=63) {
            return false;
        }

        // labels must not start or end with a hyphen
        if label.starts_with('-') || label.ends_with('-') {
            return false;
        }

        // labels may contain only the ASCII letters 'a' through 'z' (in a case-insensitive manner),
        // the digits '0' through '9', and the hyphen ('-')
        if !label
            .chars()
            .all(|c| matches!(c, 'a'..='z' | 'A'..='Z' | '0'..='9' | '-'))
        {
            return false;
        }
    }

    true
}

// see https://en.wikipedia.org/wiki/Email_address
fn is_email(v: &Value) -> bool {
    let Value::String(s) = v else {
        return true;
    };

    // entire email address to be no more than 254 characters long
    if s.len() > 254 {
        return false;
    }

    // email address is generally recognized as having two parts joined with an at-sign
    let Some(at) = s.rfind('@') else {
        return false;
    };
    let (local, domain) = (&s[..at], &s[at + 1..]);

    // local part may be up to 64 characters long
    if local.len() > 64 {
        return false;
    }

    if local.starts_with('"') && local.ends_with('"') {
        // quoted
        let local = &local[1..local.len() - 1];
        if local.contains('\\') || local.contains('"') {
            return false;
        }
    } else {
        // unquoted

        // must not start or end with a dot
        if local.starts_with('.') || local.ends_with('.') {
            return false;
        }

        // consecutive dots not allowed
        if local.contains("..") {
            return false;
        }

        // check allowd chars
        if !local.chars().all(|c| {
            matches!(c, 'a'..='z' | 'A'..='Z' | '0'..='9') || ".!#$%&'*+-/=?^_`{|}~".contains(c)
        }) {
            return false;
        }
    }

    // domain if enclosed in brackets, must match an IP address
    if domain.starts_with('[') && domain.ends_with(']') {
        let ip = &domain[1..domain.len() - 1];
        if let Some(s) = ip.strip_prefix("IPv6:") {
            return s.parse::<Ipv6Addr>().is_ok();
        }
        return s.parse::<Ipv4Addr>().is_ok();
    }

    // domain must match the requirements for a hostname
    if !is_hostname(domain) {
        return false;
    }

    true
}

fn is_json_pointer_value(v: &Value) -> bool {
    let Value::String(s) = v else {
        return true;
    };
    is_json_pointer(s)
}

// see https://www.rfc-editor.org/rfc/rfc6901#section-3
fn is_json_pointer(s: &str) -> bool {
    if s.is_empty() {
        return true;
    }
    if !s.starts_with('/') {
        return false;
    }
    for token in s.split('/').skip(1) {
        let mut chars = token.chars();
        while let Some(ch) = chars.next() {
            if ch == '~' {
                if !matches!(chars.next(), Some('0' | '1')) {
                    return false;
                }
            } else if !matches!(ch, '\x00'..='\x2E' | '\x30'..='\x7D' | '\x7F'..='\u{10FFFF}') {
                return false;
            }
        }
    }
    true
}

// see https://tools.ietf.org/html/draft-handrews-relative-json-pointer-01#section-3
fn is_relative_json_pointer(v: &Value) -> bool {
    let Value::String(s) = v else {
        return true;
    };

    // start with non-negative-integer
    let num_digits = s.chars().take_while(|c| matches!(c, '0'..='9')).count();
    if num_digits == 0 || (num_digits > 1 && s.starts_with('0')) {
        return false;
    }
    let s = &s[num_digits..];

    // followed by either json-pointer or '#'
    s == "#" || is_json_pointer(s)
}

// see https://datatracker.ietf.org/doc/html/rfc4122#page-4
fn is_uuid(v: &Value) -> bool {
    let Value::String(s) = v else {
        return true;
    };

    static HEX_GROUPS: [usize; 5] = [8, 4, 4, 4, 12];
    let mut i = 0;
    for group in s.split('-') {
        if i > HEX_GROUPS.len()
            || group.len() != HEX_GROUPS[i]
            || !group.chars().all(|c| c.is_ascii_hexdigit())
        {
            return false;
        }
        i += 1;
    }
    i == HEX_GROUPS.len()
}
