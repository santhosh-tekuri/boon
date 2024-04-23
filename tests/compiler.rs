use std::error::Error;

use boon::{Compiler, Schemas};
use serde_json::json;

#[test]
fn test_metaschema_resource() -> Result<(), Box<dyn Error>> {
    let main_schema = json!({
        "$schema": "http://tmp.com/meta.json",
        "type": "number"
    });
    let meta_schema = json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "$vocabulary": {
            "https://json-schema.org/draft/2020-12/vocab/applicator": true,
            "https://json-schema.org/draft/2020-12/vocab/core": true
        },
        "allOf": [
            { "$ref": "https://json-schema.org/draft/2020-12/meta/applicator" },
            { "$ref": "https://json-schema.org/draft/2020-12/meta/core" }
        ]
    });

    let mut schemas = Schemas::new();
    let mut compiler = Compiler::new();
    compiler.add_resource("schema.json", main_schema)?;
    compiler.add_resource("http://tmp.com/meta.json", meta_schema)?;
    compiler.compile("schema.json", &mut schemas)?;

    Ok(())
}

#[test]
fn test_compile_anchor() -> Result<(), Box<dyn Error>> {
    let schema = json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "$defs": {
            "x": {
                "$anchor": "a1",
                "type": "number"
            }
        }
    });

    let mut schemas = Schemas::new();
    let mut compiler = Compiler::new();
    compiler.add_resource("schema.json", schema)?;
    let sch_index1 = compiler.compile("schema.json#a1", &mut schemas)?;
    let sch_index2 = compiler.compile("schema.json#/$defs/x", &mut schemas)?;
    assert_eq!(sch_index1, sch_index2);

    Ok(())
}

#[test]
fn test_compile_nonstd() -> Result<(), Box<dyn Error>> {
    let schema = json!({
        "components": {
            "schemas": {
                "foo" : {
                    "$schema": "https://json-schema.org/draft/2020-12/schema",
                    "$defs": {
                        "x": {
                            "$anchor": "a",
                            "type": "number"
                        },
                        "y": {
                            "$id": "http://temp.com/y",
                            "type": "string"
                        }
                    },
                    "oneOf": [
                        { "$ref": "#a" },
                        { "$ref": "http://temp.com/y" }
                    ]
                }
            }
        }
    });

    let mut schemas = Schemas::new();
    let mut compiler = Compiler::new();
    compiler.add_resource("schema.json", schema)?;
    compiler.compile("schema.json#/components/schemas/foo", &mut schemas)?;

    Ok(())
}
