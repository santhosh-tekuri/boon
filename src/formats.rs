use std::{
    collections::HashMap,
    net::{Ipv4Addr, Ipv6Addr},
};

use once_cell::sync::Lazy;
use percent_encoding::percent_decode_str;
use regex::Regex;
use serde_json::Value;
use url::Url;

pub(crate) type Format = fn(v: &Value) -> bool;

pub(crate) static FORMATS: Lazy<HashMap<&'static str, Format>> = Lazy::new(|| {
    let mut m = HashMap::<&'static str, Format>::new();
    m.insert("regex", is_regex);
    m.insert("ipv4", is_ipv4);
    m.insert("ipv6", is_ipv6);
    m.insert("hostname", is_hostname_value);
    m.insert("idn-hostname", is_idn_hostname);
    m.insert("email", is_email);
    m.insert("date", is_date_value);
    m.insert("time", is_time_value);
    m.insert("date-time", is_date_time);
    m.insert("duration", is_duration);
    m.insert("json-pointer", is_json_pointer_value);
    m.insert("relative-json-pointer", is_relative_json_pointer);
    m.insert("uuid", is_uuid);
    m.insert("uri", is_uri);
    m.insert("iri", is_uri);
    m.insert("uri-reference", is_uri_reference);
    m.insert("iri-reference", is_uri_reference);
    m.insert("uri-template", is_uri_template);
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

fn is_date_value(v: &Value) -> bool {
    let Value::String(s) = v else {
        return true;
    };
    is_date(s)
}

fn matches_char(s: &str, index: usize, ch: char) -> bool {
    s.is_char_boundary(index) && s[index..].starts_with(ch)
}

// see https://datatracker.ietf.org/doc/html/rfc3339#section-5.6
fn is_date(s: &str) -> bool {
    // yyyy-mm-dd
    if s.len() != 10 || !matches_char(s, 4, '-') || !matches_char(s, 7, '-') {
        return false;
    }

    let mut ymd = s.splitn(3, '-').filter_map(|t| t.parse::<usize>().ok());
    let (Some(y), Some(m), Some(d)) = (ymd.next(), ymd.next(), ymd.next()) else {
        return false;
    };

    if !matches!(m, 1..=12) || !matches!(d, 1..=31) {
        return false;
    }

    match m {
        2 => {
            if y % 4 == 0 && (y % 100 != 0 || y % 400 == 0) {
                matches!(d, 1..=29) // leap year
            } else {
                matches!(d, 1..=28)
            }
        }
        4 | 6 | 9 | 11 => d <= 30,
        _ => true,
    }
}

fn is_time_value(v: &Value) -> bool {
    let Value::String(s) = v else {
        return true;
    };
    is_time(s)
}

fn is_time(mut str: &str) -> bool {
    // min: hh:mm:ssZ
    if str.len() < 9 || !matches_char(str, 2, ':') || !matches_char(str, 5, ':') {
        return false;
    }

    // parse hh:mm:ss
    if !str.is_char_boundary(8) {
        return false;
    }
    let mut hms = (str[..8])
        .splitn(3, ':')
        .filter_map(|t| t.parse::<usize>().ok());
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
        str = &str[1..];
        if !matches_char(str, 2, ':') {
            return false;
        }

        let mut zhm = str.splitn(2, ':').filter_map(|t| t.parse::<usize>().ok());
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
    s < 60 || h == 23 && m == 59
}

