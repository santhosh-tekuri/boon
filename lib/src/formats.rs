use std::{
    collections::HashMap,
    error::Error,
    net::{Ipv4Addr, Ipv6Addr},
};

use once_cell::sync::Lazy;
use percent_encoding::percent_decode_str;
use serde_json::Value;
use url::Url;

use crate::ecma;

/// Defines format for `format` keyword.
#[derive(Clone, Copy)]
pub struct Format {
    /// Name of the format
    pub name: &'static str,

    /// validates given value.
    pub func: fn(v: &Value) -> Result<(), Box<dyn Error>>,
}

pub(crate) static FORMATS: Lazy<HashMap<&'static str, Format>> = Lazy::new(|| {
    let mut m = HashMap::<&'static str, Format>::new();
    let mut register = |name, func| m.insert(name, Format { name, func });
    register("regex", validate_regex);
    register("ipv4", validate_ipv4);
    register("ipv6", validate_ipv6);
    register("hostname", validate_hostname);
    register("idn-hostname", validate_idn_hostname);
    register("email", validate_email);
    register("idn-email", validate_idn_email);
    register("date", validate_date);
    register("time", validate_time);
    register("date-time", validate_date_time);
    register("duration", validate_duration);
    register("period", validate_period);
    register("json-pointer", validate_json_pointer);
    register("relative-json-pointer", validate_relative_json_pointer);
    register("uuid", validate_uuid);
    register("uri", validate_uri);
    register("iri", validate_iri);
    register("uri-reference", validate_uri_reference);
    register("iri-reference", validate_iri_reference);
    register("uri-template", validate_uri_template);
    m
});

pub fn validate_regex(v: &Value) -> Result<(), Box<dyn Error>> {
    let Value::String(s) = v else {
        return Ok(());
    };
    ecma::convert(s).map(|_| ())
}

pub fn validate_ipv4(v: &Value) -> Result<(), Box<dyn Error>> {
    let Value::String(s) = v else {
        return Ok(());
    };
    s.parse::<Ipv4Addr>()?;
    Ok(())
}

pub fn validate_ipv6(v: &Value) -> Result<(), Box<dyn Error>> {
    let Value::String(s) = v else {
        return Ok(());
    };
    s.parse::<Ipv6Addr>()?;
    Ok(())
}

pub fn validate_date(v: &Value) -> Result<(), Box<dyn Error>> {
    let Value::String(s) = v else {
        return Ok(());
    };
    check_date(s)
}

fn matches_char(s: &str, index: usize, ch: char) -> bool {
    s.is_char_boundary(index) && s[index..].starts_with(ch)
}

// see https://datatracker.ietf.org/doc/html/rfc3339#section-5.6
fn check_date(s: &str) -> Result<(), Box<dyn Error>> {
    // yyyy-mm-dd
    if s.len() != 10 {
        Err("must be 10 characters long")?;
    }
    if !matches_char(s, 4, '-') || !matches_char(s, 7, '-') {
        Err("missing hyphen in correct place")?;
    }

    let mut ymd = s.splitn(3, '-').filter_map(|t| t.parse::<usize>().ok());
    let (Some(y), Some(m), Some(d)) = (ymd.next(), ymd.next(), ymd.next()) else {
        return Err("non-positive year/month/day")?;
    };

    if !matches!(m, 1..=12) {
        Err(format!("{m} months in year"))?;
    }
    if !matches!(d, 1..=31) {
        Err(format!("{d} days in month"))?;
    }

    match m {
        2 => {
            let mut feb_days = 28;
            if y % 4 == 0 && (y % 100 != 0 || y % 400 == 0) {
                feb_days += 1; // leap year
            };
            if d > feb_days {
                Err(format!("february has {feb_days} days only"))?;
            }
        }
        4 | 6 | 9 | 11 => {
            if d > 30 {
                Err("month has 30 days only")?;
            }
        }
        _ => {}
    }
    Ok(())
}

pub fn validate_time(v: &Value) -> Result<(), Box<dyn Error>> {
    let Value::String(s) = v else {
        return Ok(());
    };
    check_time(s)
}

