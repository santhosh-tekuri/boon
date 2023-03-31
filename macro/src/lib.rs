use boon::{Compiler, Schemas};
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
    attr.parse_nested_meta(|meta| {
        if meta.path.is_ident("file") {
            meta.input.parse::<syn::token::Eq>()?;
            let lit: syn::LitStr = meta.input.parse()?;
            file.replace(lit.value());
        }

        if file.is_none() {
            Err(meta.error("file attribute missing"))?;
        }
        Ok(())
    })
    .unwrap();
    let file = file.unwrap();
    //println!("file: {}", file);

    let x: syn::ItemStruct = syn::parse(item).unwrap();
    //println!("item: {:#?}", x);
    let struct_name = x.ident.to_string();
    //println!("structname: {}", struct_name);

    let mut schemas = Schemas::new();
    let mut compiler = Compiler::new();
    let _sch = compiler.compile(&file, &mut schemas).unwrap();
    let mut gen = boon::internal::Generator::new(struct_name);
    gen.generate(&schemas).into()
}
