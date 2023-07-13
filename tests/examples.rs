use std::{error::Error, fs::File};

use boon::{Compiler, Decoder, Format, MediaType, Schemas, UrlLoader};
use serde::de::IgnoredAny;
use serde_json::{json, Value};
use url::Url;

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

/**
This example shows how to load json schema from strings.

The schema url used plays important role in resolving
schema references.

You can see that `cat.json` is resolved internally to
another string schema where as dog.json is resolved
to local file.
*/
#[test]
fn example_from_strings() -> Result<(), Box<dyn Error>> {
    let cat_schema: Value = json!({
        "type": "object",
        "properties": {
            "speak": { "const": "meow" }
        },
        "required": ["speak"]
    });
    let pet_schema: Value = json!({
        "oneOf": [
            { "$ref": "dog.json" },
            { "$ref": "cat.json" }
        ],
    });
    let instance: Value = json!({"speak": "bow"});

    let mut schemas = Schemas::new();
    let mut compiler = Compiler::new();
    compiler.add_resource("tests/examples/pet.json", pet_schema)?;
    compiler.add_resource("tests/examples/cat.json", cat_schema)?;
    let sch_index = compiler.compile("tests/examples/pet.json", &mut schemas)?;
    let result = schemas.validate(&instance, sch_index);
    assert!(result.is_ok());

    Ok(())
}

#[test]
fn example_without_files() -> Result<(), Box<dyn Error>> {
    let cat_schema: Value = json!({
        "type": "object",
        "properties": {
            "speak": { "const": "meow" }
        },
        "required": ["speak"]
    });
    let instance: Value = json!({"speak": "meow"});

    let mut schemas = Schemas::new();
    let mut compiler = Compiler::new();
    let uri = Url::parse("uri://cat")?;
    compiler.add_resource_url(uri.clone(), cat_schema)?;
    let sch_index = compiler.compile_url(uri, &mut schemas)?;
    let result = schemas.validate(&instance, sch_index);
    assert!(result.is_ok());

    Ok(())
}

#[test]
#[ignore]
fn example_from_https() -> Result<(), Box<dyn Error>> {
    let schema_url = "https://json-schema.org/learn/examples/geographical-location.schema.json";
    let instance: Value = json!({"latitude": 48.858093, "longitude": 2.294694});

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
fn example_from_yaml_files() -> Result<(), Box<dyn Error>> {
    let schema_file = "tests/examples/schema.yml";
    let instance: Value = serde_yaml::from_reader(File::open("tests/examples/instance.yml")?)?;

    struct FileUrlLoader;
    impl UrlLoader for FileUrlLoader {
        fn load(&self, url: &str) -> Result<Value, Box<dyn Error>> {
            let url = Url::parse(url)?;
            let path = url.to_file_path().map_err(|_| "invalid file path")?;
            let file = File::open(&path)?;
            if path
                .extension()
                .filter(|&ext| ext == "yaml" || ext == "yml")
                .is_some()
            {
                Ok(serde_yaml::from_reader(file)?)
            } else {
                Ok(serde_json::from_reader(file)?)
            }
        }
    }

    let mut schemas = Schemas::new();
    let mut compiler = Compiler::new();
    compiler.register_url_loader("file", Box::new(FileUrlLoader));
    let sch_index = compiler.compile(schema_file, &mut schemas)?;
    let result = schemas.validate(&instance, sch_index);
    assert!(result.is_ok());

    Ok(())
}

#[test]
fn example_custom_format() -> Result<(), Box<dyn Error>> {
    let schema_url = "http://tmp/schema.json";
    let schema: Value = json!({"type": "string", "format": "palindrome"});
    let instance: Value = json!("step on no pets");

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
    compiler.register_format(Format {
        name: "palindrome",
        func: is_palindrome,
    });
    compiler.add_resource(schema_url, schema)?;
    let sch_index = compiler.compile(schema_url, &mut schemas)?;
    let result = schemas.validate(&instance, sch_index);
    assert!(result.is_ok());

    Ok(())
}

#[test]
fn example_custom_content_encoding() -> Result<(), Box<dyn Error>> {
    let schema_url = "http://tmp/schema.json";
    let schema: Value = json!({"type": "string", "contentEncoding": "hex"});
    let instance: Value = json!("aBcdxyz");

    fn decode(b: u8) -> Result<u8, Box<dyn Error>> {
        match b {
            b'0'..=b'9' => Ok(b - b'0'),
            b'a'..=b'f' => Ok(b - b'a' + 10),
            b'A'..=b'F' => Ok(b - b'A' + 10),
            _ => Err("decode_hex: non-hex char")?,
        }
    }
    fn decode_hex(s: &str) -> Result<Vec<u8>, Box<dyn Error>> {
        if s.len() % 2 != 0 {
            Err("decode_hex: odd length")?;
        }
        let mut bytes = s.bytes();
        let mut out = Vec::with_capacity(s.len() / 2);
        for _ in 0..out.len() {
            if let (Some(b1), Some(b2)) = (bytes.next(), bytes.next()) {
                out.push(decode(b1)? << 4 | decode(b2)?);
            } else {
                Err("decode_hex: non-ascii char")?;
            }
        }
        Ok(out)
    }

    let mut schemas = Schemas::new();
    let mut compiler = Compiler::new();
    compiler.enable_content_assertions(); // content assertions are not enabled by default
    compiler.register_content_encoding(Decoder {
        name: "hex",
        func: decode_hex,
    });
    compiler.add_resource(schema_url, schema)?;
    let sch_index = compiler.compile(schema_url, &mut schemas)?;
    let result = schemas.validate(&instance, sch_index);
    assert!(result.is_err());

    Ok(())
}

#[test]
fn example_custom_content_media_type() -> Result<(), Box<dyn Error>> {
    let schema_url = "http://tmp/schema.json";
    let schema: Value = json!({"type": "string", "contentMediaType": "application/yaml"});
    let instance: Value = json!("name:foobar");

    fn check_yaml(bytes: &[u8], deserialize: bool) -> Result<Option<Value>, Box<dyn Error>> {
        if deserialize {
            return Ok(Some(serde_yaml::from_slice(bytes)?));
        }
        serde_yaml::from_slice::<IgnoredAny>(bytes)?;
        Ok(None)
    }

    let mut schemas = Schemas::new();
    let mut compiler = Compiler::new();
    compiler.enable_content_assertions(); // content assertions are not enabled by default
    compiler.register_content_media_type(MediaType {
        name: "application/yaml",
        json_compatible: true,
        func: check_yaml,
    });
    compiler.add_resource(schema_url, schema)?;
    let sch_index = compiler.compile(schema_url, &mut schemas)?;
    let result = schemas.validate(&instance, sch_index);
    assert!(result.is_ok());

    Ok(())
}
