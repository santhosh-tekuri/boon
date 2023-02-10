use std::collections::HashMap;
use std::error::Error;
use std::fmt::Display;

use crate::roots::Roots;
use crate::Decoder;
use crate::MediaType;

struct Compiler {
    roots: Roots,
    decoders: HashMap<String, Decoder>,
    media_types: HashMap<String, MediaType>,
}

#[derive(Debug)]
pub enum CompileError {
    LoadUrlError { url: String, src: Box<dyn Error> },
    UnsupportedUrl { url: String },
    InvalidMetaSchema { url: String },
    MetaSchemaCycle { url: String },
    InvalidId { loc: String },
    DuplicateId { url: String, id: String },
}

impl Error for CompileError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::LoadUrlError { src, .. } => Some(src.as_ref()),
            _ => None,
        }
    }
}

impl Display for CompileError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::LoadUrlError { url, src } => {
                if f.alternate() {
                    write!(f, "error loading {url}: {src}")
                } else {
                    write!(f, "error loading {url}")
                }
            }
            Self::UnsupportedUrl { url } => write!(f, "loading {url} unsupported"),
            Self::InvalidMetaSchema { url } => write!(f, "invalid $schema in {url}"),
            Self::MetaSchemaCycle { url } => {
                write!(f, "cycle in resolving $schema in {url}")
            }
            Self::InvalidId { loc } => write!(f, "invalid $id at {loc}"),
            Self::DuplicateId { url, id } => write!(f, "duplicate $id {id} in {url}"),
        }
    }
}
