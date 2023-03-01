use std::{error::Error, fs::File};

use boon::{Compiler, Schemas, UrlLoader};
use serde_json::Value;

#[test]
fn example_from_files() -> Result<(), Box<dyn Error>> {
    let schema_file = "tests/examples/schema.json";

    let instance: Value = serde_json::from_reader(File::open("tests/examples/instance.json")?)?;

    let mut schemas = Schemas::new();
    let mut compiler = Compiler::new();
    let sch_index = compiler.compile(schema_file, &mut schemas)?;
    let result = schemas.validate(&instance, sch_index);
    assert!(result.is_ok());

    Ok(())
}

#[test]
fn example_from_strings() -> Result<(), Box<dyn Error>> {
    let schema_url = "http://tmp/schema.json";
    let schema: Value = serde_json::from_str(r#"{"type": "object"}"#)?;
    let instance: Value = serde_json::from_str(r#"{"foo": "bar"}"#)?;

    let mut schemas = Schemas::new();
    let mut compiler = Compiler::new();
    compiler.add_resource(schema_url, schema)?;
    let sch_index = compiler.compile(schema_url, &mut schemas)?;
    let result = schemas.validate(&instance, sch_index);
    assert!(result.is_ok());

    Ok(())
}

#[test]
#[ignore]
fn example_from_https() -> Result<(), Box<dyn Error>> {
    let schema_url = "https://json-schema.org/learn/examples/geographical-location.schema.json";
    let instance: Value =
        serde_json::from_str(r#"{"latitude": 48.858093, "longitude": 2.294694}"#)?;

    struct HttpUrlLoader;
    impl UrlLoader for HttpUrlLoader {
        fn load(&self, url: &str) -> Result<Value, Box<dyn Error>> {
            let reader = ureq::get(url).call()?.into_reader();
            Ok(serde_json::from_reader(reader)?)
        }
    }

    let mut schemas = Schemas::new();
    let mut compiler = Compiler::new();
    compiler.register_url_loader("http", Box::new(HttpUrlLoader));
    compiler.register_url_loader("https", Box::new(HttpUrlLoader));
    let sch_index = compiler.compile(schema_url, &mut schemas)?;
    let result = schemas.validate(&instance, sch_index);
    assert!(result.is_ok());

    Ok(())
}

#[test]
fn example_custom_format() -> Result<(), Box<dyn Error>> {
    let schema_url = "http://tmp/schema.json";
    let schema: Value = serde_json::from_str(r#"{"type": "string", "format": "palindrome"}"#)?;
    let instance: Value = serde_json::from_str(r#""step on no pets""#)?;

    fn is_palindrome(v: &Value) -> Result<(), Box<dyn Error>> {
        let Value::String(s) = v else {
            return Ok(()); // applicable only on strings
        };
        let mut chars = s.chars();
        while let (Some(c1), Some(c2)) = (chars.next(), chars.next_back()) {
            if c1 != c2 {
                Err("char mismatch")?;
            }
        }
        Ok(())
    }

    let mut schemas = Schemas::new();
    let mut compiler = Compiler::new();
    compiler.enable_format_assertions(); // in draft2020-12 format assertions are not enabled by default
    compiler.register_format("palindrome", is_palindrome);
    compiler.add_resource(schema_url, schema)?;
    let sch_index = compiler.compile(schema_url, &mut schemas)?;
    let result = schemas.validate(&instance, sch_index);
    assert!(result.is_ok());

    Ok(())
}
