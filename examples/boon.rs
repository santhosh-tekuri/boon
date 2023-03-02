use std::{env, error::Error, fs::File, process, str::FromStr};

use boon::{Compiler, Draft, Schemas, UrlLoader};
use getopts::Options;
use serde_json::Value;

fn main() {
    let opts = options();
    let matches = match opts.parse(env::args().skip(1)) {
        Ok(m) => m,
        Err(f) => {
            eprintln!("{f}");
            eprintln!("{}", opts.usage(BRIEF));
            process::exit(1)
        }
    };

    // draft --
    let mut draft = Draft::default();
    if let Some(v) = matches.opt_str("draft") {
        let Ok(v) = usize::from_str(&v) else {
            eprintln!("invalid draft: {v}");
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
                eprintln!("{}", opts.usage(BRIEF));
                process::exit(1);
            }
        };
    }

    // output --
    let output = matches.opt_str("output");
    if let Some(o) = &output {
        if !matches!(
            o.as_str(),
            "default" | "alt" | "flag" | "basic" | "detailed"
        ) {
            eprintln!("invalid output: {o}");
            eprintln!("{}", opts.usage(BRIEF));
            process::exit(1);
        }
    }

    // flags --
    let assert_format = matches.opt_present("assert-format");
    let assert_content = matches.opt_present("assert-content");

    // schema --
    let Some(schema) = matches.free.get(0) else {
        eprintln!("missing SCHEMA");
        eprintln!("{}", opts.usage(BRIEF));
        process::exit(1);
    };

    // compile --
    let mut schemas = Schemas::new();
    let mut compiler = Compiler::new();
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
            println!("{e:#}");
            process::exit(2);
        }
    };

    // validate --
    let mut all_valid = true;
    for instance in &matches.free[1..] {
        println!();
        let rdr = match File::open(instance) {
            Ok(rdr) => rdr,
            Err(e) => {
                println!("instance {instance}: failed");
                println!("error reading file {instance}: {e}");
                all_valid = false;
                continue;
            }
        };
        let value: Value = match serde_json::from_reader(rdr) {
            Ok(v) => v,
            Err(e) => {
                println!("instance {instance}: failed");
                println!("error parsing file {instance}: {e}");
                all_valid = false;
                continue;
            }
        };
        match schemas.validate(&value, sch) {
            Ok(_) => println!("instance {instance}: ok"),
            Err(e) => {
                println!("instance {instance}: failed");
                match &output {
                    Some(out) => match out.as_str() {
                        "default" => println!("{e}"),
                        "alt" => println!("{e:#}"),
                        "flag" => println!("{:#}", e.flag_output()),
                        "basic" => println!("{:#}", e.basic_output()),
                        "detailed" => println!("{:#}", e.detailed_output()),
                        _ => (),
                    },
                    None => println!("{e}"),
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

const BRIEF: &str =  "usage: boon [--draft VERSION] [--output FORMAT] [--assert-format] [-assert-content] SCHEMA [INSTANCE...]";

fn options() -> Options {
    let mut opts = Options::new();
    opts.optopt("d", "draft", "draft used when '$schema' attribute is missing. valid values 4, 6, 7, 2019, 2020 (default 2020)", "VERSION");
    opts.optopt(
        "o",
        "output",
        "output format. valid values default, alt, flag, basic, detailed",
        "FORMAT",
    );
    opts.optflag(
        "f",
        "assert-format",
        "enable format assertions with draft >= 2019",
    );
    opts.optflag(
        "c",
        "assert-content",
        "enable content assertions with draft >= 7",
    );
    opts
}

struct HttpUrlLoader;
impl UrlLoader for HttpUrlLoader {
    fn load(&self, url: &str) -> Result<Value, Box<dyn Error>> {
        let reader = ureq::get(url).call()?.into_reader();
        Ok(serde_json::from_reader(reader)?)
    }
}
