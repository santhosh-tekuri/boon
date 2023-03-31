use std::{env, error::Error, fs::File, path::Path};

use boon::{Compiler, Draft, Schemas};
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[test]
fn test_suites() -> Result<(), Box<dyn Error>> {
    if let Ok(suite) = env::var("TEST_SUITE") {
        test_suite(&suite)?;
    } else {
        test_suite("tests/JSON-Schema-Test-Suite")?;
        test_suite("tests/Extra-Suite")?;
    }
    Ok(())
}

fn test_suite(suite: &str) -> Result<(), Box<dyn Error>> {
    test_folder(suite, "draft2019-09", Draft::V2019_09)?;
    test_folder(suite, "draft2020-12", Draft::V2020_12)?;
    Ok(())
}

fn test_folder(suite: &str, folder: &str, draft: Draft) -> Result<(), Box<dyn Error>> {
    let output_schema_url = format!(
        "https://json-schema.org/draft/{}/output/schema",
        folder.strip_prefix("draft").unwrap()
    );
    let prefix = Path::new(suite).join("output-tests");
    let folder = prefix.join(folder);
    let content = folder.join("content");
    if !content.is_dir() {
        return Ok(());
    }
    let output_schema: Value =
        serde_json::from_reader(File::open(folder.join("output-schema.json"))?)?;
    for entry in content.read_dir()? {
        let entry = entry?;
        if !entry.file_type()?.is_file() {
            continue;
        };
        let entry_path = entry.path();
        println!("{}", entry_path.strip_prefix(&prefix)?.to_str().unwrap());
        let groups: Vec<Group> = serde_json::from_reader(File::open(entry_path)?)?;
        for group in groups {
            println!("    {}", group.description);
            let mut schemas = Schemas::new();
            let mut compiler = Compiler::new();
            compiler.set_default_draft(draft);
            let schema_url = "http://output-tests/schema";
            compiler.add_resource(schema_url, group.schema)?;
            let sch = compiler.compile(schema_url, &mut schemas)?;
            for test in group.tests {
                println!("        {}", test.description);
                match schemas.validate(&test.data, sch) {
                    Ok(_) => println!("            validation success"),
                    Err(e) => {
                        if let Some(sch) = test.output.basic {
                            let mut schemas = Schemas::new();
                            let mut compiler = Compiler::new();
                            compiler.set_default_draft(draft);
                            compiler.add_resource(&output_schema_url, output_schema.clone())?;
                            let schema_url = "http://output-tests/schema";
                            compiler.add_resource(schema_url, sch)?;
                            let sch = compiler.compile(schema_url, &mut schemas)?;
                            let basic: Value = serde_json::from_str(&e.basic_output().to_string())?;
                            let result = schemas.validate(&basic, sch);
                            if let Err(e) = result {
                                println!("{basic:#}\n");
                                for line in format!("{e}").lines() {
                                    println!("            {line}");
                                }
                                panic!("basic output did not match");
                            }
                        }
                        if let Some(sch) = test.output.detailed {
                            let mut schemas = Schemas::new();
                            let mut compiler = Compiler::new();
                            compiler.set_default_draft(draft);
                            compiler.add_resource(&output_schema_url, output_schema.clone())?;
                            let schema_url = "http://output-tests/schema";
                            compiler.add_resource(schema_url, sch)?;
                            let sch = compiler.compile(schema_url, &mut schemas)?;
                            let detailed: Value =
                                serde_json::from_str(&e.detailed_output().to_string())?;
                            let result = schemas.validate(&detailed, sch);
                            if let Err(e) = result {
                                println!("{detailed:#}\n");
                                for line in format!("{e}").lines() {
                                    println!("            {line}");
                                }
                                panic!("detailed output did not match");
                            }
                        }
                    }
                }
            }
        }
    }

    Ok(())
}

#[derive(Debug, Serialize, Deserialize)]
struct Group {
    description: String,
    schema: Value,
    tests: Vec<Test>,
}

#[derive(Debug, Serialize, Deserialize)]
struct Test {
    description: String,
    data: Value,
    output: Output,
}

#[derive(Debug, Serialize, Deserialize)]
struct Output {
    basic: Option<Value>,
    detailed: Option<Value>,
}
