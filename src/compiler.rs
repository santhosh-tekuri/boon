use std::collections::HashMap;
use std::error::Error;
use std::fmt::Display;

use url::Url;

use crate::resources::Resources;
use crate::Decoder;
use crate::MediaType;

struct Compiler {
    resources: Resources,
    decoders: HashMap<String, Decoder>,
    media_types: HashMap<String, MediaType>,
}

#[derive(Debug)]
pub enum CompileError {
    LoadResourceError { res: Url, src: Box<dyn Error> },
    LoadUnsupported { res: Url },
    InvalidMetaSchema { res: Url },
    MetaSchemaCycle { res: Url },
    InvalidId { loc: Url },
    DuplicateId { res: Url, id: Url },
}

impl Error for CompileError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::LoadResourceError { src, .. } => Some(src.as_ref()),
            _ => None,
        }
    }
}

impl Display for CompileError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::LoadResourceError { res, src } => {
                if f.alternate() {
                    write!(f, "error loading {res}: {src}")
                } else {
                    write!(f, "error loading {res}")
                }
            }
            Self::LoadUnsupported { res } => write!(f, "loading {res} unsupported"),
            Self::InvalidMetaSchema { res } => write!(f, "invalid $schema in {res}"),
            Self::MetaSchemaCycle { res } => {
                write!(f, "cycle in resolving $schema in {res}")
            }
            Self::InvalidId { loc } => write!(f, "invalid $id at {loc}"),
            Self::DuplicateId { res, id } => write!(f, "duplicate $id {id} in {res}"),
        }
    }
}