fn check_time(mut str: &str) -> Result<(), Box<dyn Error>> {
    // min: hh:mm:ssZ
    if str.len() < 9 {
        Err("less than 9 characters long")?;
    }
    if !matches_char(str, 2, ':') || !matches_char(str, 5, ':') {
        Err("missing colon in correct place")?;
    }

    // parse hh:mm:ss
    if !str.is_char_boundary(8) {
        Err("contains non-ascii char")?;
    }
    let mut hms = (str[..8])
        .splitn(3, ':')
        .filter_map(|t| t.parse::<usize>().ok());
    let (Some(mut h), Some(mut m), Some(s)) = (hms.next(), hms.next(), hms.next()) else {
        return Err("non-positive hour/min/sec")?;
    };
    if h > 23 || m > 59 || s > 60 {
        Err("hour/min/sec out of range")?;
    }
    str = &str[8..];

    // parse sec-frac if present
    if let Some(rem) = str.strip_prefix('.') {
        let n_digits = rem.chars().take_while(char::is_ascii_digit).count();
        if n_digits == 0 {
            Err("no digits in second fraction")?;
        }
        str = &rem[n_digits..];
    }

    if str != "z" && str != "Z" {
        // parse time-numoffset
        if str.len() != 6 {
            Err("offset must be 6 characters long")?;
        }
        let sign: isize = match str.chars().next() {
            Some('+') => -1,
            Some('-') => 1,
            _ => return Err("offset must begin with plus/minus")?,
        };
        str = &str[1..];
        if !matches_char(str, 2, ':') {
            Err("missing colon in offset at correct place")?;
        }

        let mut zhm = str.splitn(2, ':').filter_map(|t| t.parse::<usize>().ok());
        let (Some(zh), Some(zm)) = (zhm.next(), zhm.next()) else {
            return Err("non-positive hour/min in offset")?;
        };
        if zh > 23 || zm > 59 {
            Err("hour/min in offset out of range")?;
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
    if !(s < 60 || h == 23 && m == 59) {
        Err("invalid leap second")?
    }
    Ok(())
}

pub fn validate_date_time(v: &Value) -> Result<(), Box<dyn Error>> {
    let Value::String(s) = v else {
        return Ok(());
    };
    check_date_time(s)
}

fn check_date_time(s: &str) -> Result<(), Box<dyn Error>> {
    // min: yyyy-mm-ddThh:mm:ssZ
    if s.len() < 20 {
        Err("less than 20 characters long")?;
    }
    if !s.is_char_boundary(10) || !s[10..].starts_with(|c| matches!(c, 't' | 'T')) {
        Err("11th character must be t or T")?;
    }
    if let Err(e) = check_date(&s[..10]) {
        Err(format!("invalid date element: {e}"))?;
    }
    if let Err(e) = check_time(&s[11..]) {
        Err(format!("invalid time element: {e}"))?;
    }
    Ok(())
}

pub fn validate_duration(v: &Value) -> Result<(), Box<dyn Error>> {
    let Value::String(s) = v else {
        return Ok(());
    };
    check_duration(s)
}

// see https://datatracker.ietf.org/doc/html/rfc3339#appendix-A
fn check_duration(s: &str) -> Result<(), Box<dyn Error>> {
    // must start with 'P'
    let Some(s) = s.strip_prefix('P') else {
        return Err("must start with P")?;
    };
    if s.is_empty() {
        Err("nothing after P")?;
    }

    // dur-week
    if let Some(s) = s.strip_suffix('W') {
        if s.is_empty() {
            Err("no number in week")?;
        }
        if !s.chars().all(|c| c.is_ascii_digit()) {
            Err("invalid week")?;
        }
        return Ok(());
    }

    static UNITS: [&str; 2] = ["YMD", "HMS"];
    for (i, s) in s.split('T').enumerate() {
        let mut s = s;
        if i != 0 && s.is_empty() {
            Err("no time elements")?;
        }
        let Some(mut units) = UNITS.get(i).cloned() else {
            return Err("more than one T")?;
        };
        while !s.is_empty() {
            let digit_count = s.chars().take_while(char::is_ascii_digit).count();
            if digit_count == 0 {
                Err("missing number")?;
            }
            s = &s[digit_count..];
            let Some(unit) = s.chars().next() else {
                return Err("missing unit")?;
            };
            let Some(j) = units.find(unit) else {
                if UNITS[i].contains(unit) {
                    return Err(format!("unit {unit} out of order"))?;
                }
                return Err(format!("invalid unit {unit}"))?;
            };
            units = &units[j + 1..];
            s = &s[1..];
        }
    }

    Ok(())
}

// see https://datatracker.ietf.org/doc/html/rfc3339#appendix-A
pub fn validate_period(v: &Value) -> Result<(), Box<dyn Error>> {
    let Value::String(s) = v else {
        return Ok(());
    };

    let Some(slash) = s.find('/') else {
        return Err("missing slash")?;
    };

    let (start, end) = (&s[..slash], &s[slash + 1..]);
    if start.starts_with('P') {
        if let Err(e) = check_duration(start) {
            Err(format!("invalid start duration: {e}"))?
        }
        if let Err(e) = check_date_time(end) {
            Err(format!("invalid end date-time: {e}"))?
        }
    } else {
        if let Err(e) = check_date_time(start) {
            Err(format!("invalid start date-time: {e}"))?
        }
        if end.starts_with('P') {
            if let Err(e) = check_duration(end) {
                Err(format!("invalid end duration: {e}"))?;
            }
        } else if let Err(e) = check_date_time(end) {
            Err(format!("invalid end date-time: {e}"))?;
        }
    }
    Ok(())
}

pub fn validate_hostname(v: &Value) -> Result<(), Box<dyn Error>> {
    let Value::String(s) = v else {
        return Ok(());
    };
    check_hostname(s)
}

// see https://en.wikipedia.org/wiki/Hostname#Restrictions_on_valid_host_names
fn check_hostname(mut s: &str) -> Result<(), Box<dyn Error>> {
    // entire hostname (including the delimiting dots but not a trailing dot) has a maximum of 253 ASCII characters
    s = s.strip_suffix('.').unwrap_or(s);
    if s.len() > 253 {
        Err("more than 253 characters long")?
    }

    // Hostnames are composed of series of labels concatenated with dots, as are all domain names
    for label in s.split('.') {
        // Each label must be from 1 to 63 characters long
        if !matches!(label.len(), 1..=63) {
            Err("label must be 1 to 63 characters long")?;
        }

        // labels must not start or end with a hyphen
        if label.starts_with('-') {
            Err("label starts with hyphen")?;
        }

        if label.ends_with('-') {
            Err("label ends with hyphen")?;
        }

        // labels may contain only the ASCII letters 'a' through 'z' (in a case-insensitive manner),
        // the digits '0' through '9', and the hyphen ('-')
        if let Some(ch) = label
            .chars()
            .find(|c| !matches!(c, 'a'..='z' | 'A'..='Z' | '0'..='9' | '-'))
        {
            Err(format!("invalid character {ch:?}"))?; // todo: tell which char is invalid
        }
    }

    Ok(())
}

pub fn validate_idn_hostname(v: &Value) -> Result<(), Box<dyn Error>> {
    let Value::String(s) = v else {
        return Ok(());
    };
    check_idn_hostname(s)
}

fn check_idn_hostname(s: &str) -> Result<(), Box<dyn Error>> {
    let s = idna::domain_to_ascii_strict(s)?;
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
            Err("contains disallowed character")?;
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
            Err("unicode string must not contain '--' in 3rd and 4th position")?;
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
                Err("MIDDLE DOT is allowed between 'l' characters only")?;
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
                Err("Greek KERAIA must be followed by Greek character")?;
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
                    if i == 0 {
                        Err("Hebrew GERESH must be preceded by Hebrew character")?;
                    } else {
                        Err("Hebrew GERESHYIM must be preceded by Hebrew character")?;
                    }
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
                Err("KATAKANA MIDDLE DOT must be with Hiragana, Katakana, or Han")?;
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
            Err("ARABIC-INDIC DIGITS and Extended Arabic-Indic Digits cannot be mixed")?;
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
                Err("ZERO WIDTH JOINER must be preceded by Virama")?;
            }
            s = suffix;
        }
    }

    check_hostname(&s)
}

