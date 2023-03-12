use std::{collections::HashMap, error::Error};

use base64::Engine;
use once_cell::sync::Lazy;
use serde::de::IgnoredAny;
use serde_json::Value;

// decoders --
pub(crate) type Decoder = fn(s: &str) -> Result<Vec<u8>, Box<dyn Error>>;

pub(crate) static DECODERS: Lazy<HashMap<&'static str, Decoder>> = Lazy::new(|| {
    let mut m = HashMap::<&'static str, Decoder>::new();
    m.insert("base64", decode_base64);
    m
});

fn decode_base64(s: &str) -> Result<Vec<u8>, Box<dyn Error>> {
    Ok(base64::engine::general_purpose::STANDARD.decode(s)?)
}

// mediatypes --

/// Defines Mediatype.
#[derive(Clone, Copy)]
pub struct MediaType {
    /// Name of this media-type as defined in RFC 2046.
    /// Example: `application/json`
    pub name: &'static str,

    /// whether this media type can be deserialized to json. If so it can
    /// be validated by `contentSchema` keyword.
    pub json_compatible: bool,

    /**
    Check whether `bytes` conforms to this media-type.

    Should return `Ok(Some(Value))` if `deserialize` is `true`, otherwise it can return `Ok(None)`.
    Ideally you could deserialize to `serde::de::IgnoredAny` if `deserialize` is `false` to gain
    some performance.

    `deserialize` is always `false` if `json_compatible` is `false`.
    */
    #[allow(clippy::type_complexity)]
    pub func: fn(bytes: &[u8], deserialize: bool) -> Result<Option<Value>, Box<dyn Error>>,
}

pub(crate) static MEDIA_TYPES: Lazy<HashMap<&'static str, MediaType>> = Lazy::new(|| {
    let mut m = HashMap::<&'static str, MediaType>::new();
    m.insert(
        "application/json",
        MediaType {
            name: "application/json",
            json_compatible: true,
            func: check_json,
        },
    );
    m
});

fn check_json(bytes: &[u8], deserialize: bool) -> Result<Option<Value>, Box<dyn Error>> {
    if deserialize {
        return Ok(Some(serde_json::from_slice(bytes)?));
    }
    serde_json::from_slice::<IgnoredAny>(bytes)?;
    Ok(None)
}
