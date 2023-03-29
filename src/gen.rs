#![allow(dead_code)]

use std::str::FromStr;

use quote::{__private::TokenStream, format_ident, quote, ToTokens};

use crate::{Additional, Enum, Items, Schema, Schemas, Type};

struct Generator {
    struct_name: &'static str,
    fields: Vec<TokenStream>,
    init: Vec<TokenStream>,
}
pub use crate::formats::{
    validate_date, validate_date_time, validate_duration, validate_email, validate_hostname,
    validate_idn_email, validate_idn_hostname, validate_ipv4, validate_ipv6, validate_iri,
    validate_iri_reference, validate_json_pointer, validate_period, validate_regex,
    validate_relative_json_pointer, validate_time, validate_uri, validate_uri_reference,
    validate_uri_template, validate_uuid,
};
pub use crate::util::equals;

impl Generator {
    fn new(struct_name: &'static str) -> Self {
        Self {
            struct_name,
            fields: vec![],
            init: vec![],
        }
    }

    fn generate(&mut self, schemas: &Schemas) -> TokenStream {
        let name = format_ident!("{}", self.struct_name);
        let mut body = vec![];
        for sch in &schemas.list {
            body.push(self.gen_sch(sch));
        }

        let fields = &self.fields;
        let inits = &self.init;
        quote! {
            #![allow(dead_code)]

            struct #name{
                #(#fields),*
            }

            impl #name {
                fn new() -> Self {
                    Self{
                        #(#inits),*
                    }
                }
                #(#body)*
            }
        }
    }

    fn gen_sch(&mut self, sch: &Schema) -> TokenStream {
        let name = format_ident!("is_valid{}", sch.idx.0);
        let loc = sch.loc.to_token_stream();

        if let Some(b) = sch.boolean {
            return quote! {
                fn #name(&self, v: &serde_json::Value) -> bool {
                    #b
                }
            };
        }

        // type agnotic --
        let mut body = vec![
            self.gen_types(sch),
            self.gen_const(sch),
            self.gen_enum(sch),
            self.gen_format(sch),
            self.gen_ref(sch),
            self.gen_not(sch),
            self.gen_allof(sch),
            self.gen_anyof(sch),
            self.gen_oneof(sch),
        ];

        let mut arms = vec![];

        // array specific --
        let mut arr = vec![
            self.gen_min_items(sch),
            self.gen_max_items(sch),
            self.gen_unique_items(sch),
            self.gen_items(sch),
            self.gen_additional_items(sch),
        ];
        arr.retain(|t| !t.is_empty());
        if !arr.is_empty() {
            arms.push(quote! {
                Value::Array(arr) => { #(#arr)* }
            });
        }

        // object specific --
        let mut obj = vec![
            self.gen_min_properties(sch),
            self.gen_max_properties(sch),
            self.gen_required(sch),
        ];
        obj.retain(|t| !t.is_empty());
        if !obj.is_empty() {
            arms.push(quote! {
                Value::Object(obj) => { #(#obj)* }
            });
        }

        // string specific --
        let mut str = vec![self.gen_length(sch), self.gen_pattern(sch)];
        str.retain(|t| !t.is_empty());
        if !str.is_empty() {
            arms.push(quote! {
                Value::String(str) => { #(#str)* }
            });
        }

        // number specific --
        let mut num = vec![self.gen_num(sch)];
        num.retain(|t| !t.is_empty());
        if !num.is_empty() {
            arms.push(quote! {
                Value::Number(num) => { #(#num)* }
            });
        }
        if !arms.is_empty() {
            arms.push(quote! {
                _ => {}
            });
            body.push(quote! {
                match v {
                    #(#arms)*
                }
            });
        }

        quote! {
            #[doc=#loc]
            fn #name(&self, v: &serde_json::Value) -> bool {
                #(#body)*
                true
            }
        }
    }

    fn gen_types(&mut self, sch: &Schema) -> TokenStream {
        if sch.types.is_empty() {
            return TokenStream::new();
        }

        let field = format_ident!("types{}", sch.idx.0);
        self.fields.push(quote! {
            #field: boon::Types
        });

        let mut types = vec![];
        for t in sch.types.iter() {
            let s = t.to_string();
            let ty = format_ident!("{}{}", s[..1].to_uppercase(), &s[1..]);
            types.push(quote! {
                boon::Type::#ty
            });
        }
        self.init.push(quote! {
            #field: boon::Types::from_iter([#(#types),*])
        });

        let mut arms = vec![];
        let mut integer_arm = TokenStream::new();
        if sch.types.contains(Type::Null) {
            arms.push(quote! {
                Value::Null
            });
        }
        if sch.types.contains(Type::Boolean) {
            arms.push(quote! {
                Value::Bool(_)
            });
        }
        if sch.types.contains(Type::String) {
            arms.push(quote! {
                Value::String(_)
            });
        }
        if sch.types.contains(Type::Number) {
            arms.push(quote! {
                Value::Number(_)
            });
        } else if sch.types.contains(Type::Integer) {
            integer_arm = quote! {
                Value::Number(n) =>  n.is_i64() || n.is_u64() || n.as_f64().filter(|n| n.fract() == 0.0).is_some(),
            }
        }
        if sch.types.contains(Type::Object) {
            arms.push(quote! {
                Value::Object(_)
            });
        }
        if sch.types.contains(Type::Array) {
            arms.push(quote! {
                Value::Array(_)
            });
        }
        let arms = if arms.is_empty() {
            TokenStream::new()
        } else {
            quote! {
                #(#arms)|* => true,
            }
        };
        quote! {
            use serde_json::Value;
            let type_matched = match v {
                #arms
                #integer_arm
                _ => false,
            };
            if !type_matched {
                return false;
            }
        }
    }

    fn gen_const(&mut self, sch: &Schema) -> TokenStream {
        let Some(v) = &sch.constant else {
            return TokenStream::new();
        };

        let field = format_ident!("const{}", sch.idx.0);
        self.fields.push(quote! {
            #field: serde_json::Value
        });

        let json_str =
            TokenStream::from_str(&format!("{:#}", v)).expect("must be valid tokenstream");
        self.init.push(quote! {
            #field: serde_json::json!(#json_str)
        });

        quote! {
            if !boon::gen::equals(v, &self.#field) {
                return false;
            }
        }
    }

    fn gen_enum(&mut self, sch: &Schema) -> TokenStream {
        let Some(Enum { values, .. }) = &sch.enum_ else {
            return TokenStream::new();
        };

        let field = format_ident!("enum{}", sch.idx.0);
        self.fields.push(quote! {
            #field: Vec<serde_json::Value>
        });

        let mut items = vec![];
        for v in values {
            let json_str =
                TokenStream::from_str(&format!("{:#}", v)).expect("must be valid tokenstream");
            items.push(quote! {
                serde_json::json!(#json_str)
            });
        }
        self.init.push(quote! {
            #field: vec![#(#items),*]
        });

        quote! {
            if !self.#field.iter().any(|e| boon::gen::equals(e, v)) {
                return false;
            }
        }
    }

    fn gen_format(&mut self, sch: &Schema) -> TokenStream {
        let Some(format) = &sch.format else {
            return TokenStream::new();
        };
        let func = format_ident!("validate_{}", format.name.replace('-', "_"));
        quote! {
            if boon::gen::#func(v).is_err() {
                return false;
            }
        }
    }

    fn gen_ref(&mut self, sch: &Schema) -> TokenStream {
        let Some(sch) = sch.ref_ else {
            return TokenStream::new();
        };

        let name = format_ident!("is_valid{}", sch.0);
        quote! {
            debug_assert!(true, "$ref");
            if !self.#name(v) {
                return false;
            }
        }
    }

    fn gen_not(&mut self, sch: &Schema) -> TokenStream {
        let Some(sch) = sch.not else {
            return TokenStream::new();
        };

        let name = format_ident!("is_valid{}", sch.0);
        quote! {
            debug_assert!(true, "not");
            if self.#name(v) {
                return false;
            }
        }
    }

    fn gen_allof(&mut self, sch: &Schema) -> TokenStream {
        if sch.all_of.is_empty() {
            return TokenStream::new();
        }
        let mut allof = vec![];
        for sch in &sch.all_of {
            let name = format_ident!("is_valid{}", sch.0);
            allof.push(quote! {
                self.#name(v)
            });
        }
        quote! {
            if #(!#allof)||* {
                return false;
            }
        }
    }

    fn gen_anyof(&mut self, sch: &Schema) -> TokenStream {
        if sch.any_of.is_empty() {
            return TokenStream::new();
        }
        let mut anyof = vec![];
        for sch in &sch.any_of {
            let name = format_ident!("is_valid{}", sch.0);
            anyof.push(quote! {
                self.#name(v)
            });
        }
        quote! {
            if #(!#anyof)&&* {
                return false;
            }
        }
    }

    fn gen_oneof(&mut self, sch: &Schema) -> TokenStream {
        if sch.one_of.is_empty() {
            return TokenStream::new();
        }
        let mut tokens = vec![];
        tokens.push(quote! {
            let mut oneof_matched = 0;
        });
        let len = sch.one_of.len();
        for (i, sch) in sch.one_of.iter().enumerate() {
            let name = format_ident!("is_valid{}", sch.0);
            tokens.push(quote! {
                if self.#name(v) {
                    oneof_matched += 1;
                }
            });
            if i > 0 && i != len - 1 {
                tokens.push(quote! {
                    if oneof_matched > 1 {
                        return false;
                    }
                });
            }
        }
        tokens.push(quote! {
            if oneof_matched != 1 {
                return false;
            }
        });
        TokenStream::from_iter(tokens)
    }

    fn gen_min_properties(&mut self, sch: &Schema) -> TokenStream {
        let Some(min) = sch.min_properties else {
            return TokenStream::new();
        };
        let min = min.into_token_stream();
        quote! {
            if obj.len() < #min {
                return false;
            }
        }
    }

    fn gen_max_properties(&mut self, sch: &Schema) -> TokenStream {
        let Some(max) = sch.max_properties else {
            return TokenStream::new();
        };
        let max = max.into_token_stream();
        quote! {
            if obj.len() > #max {
                return false;
            }
        }
    }

    fn gen_required(&mut self, sch: &Schema) -> TokenStream {
        if sch.required.is_empty() {
            return TokenStream::new();
        }

        let field = format_ident!("required{}", sch.idx.0);
        self.fields.push(quote! {
            #field: Vec<&'static str>
        });

        let mut props = vec![];
        for prop in &sch.required {
            props.push(prop.to_token_stream());
        }
        self.init.push(quote! {
            #field: vec![#(#props),*]
        });

        if sch.required.len() == 1 {
            let prop = sch.required[0].to_token_stream();
            quote! {
                if !obj.contains_key(#prop) {
                    return false;
                }
            }
        } else {
            quote! {
                if !self.#field.iter().all(|p| obj.contains_key(*p)) {
                    return false;
                }
            }
        }
    }

    fn gen_min_items(&mut self, sch: &Schema) -> TokenStream {
        let Some(min) = sch.min_items else {
            return TokenStream::new();
        };
        let min = min.into_token_stream();
        quote! {
            if arr.len() < #min {
                return false;
            }
        }
    }

    fn gen_max_items(&mut self, sch: &Schema) -> TokenStream {
        let Some(max) = sch.max_items else {
            return TokenStream::new();
        };
        let max = max.into_token_stream();
        quote! {
            if arr.len() > #max {
                return false;
            }
        }
    }

    fn gen_unique_items(&mut self, sch: &Schema) -> TokenStream {
        if !sch.unique_items {
            return TokenStream::new();
        };
        quote! {
            for i in 1..arr.len() {
                for j in 0..i {
                    if !boon::gen::equals(&arr[i], &arr[j]) {
                        return false;
                    }
                }
            }
        }
    }

    fn gen_items(&mut self, sch: &Schema) -> TokenStream {
        let Some(items) = &sch.items else {
            return TokenStream::new();
        };
        match items {
            Items::SchemaRef(sch) => {
                let name = format_ident!("is_valid{}", sch.0);
                quote! {
                    if !arr.iter().all(|item| self.#name(item)) {
                        return false;
                    }
                }
            }
            Items::SchemaRefs(list) => {
                let mut tokens = vec![];
                for (i, sch) in list.iter().enumerate() {
                    let name = format_ident!("is_valid{}", sch.0);
                    let i = i.into_token_stream();
                    tokens.push(quote! {
                        if arr.len()>#i {
                            if !self.#name(&arr[#i]) {
                                return false;
                            }
                        }
                    });
                }
                TokenStream::from_iter(tokens)
            }
        }
    }

    fn gen_additional_items(&mut self, sch: &Schema) -> TokenStream {
        let Some(additional) = &sch.additional_items else {
            return TokenStream::new();
        };
        let size = match &sch.items {
            None => 0,
            Some(Items::SchemaRef(_)) => return TokenStream::new(),
            Some(Items::SchemaRefs(list)) => list.len(),
        };
        let size = size.into_token_stream();
        match additional {
            Additional::Bool(true) => TokenStream::new(),
            Additional::Bool(false) => quote! {
                if arr.len()>#size {
                    return false;
                }
            },
            Additional::SchemaRef(sch) => {
                let name = format_ident!("is_valid{}", sch.0);
                quote! {
                    if arr.len()>#size {
                        for item in &arr[#size..] {
                            if !self.#name(item) {
                                return false;
                            }
                        }
                    }
                }
            }
        }
    }

    fn gen_length(&mut self, sch: &Schema) -> TokenStream {
        if sch.min_length.is_none() && sch.max_length.is_none() {
            return TokenStream::new();
        }
        let mut tokens = vec![quote! {
            let len = str.chars().count();
        }];
        if let Some(min) = sch.min_length {
            tokens.push(quote! {
                if len<#min {
                    return false;
                }
            });
        }
        if let Some(max) = sch.max_length {
            tokens.push(quote! {
                if len>#max {
                    return false;
                }
            });
        }
        TokenStream::from_iter(tokens)
    }

    fn gen_pattern(&mut self, sch: &Schema) -> TokenStream {
        let Some(regex) = &sch.pattern else {
            return TokenStream::new();
        };
        let field = format_ident!("pattern{}", sch.idx.0);
        self.fields.push(quote! {
            #field: regex::Regex
        });
        let str = regex.as_str();
        self.init.push(quote! {
            #field: regex::Regex::new(#str).expect("must be valid regex")
        });
        quote! {
            if !self.#field.is_match(str) {
                return false;
            }
        }
    }

    fn gen_num(&mut self, sch: &Schema) -> TokenStream {
        if sch.minimum.is_none()
            && sch.maximum.is_none()
            && sch.exclusive_minimum.is_none()
            && sch.exclusive_maximum.is_none()
            && sch.multiple_of.is_none()
        {
            return TokenStream::new();
        }
        let mut tokens = vec![];
        if let Some(min) = &sch.minimum {
            let field = format_ident!("minimum{}", sch.idx.0);
            self.fields.push(quote! {
                #field: serde_json::Number
            });
            let str = format!("{min}");
            self.init.push(quote! {
                #field: std::str::FromStr::from_str(#str).expect("must be valid number")
            });
            tokens.push(quote! {
                if let Some(minf) = self.#field.as_f64() {
                    if numf < minf {
                        return false;
                    }
                }
            });
        }
        if let Some(max) = &sch.maximum {
            let field = format_ident!("maximum{}", sch.idx.0);
            self.fields.push(quote! {
                #field: serde_json::Number
            });
            let str = format!("{max}");
            self.init.push(quote! {
                #field: std::str::FromStr::from_str(#str).expect("must be valid number")
            });
            tokens.push(quote! {
                if let Some(maxf) = self.#field.as_f64() {
                    if numf > maxf {
                        return false;
                    }
                }
            });
        }
        if let Some(ex_min) = &sch.exclusive_minimum {
            let field = format_ident!("exclusive_minimum{}", sch.idx.0);
            self.fields.push(quote! {
                #field: serde_json::Number
            });
            let str = format!("{ex_min}");
            self.init.push(quote! {
                #field: std::str::FromStr::from_str(#str).expect("must be valid number")
            });
            tokens.push(quote! {
                if let Some(ex_minf) = self.#field.as_f64() {
                    if numf <= ex_minf {
                        return false;
                    }
                }
            });
        }
        if let Some(ex_max) = &sch.exclusive_maximum {
            let field = format_ident!("exclusive_maximum{}", sch.idx.0);
            self.fields.push(quote! {
                #field: serde_json::Number
            });
            let str = format!("{ex_max}");
            self.init.push(quote! {
                #field: std::str::FromStr::from_str(#str).expect("must be valid number")
            });
            tokens.push(quote! {
                if let Some(ex_maxf) = self.#field.as_f64() {
                    if numf >= ex_maxf {
                        return false;
                    }
                }
            });
        }
        if let Some(mul) = &sch.multiple_of {
            let field = format_ident!("multiple_of{}", sch.idx.0);
            self.fields.push(quote! {
                #field: serde_json::Number
            });
            let str = format!("{mul}");
            self.init.push(quote! {
                #field: std::str::FromStr::from_str(#str).expect("must be valid number")
            });
            tokens.push(quote! {
                if let Some(mulf) = self.#field.as_f64() {
                    if (numf / mulf).fract() != 0.0 {
                        return false;
                    }
                }
            });
        }
        quote! {
            if let Some(numf) = num.as_f64() {
                #(#tokens)*
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use crate::{Compiler, Schemas};

    use super::*;

    #[test]
    fn test_gen() {
        let mut schemas = Schemas::new();
        let mut compiler = Compiler::new();
        let _sch = compiler.compile("test.json", &mut schemas).unwrap();
        let tokens = Generator::new("Schema").generate(&schemas);
        fs::write("../gen/src/lib.rs", format!("{}", tokens)).unwrap();
    }
}