pub fn validate_email(v: &Value) -> Result<(), Box<dyn Error>> {
    let Value::String(s) = v else {
        return Ok(());
    };
    check_email(s)
}

// see https://en.wikipedia.org/wiki/Email_address
fn check_email(s: &str) -> Result<(), Box<dyn Error>> {
    // entire email address to be no more than 254 characters long
    if s.len() > 254 {
        Err("more than 254 characters long")?;
    }

    // email address is generally recognized as having two parts joined with an at-sign
    let Some(at) = s.rfind('@') else {
        return Err("missing @")?;
    };
    let (local, domain) = (&s[..at], &s[at + 1..]);

    // local part may be up to 64 characters long
    if local.len() > 64 {
        Err("local part more than 64 characters long")?;
    }

    if local.starts_with('"') && local.ends_with('"') {
        // quoted
        let local = &local[1..local.len() - 1];
        if local.contains(|c| matches!(c, '\\' | '"')) {
            Err("backslash and quote not allowed within quoted local part")?;
        }
    } else {
        // unquoted

        if local.starts_with('.') {
            Err("starts with dot")?;
        }
        if local.ends_with('.') {
            Err("ends with dot")?;
        }

        // consecutive dots not allowed
        if local.contains("..") {
            Err("consecutive dots")?;
        }

        // check allowd chars
        if let Some(ch) = local
            .chars()
            .find(|c| !(c.is_ascii_alphanumeric() || ".!#$%&'*+-/=?^_`{|}~".contains(*c)))
        {
            Err(format!("invalid character {ch:?}"))?;
        }
    }

    // domain if enclosed in brackets, must match an IP address
    if domain.starts_with('[') && domain.ends_with(']') {
        let s = &domain[1..domain.len() - 1];
        if let Some(s) = s.strip_prefix("IPv6:") {
            if let Err(e) = s.parse::<Ipv6Addr>() {
                Err(format!("invalid ipv6 address: {e}"))?;
            }
            return Ok(());
        }
        if let Err(e) = s.parse::<Ipv4Addr>() {
            Err(format!("invalid ipv4 address: {e}"))?;
        }
        return Ok(());
    }

    // domain must match the requirements for a hostname
    if let Err(e) = check_hostname(domain) {
        Err(format!("invalid domain: {e}"))?;
    }

    Ok(())
}

