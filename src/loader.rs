use std::{collections::HashMap, error::Error, fs::File};

use serde_json::Value;
use url::Url;

pub trait ResourceLoader {
    fn load(&self, url: &Url) -> Result<Value, LoadResourceError>;
}

// --

#[derive(Debug)]
pub enum LoadResourceError {
    Load(Box<dyn Error>),
    Unsupported,
}

impl<E> From<E> for LoadResourceError
where
    E: Into<Box<dyn Error>>,
{
    fn from(value: E) -> Self {
        LoadResourceError::Load(value.into())
    }
}

// --

struct FileLoader;

impl ResourceLoader for FileLoader {
    fn load(&self, url: &Url) -> Result<Value, LoadResourceError> {
        let path = url.to_file_path().map_err(|_| "invalid file path")?;
        let file = File::open(path)?;
        Ok(serde_json::from_reader(file)?)
    }
}

// --

pub struct DefaultResourceLoader(HashMap<&'static str, Box<dyn ResourceLoader>>);

impl DefaultResourceLoader {
    pub fn new() -> Self {
        let mut v = Self(Default::default());
        v.0.insert("file", Box::new(FileLoader));
        v
    }

    pub fn register(&mut self, schema: &'static str, loader: Box<dyn ResourceLoader>) {
        self.0.insert(schema, loader);
    }
}

impl ResourceLoader for DefaultResourceLoader {
    fn load(&self, url: &Url) -> Result<Value, LoadResourceError> {
        match self.0.get(url.scheme()) {
            Some(rl) => rl.load(url),
            None => Err(LoadResourceError::Unsupported),
        }
    }
}

// --

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;

    #[test]
    fn test_default_resource_loader() {
        let path = fs::canonicalize("test.json").unwrap();
        let url = Url::from_file_path(path).unwrap();
        let doc = DefaultResourceLoader::new().load(&url).unwrap();
        println!("{:?}", doc);
    }
}
