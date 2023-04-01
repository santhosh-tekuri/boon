use std::{env, fs::File, path::Path};

use boon::{internal::Value, Compiler, Schemas, UrlLoader};
use proc_macro::TokenStream;

#[proc_macro_attribute]
pub fn compile(args: TokenStream, item: TokenStream) -> TokenStream {
    let attr = {
        let dummy = format!("#[xcompile({})] struct Dummy;", args);
        let derive_input: syn::DeriveInput = syn::parse_str(&dummy).unwrap();
        derive_input.attrs.into_iter().next().unwrap()
    };
    //println!("attr: {:#?}", attr);

    // parse attr args
    let mut file = None;
    let mut draft = None;
    attr.parse_nested_meta(|meta| {
        if meta.path.is_ident("file") {
            meta.input.parse::<syn::token::Eq>()?;
            let lit: syn::LitStr = meta.input.parse()?;
            file.replace(lit.value());
            return Ok(());
        }
        if meta.path.is_ident("draft") {
            meta.input.parse::<syn::token::Eq>()?;
            let lit: syn::LitStr = meta.input.parse()?;
            draft.replace(match lit.value().as_str() {
                "4" => boon::Draft::V4,
                "6" => boon::Draft::V6,
                "7" => boon::Draft::V7,
                "2019-09" => boon::Draft::V2019_09,
                "2019-12" => boon::Draft::V2020_12,
                _ => {
                    return Err(
                        meta.error("invalid draft. must be 4 or 6 or 7 or 2019-09 or 2020-12")
                    )
                }
            });
            return Ok(());
        }
        Err(meta.error("unrecognized compile"))
    })
    .unwrap();

    if file.is_none() {
        panic!("file attribute missing");
    }
    let file = file.unwrap();
    //println!("file: {}", file);

    let struct_name = {
        let x: syn::ItemStruct = syn::parse(item).unwrap();
        //println!("item: {:#?}", x);
        x.ident.to_string()
    };
    //println!("structname: {}", struct_name);

    let mut schemas = Schemas::new();
    let mut compiler = Compiler::new();
    if let Ok(remotes) = env::var("BOON_SUITE") {
        println!("got remotes: {}", remotes);
        compiler.register_url_loader("http", Box::new(RemotesLoader(remotes.clone())));
        compiler.register_url_loader("https", Box::new(RemotesLoader(remotes)));
    }
    if let Some(draft) = draft {
        compiler.set_default_draft(draft);
    }
    if let Ok(draft) = env::var("BOON_DRAFT") {
        let draft = match draft.as_str() {
            "4" => boon::Draft::V4,
            "6" => boon::Draft::V6,
            "7" => boon::Draft::V7,
            "2019-09" => boon::Draft::V2019_09,
            "2019-12" => boon::Draft::V2020_12,
            _ => panic!("invalid draft in BOON_DRAFT"),
        };
        compiler.set_default_draft(draft);
    }
    let _sch = match compiler.compile(&file, &mut schemas) {
        Ok(sch) => sch,
        Err(e) => {
            panic!("{e:#}");
        }
    };
    let _sch = compiler.compile(&file, &mut schemas).unwrap();
    let mut gen = boon::internal::Generator::new(struct_name);
    gen.generate(&schemas).into()
}

struct RemotesLoader(String);
impl UrlLoader for RemotesLoader {
    fn load(&self, url: &str) -> Result<Value, Box<dyn std::error::Error>> {
        // remotes folder --
        if let Some(path) = url.strip_prefix("http://localhost:1234/") {
            let path = Path::new(&self.0).join("remotes").join(path);
            let file = File::open(path)?;
            let json: Value = boon::internal::from_reader(file)?;
            return Ok(json);
        }
        Err("no internet")?
    }
}