pub fn validate_idn_email(v: &Value) -> Result<(), Box<dyn Error>> {
    let Value::String(s) = v else {
        return Ok(());
    };

    let Some(at) = s.rfind('@') else {
        return Err("missing @")?;
    };
    let (local, domain) = (&s[..at], &s[at + 1..]);

    let local = idna::domain_to_ascii_strict(local)?;
    let domain = idna::domain_to_ascii_strict(domain)?;
    if let Err(e) = check_idn_hostname(&domain) {
        Err(format!("invalid domain: {e}"))?;
    }
    check_email(&format!("{local}@{domain}"))
}

pub fn validate_json_pointer(v: &Value) -> Result<(), Box<dyn Error>> {
    let Value::String(s) = v else {
        return Ok(());
    };
    check_json_pointer(s)
}

// see https://www.rfc-editor.org/rfc/rfc6901#section-3
fn check_json_pointer(s: &str) -> Result<(), Box<dyn Error>> {
    if s.is_empty() {
        return Ok(());
    }
    if !s.starts_with('/') {
        Err("not starting with slash")?;
    }
    for token in s.split('/').skip(1) {
        let mut chars = token.chars();
        while let Some(ch) = chars.next() {
            if ch == '~' {
                if !matches!(chars.next(), Some('0' | '1')) {
                    Err("~ must be followed by 0 or 1")?;
                }
            } else if !matches!(ch, '\x00'..='\x2E' | '\x30'..='\x7D' | '\x7F'..='\u{10FFFF}') {
                Err("contains disallowed character")?;
            }
        }
    }
    Ok(())
}

