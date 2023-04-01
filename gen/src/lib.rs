pub use boon::internal::*;
pub use boon::*;
pub use boon_procmacro::*;

#[cfg(test)]
mod tests {
    use std::{env, error::Error, ffi::OsStr, fs::File, path::Path, process::Command};

    use serde::Deserialize;
    use serde_json::Value;

    static SKIP: [&str; 2] = [
        "zeroTerminatedFloats.json", // only draft4: this behavior is changed in later drafts
        "float-overflow.json",
    ];

    #[derive(Debug, Deserialize)]
    struct Group {
        description: String,
        schema: Value,
        tests: Value,
    }

    #[test]
    fn test_suite() -> Result<(), Box<dyn Error>> {
        let suite = "../lib/tests/JSON-Schema-Test-Suite";
        test_dir(suite, "draft4", "4")?;
        Ok(())
    }

    fn test_dir(suite: &str, path: &str, draft: &str) -> Result<(), Box<dyn Error>> {
        let prefix = Path::new(suite).join("tests");
        let dir = prefix.join(path);
        if !dir.is_dir() {
            return Ok(());
        }
        for entry in dir.read_dir()? {
            let entry = entry?;
            let file_type = entry.file_type()?;
            let tmp_entry_path = entry.path();
            let entry_path = tmp_entry_path.strip_prefix(&prefix)?.to_str().unwrap();
            if file_type.is_file() {
                if !SKIP.iter().any(|n| OsStr::new(n) == entry.file_name()) {
                    test_file(suite, entry_path, draft)?;
                }
            } else if file_type.is_dir() {
                test_dir(suite, entry_path, draft)?;
            }
        }
        Ok(())
    }

    fn test_file(suite: &str, file: &str, draft: &str) -> Result<(), Box<dyn Error>> {
        println!("FILE: {}", file);
        let path = Path::new(suite).join("tests").join(file);
        let groups: Vec<Group> = serde_json::from_reader(File::open(path)?)?;
        for group in groups {
            println!("GROUP: {}", group.description);
            serde_json::to_writer_pretty(File::create("tests/suite/schema.json")?, &group.schema)?;
            serde_json::to_writer_pretty(File::create("tests/suite/tests.json")?, &group.tests)?;
            cargo_test(suite, draft)?;
        }
        Ok(())
    }

    fn cargo_test(suite: &str, draft: &str) -> Result<(), Box<dyn Error>> {
        let mut cmd = Command::new(env::var("CARGO")?);
        cmd.env("BOON_SUITE", format!("../../{suite}"));
        cmd.env("BOON_DRAFT", draft);
        cmd.current_dir(Path::new(&env::var("CARGO_MANIFEST_DIR")?).join("tests/suite"));
        cmd.args(["test", "--lib", "-q", "--", "--nocapture"]);
        if !cmd.spawn()?.wait()?.success() {
            Err("cargo test failed")?
        };
        Ok(())
    }
}
