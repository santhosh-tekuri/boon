pub use boon::internal::*;
pub use boon::*;
pub use boon_procmacro::*;

#[cfg(test)]
mod tests {
    use std::{env, error::Error, fs::File, path::Path, process::Command};

    use serde::Deserialize;
    use serde_json::Value;

    #[derive(Debug, Deserialize)]
    struct Group {
        description: String,
        schema: Value,
        tests: Value,
    }

    #[test]
    fn test_suite() -> Result<(), Box<dyn Error>> {
        let file = "../lib/tests/JSON-Schema-Test-Suite/tests/draft4/enum.json";
        println!("FILE: {}", file);
        let groups: Vec<Group> = serde_json::from_reader(File::open(file)?)?;
        for group in &groups {
            println!("GROUP: {}", group.description);
            serde_json::to_writer(File::create("tests/suite/schema.json")?, &group.schema)?;
            serde_json::to_writer(File::create("tests/suite/tests.json")?, &group.tests)?;
            cargo_test()?;
        }
        Ok(())
    }

    fn cargo_test() -> Result<(), Box<dyn Error>> {
        let mut cmd = Command::new(env::var("CARGO")?);
        cmd.current_dir(Path::new(&env::var("CARGO_MANIFEST_DIR")?).join("tests/suite"));
        cmd.args(["test", "--lib", "-q", "--", "--nocapture"]);
        if !cmd.spawn()?.wait()?.success() {
            Err("cargo test failed")?
        };
        Ok(())
    }
}
