use std::{env, error::Error, fs::File, io::BufReader, process, str::FromStr};

use boon::{Compiler, Draft, Schemas, UrlLoader};
use getopts::Options;
use serde_json::Value;
use url::Url;

fn main() {
    let opts = options();
    let matches = match opts.parse(env::args().skip(1)) {
        Ok(m) => m,
        Err(f) => {
            eprintln!("{f}");
            eprintln!();
            eprintln!("{}", opts.usage(BRIEF));
            process::exit(1)
        }
    };

    if matches.opt_present("help") {
        println!("{}", opts.usage(BRIEF));
        process::exit(0);
    }

    // draft --
    let mut draft = Draft::default();
    if let Some(v) = matches.opt_str("draft") {
        let Ok(v) = usize::from_str(&v) else {
            eprintln!("invalid draft: {v}");
            eprintln!();
            eprintln!("{}", opts.usage(BRIEF));
            process::exit(1);
        };
        draft = match v {
            4 => Draft::V4,
            6 => Draft::V6,
            7 => Draft::V7,
            2019 => Draft::V2019_09,
            2020 => Draft::V2020_12,
            _ => {
                eprintln!("invalid draft: {v}");
                eprintln!();
                eprintln!("{}", opts.usage(BRIEF));
                process::exit(1);
            }
        };
    }

    // output --
    let output = matches.opt_str("output");
    if let Some(o) = &output {
        if !matches!(o.as_str(), "simple" | "alt" | "flag" | "basic" | "detailed") {
            eprintln!("invalid output: {o}");
            eprintln!();
            eprintln!("{}", opts.usage(BRIEF));
            process::exit(1);
        }
    }

    // flags --
    let quiet = matches.opt_present("quiet");
    let assert_format = matches.opt_present("assert-format");
    let assert_content = matches.opt_present("assert-content");

    // schema --
    let Some(schema) = matches.free.get(0) else {
        eprintln!("missing SCHEMA");
        eprintln!();
        eprintln!("{}", opts.usage(BRIEF));
        process::exit(1);
    };

    // compile --
    let mut schemas = Schemas::new();
    let mut compiler = Compiler::new();
    compiler.register_url_loader("file", Box::new(FileUrlLoader));
    compiler.register_url_loader("http", Box::new(HttpUrlLoader));
    compiler.register_url_loader("https", Box::new(HttpUrlLoader));
    compiler.set_default_draft(draft);
    if assert_format {
        compiler.enable_format_assertions();
    }
    if assert_content {
        compiler.enable_content_assertions();
    }
    let sch = match compiler.compile(schema, &mut schemas) {
        Ok(sch) => {
            println!("schema {schema}: ok");
            sch
        }
        Err(e) => {
            println!("schema {schema}: failed");
            if !quiet {
                println!("{e:#}");
            }
            process::exit(2);
        }
    };

    // validate --
    let mut all_valid = true;
    for instance in &matches.free[1..] {
        if !quiet {
            println!();
        }
        let rdr = match File::open(instance) {
            Ok(rdr) => BufReader::new(rdr),
            Err(e) => {
                println!("instance {instance}: failed");
                if !quiet {
                    println!("error reading file {instance}: {e}");
                }
                all_valid = false;
                continue;
            }
        };
        let value: Result<Value, String> =
            if instance.ends_with(".yaml") || instance.ends_with(".yml") {
                serde_yaml::from_reader(rdr).map_err(|e| e.to_string())
            } else {
                serde_json::from_reader(rdr).map_err(|e| e.to_string())
            };
        let value = match value {
            Ok(v) => v,
            Err(e) => {
                println!("instance {instance}: failed");
                if !quiet {
                    println!("error parsing file {instance}: {e}");
                }
                all_valid = false;
                continue;
            }
        };
        match schemas.validate(&value, sch) {
            Ok(_) => println!("instance {instance}: ok"),
            Err(e) => {
                println!("instance {instance}: failed");
                if !quiet {
                    match &output {
                        Some(out) => match out.as_str() {
                            "simple" => println!("{e}"),
                            "alt" => println!("{e:#}"),
                            "flag" => println!("{:#}", e.flag_output()),
                            "basic" => println!("{:#}", e.basic_output()),
                            "detailed" => println!("{:#}", e.detailed_output()),
                            _ => (),
                        },
                        None => println!("{e}"),
                    }
                }
                all_valid = false;
                continue;
            }
        };
    }
    if !all_valid {
        process::exit(2);
    }
}

const BRIEF: &str = "Usage: boon [OPTIONS] SCHEMA [INSTANCE...]";

fn options() -> Options {
    let mut opts = Options::new();
    opts.optflag("h", "help", "Print help information");
    opts.optflag("q", "quiet", "Do not print errors");
    opts.optopt(
        "d",
        "draft",
        "Draft used when '$schema' is missing. Valid values 4, 6, 7, 2019, 2020 (default 2020)",
        "<VER>",
    );
    opts.optopt(
        "o",
        "output",
        "Output format. Valid values simple, alt, flag, basic, detailed (default simple)",
        "<FMT>",
    );
    opts.optflag(
        "f",
        "assert-format",
        "Enable format assertions with draft >= 2019",
    );
    opts.optflag(
        "c",
        "assert-content",
        "Enable content assertions with draft >= 7",
    );
    opts
}

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

struct HttpUrlLoader;
impl UrlLoader for HttpUrlLoader {
    fn load(&self, url: &str) -> Result<Value, Box<dyn Error>> {
        let response = ureq::get(url).call()?;
        let is_yaml = url.ends_with(".yaml") || url.ends_with(".yml") || {
            let ctype = response.content_type();
            ctype.ends_with("/yaml") || ctype.ends_with("-yaml")
        };
        if is_yaml {
            Ok(serde_yaml::from_reader(response.into_reader())?)
        } else {
            Ok(serde_json::from_reader(response.into_reader())?)
        }
    }
}
