use std::{collections::HashMap, error::Error};

use base64::Engine;
use once_cell::sync::Lazy;
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
pub(crate) type MediaType = fn(bytes: &[u8]) -> bool;

pub(crate) static MEDIA_TYPES: Lazy<HashMap<&'static str, MediaType>> = Lazy::new(|| {
    let mut m = HashMap::<&'static str, MediaType>::new();
    m.insert("application/json", is_json);
    m
});

fn is_json(bytes: &[u8]) -> bool {
    serde_json::from_slice::<Value>(bytes).is_ok()
}