// see https://tools.ietf.org/html/draft-handrews-relative-json-pointer-01#section-3
pub fn validate_relative_json_pointer(v: &Value) -> Result<(), Box<dyn Error>> {
    let Value::String(s) = v else {
        return Ok(());
    };

    // start with non-negative-integer
    let num_digits = s.chars().take_while(char::is_ascii_digit).count();
    if num_digits == 0 {
        Err("must start with non-negative integer")?;
    }
    if num_digits > 1 && s.starts_with('0') {
        Err("starts with zero")?;
    }
    let s = &s[num_digits..];

    // followed by either json-pointer or '#'
    if s == "#" {
        return Ok(());
    }
    if let Err(e) = check_json_pointer(s) {
        Err(format!("invalid json-pointer element: {e}"))?;
    }
    Ok(())
}

// see https://datatracker.ietf.org/doc/html/rfc4122#page-4
pub fn validate_uuid(v: &Value) -> Result<(), Box<dyn Error>> {
    let Value::String(s) = v else {
        return Ok(());
    };

    static HEX_GROUPS: [usize; 5] = [8, 4, 4, 4, 12];
    let mut i = 0;
    for group in s.split('-') {
        if i > HEX_GROUPS.len() {
            Err("more than 5 elements")?;
        }
        if group.len() != HEX_GROUPS[i] {
            Err(format!(
                "element {} must be {} characters long",
                i + 1,
                HEX_GROUPS[i]
            ))?;
        }
        if let Some(ch) = group.chars().find(|c| !c.is_ascii_hexdigit()) {
            Err(format!("non-hex character {ch:?}"))?;
        }
        i += 1;
    }
    if i != HEX_GROUPS.len() {
        Err("must have 5 elements")?;
    }
    Ok(())
}

pub fn validate_uri(v: &Value) -> Result<(), Box<dyn Error>> {
    let Value::String(s) = v else {
        return Ok(());
    };
    if fluent_uri::Uri::parse(s)?.is_relative() {
        Err("relative url")?;
    };
    Ok(())
}

pub fn validate_iri(v: &Value) -> Result<(), Box<dyn Error>> {
    let Value::String(s) = v else {
        return Ok(());
    };
    match Url::parse(s) {
        Ok(_) => Ok(()),
        Err(url::ParseError::RelativeUrlWithoutBase) => Err("relative url")?,
        Err(e) => Err(e)?,
    }
}

static TEMP_URL: Lazy<Url> = Lazy::new(|| Url::parse("http://temp.com").unwrap());

fn parse_uri_reference(s: &str) -> Result<Url, Box<dyn Error>> {
    if s.contains('\\') {
        Err("contains \\\\")?;
    }
    Ok(TEMP_URL.join(s)?)
}

pub fn validate_uri_reference(v: &Value) -> Result<(), Box<dyn Error>> {
    let Value::String(s) = v else {
        return Ok(());
    };
    fluent_uri::Uri::parse(s)?;
    Ok(())
}

pub fn validate_iri_reference(v: &Value) -> Result<(), Box<dyn Error>> {
    let Value::String(s) = v else {
        return Ok(());
    };
    parse_uri_reference(s)?;
    Ok(())
}

pub fn validate_uri_template(v: &Value) -> Result<(), Box<dyn Error>> {
    let Value::String(s) = v else {
        return Ok(());
    };

    let url = parse_uri_reference(s)?;

    let path = url.path();
    // path we got has curly bases percent encoded
    let path = percent_decode_str(path).decode_utf8()?;

    // ensure curly brackets are not nested and balanced
    for part in path.as_ref().split('/') {
        let mut want = true;
        for got in part
            .chars()
            .filter(|c| matches!(c, '{' | '}'))
            .map(|c| c == '{')
        {
            if got != want {
                Err("nested curly brackets")?;
            }
            want = !want;
        }
        if !want {
            Err("no matching closing bracket")?
        }
    }
    Ok(())
}
