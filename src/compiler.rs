use std::collections::HashMap;

use url::Url;

use crate::loader::LoadResourceError;
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
    LoadResourceError(LoadResourceError),
    InvalidMetaSchema { resource_url: Url },
    MetaSchemaCycle { resource_url: Url },
    InvalidId { url: Url },
}

impl From<LoadResourceError> for CompileError {
    fn from(value: LoadResourceError) -> Self {
        CompileError::LoadResourceError(value)
    }
}
