use std::{
    collections::HashMap,
    net::{Ipv4Addr, Ipv6Addr},
};

use chrono::NaiveDate;
use once_cell::sync::Lazy;
use serde_json::Value;

use crate::Format;

pub(crate) static FORMATS: Lazy<HashMap<&'static str, Format>> = Lazy::new(|| {
    let mut m = HashMap::<&'static str, Format>::new();
    m.insert("ipv4", is_ipv4);
    m.insert("ipv6", is_ipv6);
    m.insert("hostname", is_hostname_value);
    m.insert("email", is_email);
    m.insert("date", is_date);
    m.insert("json-pointer", is_json_pointer_value);
    m
});

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

fn is_hostname_value(v: &Value) -> bool {
    let Value::String(s) = v else {
        return true;
    };
    return is_hostname(s);
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
    return is_json_pointer(s);
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
