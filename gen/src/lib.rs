pub use boon::internal::*;
pub use boon::*;
pub use boon_procmacro::*;

#[cfg(test)]
mod tests {
    use std::{env, error::Error, path::Path, process::Command};

    #[test]
    fn test_suite() -> Result<(), Box<dyn Error>> {
        cargo_test()?;
        Ok(())
    }

    fn cargo_test() -> Result<(), Box<dyn Error>> {
        let mut cmd = Command::new(env::var("CARGO")?);
        cmd.current_dir(Path::new(&env::var("CARGO_MANIFEST_DIR")?).join("tests/suite"));
        cmd.args(["test", "-q", "--", "--nocapture"]);
        if !cmd.spawn()?.wait()?.success() {
            Err("cargo test failed")?
        };
        Ok(())
    }
}
