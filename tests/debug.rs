use std::{error::Error, fs::File};

use boon::{Compiler, Schemas, UrlLoader};
use serde_json::{Map, Value};

#[test]
fn test_debug() -> Result<(), Box<dyn Error>> {
    let test: Value = serde_json::from_reader(File::open("tests/debug.json")?)?;
    let mut schemas = Schemas::new();
    let mut compiler = Compiler::new();
    compiler.enable_format_assertions();
    compiler.enable_content_assertions();
    let remotes = Remotes(test["remotes"].as_object().unwrap().clone());
    compiler.register_url_loader("http", Box::new(remotes));
    let url = "http://debug.com/schema.json";
    compiler.add_resource(url, test["schema"].clone())?;
    let sch = compiler.compile(url, &mut schemas)?;
    let result = schemas.validate(&test["data"], sch);
    if let Err(e) = &result {
        for line in format!("{e}").lines() {
            println!("        {line}");
        }
        for line in format!("{e:#}").lines() {
            println!("        {line}");
        }
    }
    assert_eq!(result.is_ok(), test["valid"].as_bool().unwrap());
    Ok(())
}

struct Remotes(Map<String, Value>);

impl UrlLoader for Remotes {
    fn load(&self, url: &str) -> Result<Value, Box<dyn Error>> {
        if let Some(v) = self.0.get(url) {
            return Ok(v.clone());
        }
        Err("remote not found")?
    }
}