fn is_date_time(v: &Value) -> bool {
    let Value::String(s) = v else {
        return true;
    };

    // min: yyyy-mm-ddThh:mm:ssZ
    if s.len() < 20 {
        return false;
    }
    if !s.is_char_boundary(10) || !s[10..].starts_with(|c| matches!(c, 't' | 'T')) {
        return false;
    }
    is_date(&s[..10]) && is_time(&s[11..])
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
    for (i, s) in s.split('T').enumerate() {
        let mut s = s;
        if i != 0 && s.is_empty() {
            return false;
        }
        if i > UNITS.len() {
            return false;
        }
        let mut units = UNITS[i];
        while !s.is_empty() {
            let digit_count = s.chars().take_while(|c| c.is_ascii_digit()).count();
            if digit_count == 0 {
                return false;
            }
            s = &s[digit_count..];
            let Some(unit) = s.chars().next() else {
                return false;
            };
            let Some(j) = units.find(unit) else {
                return false;
            };
            units = &units[j + 1..];
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

fn is_idn_hostname(v: &Value) -> bool {
    let Value::String(s) = v else {
        return true;
    };

    let Ok(s) = idna::domain_to_ascii_strict(s) else {
        return false;
    };
    let unicode = idna::domain_to_unicode(&s).0;

    // see https://www.rfc-editor.org/rfc/rfc5892#section-2.6
    {
        static DISALLOWED: [char; 10] = [
            '\u{0640}', //  ARABIC TATWEEL
            '\u{07FA}', //  NKO LAJANYALAN
            '\u{302E}', //  HANGUL SINGLE DOT TONE MARK
            '\u{302F}', //  HANGUL DOUBLE DOT TONE MARK
            '\u{3031}', //  VERTICAL KANA REPEAT MARK
            '\u{3032}', //  VERTICAL KANA REPEAT WITH VOICED SOUND MARK
            '\u{3033}', //  VERTICAL KANA REPEAT MARK UPPER HALF
            '\u{3034}', //  VERTICAL KANA REPEAT WITH VOICED SOUND MARK UPPER HA
            '\u{3035}', //  VERTICAL KANA REPEAT MARK LOWER HALF
            '\u{303B}', //  VERTICAL IDEOGRAPHIC ITERATION MARK
        ];
        if unicode.contains(DISALLOWED) {
            return false;
        }
    }

    // unicode string must not contain "--" in 3rd and 4th position
    // and must not start and end with a '-'
    // see https://www.rfc-editor.org/rfc/rfc5891#section-4.2.3.1
    {
        let count: usize = unicode
            .chars()
            .skip(2)
            .take(2)
            .map(|c| if c == '-' { 1 } else { 0 })
            .sum();
        if count == 2 {
            return false;
        }
    }

    // MIDDLE DOT is allowed between 'l' characters only
    // see https://www.rfc-editor.org/rfc/rfc5892#appendix-A.3
    {
        let middle_dot = '\u{00b7}';
        let mut s = unicode.as_str();
        while let Some(i) = s.find(middle_dot) {
            let prefix = &s[..i];
            let suffix = &s[i + middle_dot.len_utf8()..];
            if !prefix.ends_with('l') || !suffix.ends_with('l') {
                return false;
            }
            s = suffix;
        }
    }

    // Greek KERAIA must be followed by Greek character
    // see https://www.rfc-editor.org/rfc/rfc5892#appendix-A.4
    {
        let keralia = '\u{0375}';
        let greek = '\u{0370}'..='\u{03FF}';
        let mut s = unicode.as_str();
        while let Some(i) = s.find(keralia) {
            let suffix = &s[i + keralia.len_utf8()..];
            if !suffix.starts_with(|c| greek.contains(&c)) {
                return false;
            }
            s = suffix;
        }
    }

    // Hebrew GERESH must be preceded by Hebrew character
    // see https://www.rfc-editor.org/rfc/rfc5892#appendix-A.5
    //
    // Hebrew GERSHAYIM must be preceded by Hebrew character
    // see https://www.rfc-editor.org/rfc/rfc5892#appendix-A.6
    {
        let geresh = '\u{05F3}';
        let gereshayim = '\u{05F4}';
        let hebrew = '\u{0590}'..='\u{05FF}';
        for ch in [geresh, gereshayim] {
            let mut s = unicode.as_str();
            while let Some(i) = s.find(ch) {
                let prefix = &s[..i];
                let suffix = &s[i + ch.len_utf8()..];
                if !prefix.ends_with(|c| hebrew.contains(&c)) {
                    return false;
                }
                s = suffix;
            }
        }
    }

    // KATAKANA MIDDLE DOT must be with Hiragana, Katakana, or Han
    // see https://www.rfc-editor.org/rfc/rfc5892#appendix-A.7
    {
        let katakana_middle_dot = '\u{30FB}';
        let hiragana = '\u{3040}'..='\u{309F}';
        let katakana = '\u{30A0}'..='\u{30FF}';
        let han = '\u{4E00}'..='\u{9FFF}'; // https://en.wikipedia.org/wiki/CJK_Unified_Ideographs_(Unicode_block): is this range correct??
        if unicode.contains(katakana_middle_dot) {
            if unicode.contains(|c| hiragana.contains(&c))
                || unicode.contains(|c| c != katakana_middle_dot && katakana.contains(&c))
                || unicode.contains(|c| han.contains(&c))
            {
                // ok
            } else {
                return false;
            }
        }
    }

    // ARABIC-INDIC DIGITS and Extended Arabic-Indic Digits cannot be mixed
    // see https://www.rfc-editor.org/rfc/rfc5892#appendix-A.8
    // see https://www.rfc-editor.org/rfc/rfc5892#appendix-A.9
    {
        let arabic_indic_digits = '\u{0660}'..='\u{0669}';
        let extended_arabic_indic_digits = '\u{06F0}'..='\u{06F9}';
        if unicode.contains(|c| arabic_indic_digits.contains(&c))
            && unicode.contains(|c| extended_arabic_indic_digits.contains(&c))
        {
            return false;
        }
    }

    // ZERO WIDTH JOINER must be preceded by Virama
    // see https://www.rfc-editor.org/rfc/rfc5892#appendix-A.2
    {
        let zero_width_jointer = '\u{200D}';
        static VIRAMA: [char; 61] = [
            '\u{094D}',
            '\u{09CD}',
            '\u{0A4D}',
            '\u{0ACD}',
            '\u{0B4D}',
            '\u{0BCD}',
            '\u{0C4D}',
            '\u{0CCD}',
            '\u{0D3B}',
            '\u{0D3C}',
            '\u{0D4D}',
            '\u{0DCA}',
            '\u{0E3A}',
            '\u{0EBA}',
            '\u{0F84}',
            '\u{1039}',
            '\u{103A}',
            '\u{1714}',
            '\u{1734}',
            '\u{17D2}',
            '\u{1A60}',
            '\u{1B44}',
            '\u{1BAA}',
            '\u{1BAB}',
            '\u{1BF2}',
            '\u{1BF3}',
            '\u{2D7F}',
            '\u{A806}',
            '\u{A82C}',
            '\u{A8C4}',
            '\u{A953}',
            '\u{A9C0}',
            '\u{AAF6}',
            '\u{ABED}',
            '\u{10A3F}',
            '\u{11046}',
            '\u{1107F}',
            '\u{110B9}',
            '\u{11133}',
            '\u{11134}',
            '\u{111C0}',
            '\u{11235}',
            '\u{112EA}',
            '\u{1134D}',
            '\u{11442}',
            '\u{114C2}',
            '\u{115BF}',
            '\u{1163F}',
            '\u{116B6}',
            '\u{1172B}',
            '\u{11839}',
            '\u{1193D}',
            '\u{1193E}',
            '\u{119E0}',
            '\u{11A34}',
            '\u{11A47}',
            '\u{11A99}',
            '\u{11C3F}',
            '\u{11D44}',
            '\u{11D45}',
            '\u{11D97}',
        ]; // https://www.compart.com/en/unicode/combining/9
        let mut s = unicode.as_str();
        while let Some(i) = s.find(zero_width_jointer) {
            let prefix = &s[..i];
            let suffix = &s[i + zero_width_jointer.len_utf8()..];
            if !prefix.ends_with(VIRAMA) {
                return false;
            }
            s = suffix;
        }
    }

    is_hostname(&s)
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
        let s = &domain[1..domain.len() - 1];
        if let Some(s) = s.strip_prefix("IPv6:") {
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
    let num_digits = s.chars().take_while(|c| c.is_ascii_digit()).count();
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

fn is_uri(v: &Value) -> bool {
    let Value::String(s) = v else {
        return true;
    };
    Url::parse(s).is_ok()
}

fn parse_uri_reference(s: &str) -> Option<Url> {
    match Url::parse(s) {
        Ok(url) => Some(url),
        Err(url::ParseError::RelativeUrlWithoutBase) => match Url::parse("http://temp.com") {
            Ok(url) => {
                if s.contains('\\') {
                    return None;
                }
                Some(url)
            }
            _ => None,
        },
        _ => None,
    }
}

fn is_uri_reference(v: &Value) -> bool {
    let Value::String(s) = v else {
        return true;
    };
    parse_uri_reference(s).is_some()
}

fn is_uri_template(v: &Value) -> bool {
    let Value::String(s) = v else {
        return true;
    };

    let Some(url) = parse_uri_reference(s) else {
        return false;
    };

    let path = url.path();
    // path we got has curly bases percent encoded
    let Ok(path) = percent_decode_str(path).decode_utf8() else {
        return false;
    };

    // ensure curly brackets are not nested and balanced
    for part in path.as_ref().split('/') {
        let mut want = true;
        for got in part
            .chars()
            .filter(|c| matches!(c, '{' | '}'))
            .map(|c| c == '{')
        {
            if got != want {
                return false;
            }
            want = !want;
        }
        if !want {
            // no matching closing bracket
            return false;
        }
    }
    true
}
