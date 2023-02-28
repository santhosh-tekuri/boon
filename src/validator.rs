use std::{borrow::Cow, cmp::min, collections::HashSet, fmt::Write};

use serde_json::Value;

use crate::{util::*, *};

pub(crate) fn validate(
    v: &Value,
    schema: &Schema,
    schemas: &Schemas,
) -> Result<(), ValidationError> {
    let scope = Scope {
        sch: schema.idx,
        kw_path: Cow::from(""),
        vid: 0,
        parent: None,
    };
    let mut vloc = String::new();
    let result = Validator {
        v,
        schema,
        schemas,
        scope,
        uneval: Uneval::from(v),
        errors: vec![],
    }
    .validate(JsonPointer::new(&mut vloc));
    match result {
        Err(e) => {
            let mut err = ValidationError {
                keyword_location: String::new(),
                absolute_keyword_location: schema.loc.clone(),
                instance_location: String::new(),
                kind: ErrorKind::Schema {
                    url: schema.loc.clone(),
                },
                causes: vec![],
            };
            if let ErrorKind::Group = e.kind {
                err.causes = e.causes;
            } else {
                err.causes.push(e);
            }
            Err(err)
        }
        Ok(_) => Ok(()),
    }
}

macro_rules! kind {
    ($kind:ident, $name:ident: $value:expr) => {
        ErrorKind::$kind { $name: $value }
    };
    ($kind:ident, $got:expr, $want:expr) => {
        ErrorKind::$kind {
            got: $got,
            want: $want,
        }
    };
    ($kind: ident) => {
        ErrorKind::$kind
    };
}

struct Validator<'v, 'a, 'b, 'd> {
    v: &'v Value,
    schema: &'a Schema,
    schemas: &'b Schemas,
    scope: Scope<'d>,
    uneval: Uneval<'v>,
    errors: Vec<ValidationError>,
}

impl<'v, 'a, 'b, 'd> Validator<'v, 'a, 'b, 'd> {
    fn validate(mut self, mut vloc: JsonPointer) -> Result<Uneval<'v>, ValidationError> {
        let s = self.schema;
        let v = self.v;

        if self.scope.has_cycle() {
            return Err(self.error("", &vloc, kind!(RefCycle)));
        }

        // boolean --
        if let Some(b) = s.boolean {
            if !b {
                return Err(self.error("", &vloc, kind!(FalseSchema)));
            }
            return Ok(self.uneval);
        }

        // type --
        if !s.types.is_empty() {
            let v_type = Type::of(v);
            let matched = self.schema.types.iter().any(|t| {
                if *t == Type::Integer && v_type == Type::Number {
                    if let Value::Number(n) = v {
                        return n.is_i64()
                            || n.is_u64()
                            || n.as_f64().filter(|n| n.fract() == 0.0).is_some();
                    }
                }
                *t == v_type
            });
            if !matched {
                self.add_error("type", &vloc, kind!(Type, v_type, s.types.clone()));
            }
        }

        // enum --
        if !s.enum_.is_empty() && !s.enum_.iter().any(|e| equals(e, v)) {
            self.add_error("enum", &vloc, kind!(Enum, v.clone(), s.enum_.clone()));
        }

        // constant --
        if let Some(c) = &s.constant {
            if !equals(v, c) {
                self.add_error("const", &vloc, kind!(Const, v.clone(), c.clone()));
            }
        }

        // format --
        if let Some((format, check)) = &s.format {
            if let Err(e) = check(v) {
                let kind = ErrorKind::Format {
                    got: v.clone(),
                    want: format.clone(),
                    reason: e.to_string(),
                };
                self.add_error("format", &vloc, kind);
            }
        }

        self.obj_validate(vloc.copy());
        self.arr_validate(vloc.copy());
        self.str_validate(vloc.copy());
        self.num_validate(vloc.copy());

        self.refs_validate(vloc.copy());
        self.cond_validate(vloc.copy());
        self.uneval_validate(vloc.copy());

        match self.errors.len() {
            0 => Ok(self.uneval),
            1 => Err(self.errors.remove(0)),
            _ => {
                let mut e = self.error("", &vloc, kind!(Group));
                e.causes = self.errors;
                Err(e)
            }
        }
    }
}

// type specific validations
impl<'v, 'a, 'b, 'd> Validator<'v, 'a, 'b, 'd> {
    fn obj_validate(&mut self, mut vloc: JsonPointer) {
        let Value::Object(obj) = self.v else {
            return;
        };

        let s = self.schema;
        macro_rules! add_err {
            ($result:expr) => {
                let result = $result;
                self.errors.extend(result.err().into_iter());
            };
        }

        // minProperties --
        if let Some(min) = s.min_properties {
            if obj.len() < min {
                self.add_error("minProperties", &vloc, kind!(MinProperties, obj.len(), min));
            }
        }

        // maxProperties --
        if let Some(max) = s.max_properties {
            if obj.len() > max {
                self.add_error("maxProperties", &vloc, kind!(MaxProperties, obj.len(), max));
            }
        }

        // required --
        let missing = s
            .required
            .iter()
            .filter(|p| !obj.contains_key(p.as_str()))
            .cloned()
            .collect::<Vec<String>>();
        if !missing.is_empty() {
            self.add_error("required", &vloc, kind!(Required, want: missing));
        }

        // dependencies --
        for (pname, dependency) in &s.dependencies {
            if obj.contains_key(pname) {
                let kw_path = format!("dependencies/{}", escape(pname));
                match dependency {
                    Dependency::Props(required) => {
                        let missing = required
                            .iter()
                            .filter(|p| !obj.contains_key(p.as_str()))
                            .cloned()
                            .collect::<Vec<String>>();
                        if !missing.is_empty() {
                            self.add_error(
                                &kw_path,
                                &vloc,
                                kind!(DependentRequired, pname.clone(), missing),
                            );
                        }
                    }
                    Dependency::SchemaRef(sch) => {
                        add_err!(self.validate_self(*sch, kw_path.into(), vloc.copy()));
                    }
                }
            }
        }

        // dependentSchemas --
        for (pname, sch) in &s.dependent_schemas {
            if obj.contains_key(pname) {
                let kw_path = format!("dependentSchemas/{}", escape(pname));
                add_err!(self.validate_self(*sch, kw_path.into(), vloc.copy()));
            }
        }

        // dependentRequired --
        for (pname, required) in &s.dependent_required {
            if obj.contains_key(pname) {
                let missing = required
                    .iter()
                    .filter(|p| !obj.contains_key(p.as_str()))
                    .cloned()
                    .collect::<Vec<String>>();
                if !missing.is_empty() {
                    let kind = kind!(DependentRequired, pname.clone(), missing);
                    self.add_error(&format!("dependentRequired/{}", escape(pname)), &vloc, kind);
                }
            }
        }

        // properties --
        for (pname, &psch) in &s.properties {
            if let Some(pvalue) = obj.get(pname) {
                self.uneval.props.remove(pname);
                let kw_path = format!("properties/{}", escape(pname));
                add_err!(self.validate_val(psch, kw_path.into(), pvalue, vloc.prop(pname)));
            }
        }

        // patternProperties --
        for (regex, psch) in &s.pattern_properties {
            for (pname, pvalue) in obj.iter().filter(|(pname, _)| regex.is_match(pname)) {
                self.uneval.props.remove(pname);
                let kw_path = format!("patternProperties/{}", escape(regex.as_str()));
                add_err!(self.validate_val(*psch, kw_path.into(), pvalue, vloc.prop(pname)));
            }
        }

        // propertyNames --
        if let Some(sch) = &s.property_names {
            for pname in obj.keys() {
                let v = Value::String(pname.to_owned());
                add_err!(self.validate_val(*sch, "propertyNames".into(), &v, vloc.prop(pname)));
            }
        }

        // additionalProperties --
        if let Some(additional) = &s.additional_properties {
            let kw_path = "additionalProperties";
            match additional {
                Additional::Bool(allowed) => {
                    if !allowed && !self.uneval.props.is_empty() {
                        let kind = kind!(AdditionalProperties, got: self.uneval.props.iter().cloned().cloned().collect());
                        self.add_error(kw_path, &vloc, kind);
                    }
                }
                Additional::SchemaRef(sch) => {
                    for &pname in self.uneval.props.iter() {
                        if let Some(pvalue) = obj.get(pname) {
                            let result =
                                self.validate_val(*sch, kw_path.into(), pvalue, vloc.prop(pname));
                            add_err!(result);
                        }
                    }
                }
            }
            self.uneval.props.clear();
        }
    }

    fn arr_validate(&mut self, mut vloc: JsonPointer) {
        let Value::Array(arr) = self.v else {
            return;
        };

        let s = self.schema;
        macro_rules! add_err {
            ($result:expr) => {
                let result = $result;
                self.errors.extend(result.err().into_iter());
            };
        }

        // minItems --
        if let Some(min) = s.min_items {
            if arr.len() < min {
                self.add_error("minItems", &vloc, kind!(MinItems, arr.len(), min));
            }
        }

        // maxItems --
        if let Some(max) = s.max_items {
            if arr.len() > max {
                self.add_error("maxItems", &vloc, kind!(MaxItems, arr.len(), max));
            }
        }

        // uniqueItems --
        if s.unique_items {
            for i in 1..arr.len() {
                for j in 0..i {
                    if equals(&arr[i], &arr[j]) {
                        self.add_error("uniqueItems", &vloc, kind!(UniqueItems, got: [j, i]));
                    }
                }
            }
        }

        // items --
        if let Some(items) = &s.items {
            match items {
                Items::SchemaRef(sch) => {
                    for (i, item) in arr.iter().enumerate() {
                        add_err!(self.validate_val(*sch, "items".into(), item, vloc.item(i)));
                    }
                    self.uneval.items.clear();
                }
                Items::SchemaRefs(list) => {
                    for (i, (item, sch)) in arr.iter().zip(list).enumerate() {
                        self.uneval.items.remove(&i);
                        let kw_path = format!("items/{i}");
                        add_err!(self.validate_val(*sch, kw_path.into(), item, vloc.item(i)));
                    }
                }
            }
        }

        // additionalItems --
        if let Some(additional) = &s.additional_items {
            let kw_path = "additionalItems";
            match additional {
                Additional::Bool(allowed) => {
                    if !allowed && !self.uneval.items.is_empty() {
                        let kind = kind!(AdditionalItems, got: arr.len() - self.uneval.items.len());
                        self.add_error(kw_path, &vloc, kind);
                    }
                }
                Additional::SchemaRef(sch) => {
                    let from = arr.len() - self.uneval.items.len();
                    for (i, item) in arr[from..].iter().enumerate() {
                        add_err!(self.validate_val(*sch, kw_path.into(), item, vloc.item(i)));
                    }
                }
            }
            self.uneval.items.clear();
        }

        // prefixItems --
        for (i, (sch, item)) in s.prefix_items.iter().zip(arr).enumerate() {
            self.uneval.items.remove(&i);
            let kw_path = format!("prefixItems/{i}");
            add_err!(self.validate_val(*sch, kw_path.into(), item, vloc.item(i)));
        }

        // items2020 --
        if let Some(sch) = &s.items2020 {
            let from = min(arr.len(), s.prefix_items.len());
            for (i, item) in arr[from..].iter().enumerate() {
                add_err!(self.validate_val(*sch, "items".into(), item, vloc.item(i)));
            }
            self.uneval.items.clear();
        }

        // contains --
        let mut contains_matched = vec![];
        let mut contains_errors = vec![];
        if let Some(sch) = &s.contains {
            for (i, item) in arr.iter().enumerate() {
                if let Err(e) = self.validate_val(*sch, "contains".into(), item, vloc.item(i)) {
                    contains_errors.push(e);
                } else {
                    contains_matched.push(i);
                    if s.draft_version >= 2020 {
                        self.uneval.items.remove(&i);
                    }
                }
            }
        }

        // minContains --
        if let Some(min) = s.min_contains {
            if contains_matched.len() < min {
                let kind = kind!(MinContains, contains_matched.clone(), min);
                let mut e = self.error("minContains", &vloc, kind);
                e.causes = contains_errors;
                self.errors.push(e);
            }
        } else if s.contains.is_some() && contains_matched.is_empty() {
            let mut e = self.error("contains", &vloc, kind!(Contains));
            e.causes = contains_errors;
            self.errors.push(e);
        }

        // maxContains --
        if let Some(max) = s.max_contains {
            if contains_matched.len() > max {
                let kind = kind!(MaxContains, contains_matched, max);
                self.add_error("maxContains", &vloc, kind);
            }
        }
    }

    fn str_validate(&mut self, vloc: JsonPointer) {
        let Value::String(str) = self.v else {
            return;
        };

        let s = self.schema;
        let mut len = None;

        // minLength --
        if let Some(min) = s.min_length {
            let len = len.get_or_insert_with(|| str.chars().count());
            if *len < min {
                self.add_error("minLength", &vloc, kind!(MinLength, *len, min));
            }
        }

        // maxLength --
        if let Some(max) = s.max_length {
            let len = len.get_or_insert_with(|| str.chars().count());
            if *len > max {
                self.add_error("maxLength", &vloc, kind!(MaxLength, *len, max));
            }
        }

        // pattern --
        if let Some(regex) = &s.pattern {
            if !regex.is_match(str) {
                let kind = kind!(Pattern, str.clone(), regex.as_str().to_string());
                self.add_error("pattern", &vloc, kind);
            }
        }

        // contentEncoding --
        let mut decoded = Cow::from(str.as_bytes());
        if let Some((encoding, decode)) = &s.content_encoding {
            match decode(str) {
                Some(bytes) => decoded = Cow::from(bytes),
                None => {
                    let kind = kind!(ContentEncoding, str.clone(), encoding.clone());
                    self.add_error("contentEncoding", &vloc, kind)
                }
            }
        }

        // contentMediaType --
        if let Some((media_type, check)) = &s.content_media_type {
            if !check(decoded.as_ref()) {
                let kind = kind!(ContentMediaType, decoded.into_owned(), media_type.clone());
                self.add_error("contentMediaType", &vloc, kind);
            }
        }
    }

    fn num_validate(&mut self, vloc: JsonPointer) {
        let Value::Number(num) = self.v else {
            return;
        };

        let s = self.schema;

        // minimum --
        if let Some(min) = &s.minimum {
            if let (Some(minf), Some(numf)) = (min.as_f64(), num.as_f64()) {
                if numf < minf {
                    self.add_error("minimum", &vloc, kind!(Minimum, num.clone(), min.clone()));
                }
            }
        }

        // maximum --
        if let Some(max) = &s.maximum {
            if let (Some(maxf), Some(numf)) = (max.as_f64(), num.as_f64()) {
                if numf > maxf {
                    self.add_error("maximum", &vloc, kind!(Maximum, num.clone(), max.clone()));
                }
            }
        }

        // exclusiveMinimum --
        if let Some(ex_min) = &s.exclusive_minimum {
            if let (Some(ex_minf), Some(numf)) = (ex_min.as_f64(), num.as_f64()) {
                if numf <= ex_minf {
                    let kind = kind!(ExclusiveMinimum, num.clone(), ex_min.clone());
                    self.add_error("exclusiveMinimum", &vloc, kind);
                }
            }
        }

        // exclusiveMaximum --
        if let Some(ex_max) = &s.exclusive_maximum {
            if let (Some(ex_maxf), Some(numf)) = (ex_max.as_f64(), num.as_f64()) {
                if numf >= ex_maxf {
                    let kind = kind!(ExclusiveMaximum, num.clone(), ex_max.clone());
                    self.add_error("exclusiveMaximum", &vloc, kind);
                }
            }
        }

        // multipleOf --
        if let Some(mul) = &s.multiple_of {
            if let (Some(mulf), Some(numf)) = (mul.as_f64(), num.as_f64()) {
                if (numf / mulf).fract() != 0.0 {
                    let kind = kind!(MultipleOf, num.clone(), mul.clone());
                    self.add_error("multipleOf", &vloc, kind);
                }
            }
        }
    }
}

// references validation
impl<'v, 'a, 'b, 'd> Validator<'v, 'a, 'b, 'd> {
    fn refs_validate(&mut self, mut vloc: JsonPointer) {
        let s = self.schema;
        macro_rules! add_err {
            ($result:expr) => {
                let result = $result;
                self.errors.extend(result.err().into_iter());
            };
        }

        // $ref --
        if let Some(ref_) = s.ref_ {
            add_err!(self.validate_ref(ref_, "$ref", vloc.copy()));
        }

        // $recursiveRef --
        if let Some(mut sch) = s.recursive_ref {
            if self.schemas.get(sch).recursive_anchor {
                sch = self.resolve_recursive_anchor().unwrap_or(sch);
            }
            add_err!(self.validate_ref(sch, "$recursiveRef", vloc.copy()));
        }

        // $dynamicRef --
        if let Some(mut sch) = s.dynamic_ref {
            if let Some(name) = &self.schemas.get(sch).dynamic_anchor {
                sch = self.resolve_dynamic_anchor(name).unwrap_or(sch);
            }
            add_err!(self.validate_ref(sch, "$dynamicRef", vloc.copy()));
        }
    }

    fn validate_ref(
        &mut self,
        sch: SchemaIndex,
        kw: &'static str,
        mut vloc: JsonPointer,
    ) -> Result<(), ValidationError> {
        if let Err(ref_err) = self.validate_self(sch, kw.into(), vloc.copy()) {
            let mut err = self.error(
                kw,
                &vloc,
                ErrorKind::Reference {
                    url: self.schemas.get(sch).loc.clone(),
                },
            );
            if let ErrorKind::Group = ref_err.kind {
                err.causes = ref_err.causes;
            } else {
                err.causes.push(ref_err);
            }
            return Err(err);
        }
        Ok(())
    }

    fn resolve_recursive_anchor(&self) -> Option<SchemaIndex> {
        let mut scope = &self.scope;
        let mut sch = None;
        loop {
            let scope_sch = self.schemas.get(scope.sch);
            let base_sch = self.schemas.get(scope_sch.resource);
            if base_sch.recursive_anchor {
                sch.replace(scope.sch);
            }
            if let Some(parent) = scope.parent {
                scope = parent;
            } else {
                return sch;
            }
        }
    }

    fn resolve_dynamic_anchor(&self, name: &String) -> Option<SchemaIndex> {
        let mut scope = &self.scope;
        let mut sch = None;
        loop {
            let scope_sch = self.schemas.get(scope.sch);
            let base_sch = self.schemas.get(scope_sch.resource);
            debug_assert_eq!(base_sch.idx, base_sch.resource);
            if let Some(dsch) = base_sch.dynamic_anchors.get(name) {
                sch.replace(*dsch);
            }
            if let Some(parent) = scope.parent {
                scope = parent;
            } else {
                return sch;
            }
        }
    }
}

// conditional validation
impl<'v, 'a, 'b, 'd> Validator<'v, 'a, 'b, 'd> {
    fn cond_validate(&mut self, mut vloc: JsonPointer) {
        let s = self.schema;
        macro_rules! add_err {
            ($result:expr) => {
                let result = $result;
                self.errors.extend(result.err().into_iter());
            };
        }

        // not --
        if let Some(not) = s.not {
            if self.validate_self(not, "not".into(), vloc.copy()).is_ok() {
                self.add_error("not", &vloc, kind!(Not));
            }
        }

        // allOf --
        if !s.all_of.is_empty() {
            let (mut failed, mut allof_errors) = (vec![], vec![]);
            for (i, sch) in s.all_of.iter().enumerate() {
                let kw_path = format!("allOf/{i}");
                if let Err(e) = self.validate_self(*sch, kw_path.into(), vloc.copy()) {
                    failed.push(i);
                    allof_errors.push(e);
                }
            }
            if !failed.is_empty() {
                self.add_errors(allof_errors, "allOf", &vloc, kind!(AllOf, got: failed));
            }
        }

        // anyOf --
        if !s.any_of.is_empty() {
            // NOTE: all schemas must be checked
            let mut anyof_errors = vec![];
            for (i, sch) in s.any_of.iter().enumerate() {
                let kw_path = format!("anyOf/{i}");
                if let Err(e) = self.validate_self(*sch, kw_path.into(), vloc.copy()) {
                    anyof_errors.push(e);
                }
            }
            if anyof_errors.len() == s.any_of.len() {
                // none matched
                self.add_errors(anyof_errors, "anyOf", &vloc, kind!(AnyOf));
            }
        }

        // oneOf --
        if !s.one_of.is_empty() {
            let (mut matched, mut oneof_errors) = (vec![], vec![]);
            for (i, sch) in s.one_of.iter().enumerate() {
                let kw_path = format!("oneOf/{i}");
                if let Err(e) = self.validate_self(*sch, kw_path.into(), vloc.copy()) {
                    oneof_errors.push(e);
                } else {
                    matched.push(i);
                    if matched.len() == 2 {
                        break;
                    }
                }
            }
            if matched.is_empty() {
                // none matched
                self.add_errors(oneof_errors, "oneOf", &vloc, kind!(OneOf, got: matched));
            } else if matched.len() > 1 {
                self.add_error("oneOf", &vloc, kind!(OneOf, got: matched));
            }
        }

        // if, then, else --
        if let Some(if_) = s.if_ {
            if self.validate_self(if_, "if".into(), vloc.copy()).is_ok() {
                if let Some(then) = s.then {
                    add_err!(self.validate_self(then, "then".into(), vloc.copy()));
                }
            } else if let Some(else_) = s.else_ {
                add_err!(self.validate_self(else_, "else".into(), vloc.copy()));
            }
        }
    }
}

// conditional validation
impl<'v, 'a, 'b, 'd> Validator<'v, 'a, 'b, 'd> {
    fn uneval_validate(&mut self, mut vloc: JsonPointer) {
        let s = self.schema;
        let v = self.v;
        macro_rules! add_err {
            ($result:expr) => {
                let result = $result;
                self.errors.extend(result.err().into_iter());
            };
        }

        // unevaluatedProps --
        if let (Some(sch), Value::Object(obj)) = (s.unevaluated_properties, v) {
            for pname in &self.uneval.props {
                if let Some(pvalue) = obj.get(*pname) {
                    let kw_path = "unevaluatedProperties";
                    add_err!(self.validate_val(sch, kw_path.into(), pvalue, vloc.prop(pname)));
                }
            }
            self.uneval.props.clear();
        }

        // unevaluatedItems --
        if let (Some(sch), Value::Array(arr)) = (s.unevaluated_items, v) {
            for i in &self.uneval.items {
                if let Some(pvalue) = arr.get(*i) {
                    let kw_path = "unevaluatedItems";
                    add_err!(self.validate_val(sch, kw_path.into(), pvalue, vloc.item(*i)));
                }
            }
            self.uneval.items.clear();
        }
    }
}

// validation helpers --
impl<'v, 'a, 'b, 'd> Validator<'v, 'a, 'b, 'd> {
    fn validate_val(
        &self,
        sch: SchemaIndex,
        kw_path: Cow<'static, str>,
        v: &Value,
        vloc: JsonPointer,
    ) -> Result<(), ValidationError> {
        let scope = Scope::child(sch, kw_path, self.scope.vid + 1, &self.scope);
        Validator {
            v,
            schema: self.schemas.get(sch),
            schemas: self.schemas,
            scope,
            uneval: Uneval::from(v),
            errors: vec![],
        }
        .validate(vloc)
        .map(|_| ())
    }

    fn validate_self(
        &mut self,
        sch: SchemaIndex,
        kw_path: Cow<'static, str>,
        vloc: JsonPointer,
    ) -> Result<(), ValidationError> {
        let scope = Scope::child(sch, kw_path, self.scope.vid, &self.scope);
        let result = Validator {
            v: self.v,
            schema: self.schemas.get(sch),
            schemas: self.schemas,
            scope,
            uneval: Uneval::from(self.v),
            errors: vec![],
        }
        .validate(vloc);
        if let Ok(reply) = &result {
            self.uneval.merge(reply);
        }
        result.map(|_| ())
    }
}

// error helpers
impl<'v, 'a, 'b, 'd> Validator<'v, 'a, 'b, 'd> {
    fn error(&self, kw_path: &str, vloc: &JsonPointer, kind: ErrorKind) -> ValidationError {
        ValidationError {
            keyword_location: self.scope.kw_loc(kw_path),
            absolute_keyword_location: match kw_path.is_empty() {
                true => self.schema.loc.clone(),
                false => format!("{}/{kw_path}", self.schema.loc),
            },
            instance_location: vloc.to_string(),
            kind,
            causes: vec![],
        }
    }

    fn add_error(&mut self, kw_path: &str, vloc: &JsonPointer, kind: ErrorKind) {
        self.errors.push(self.error(kw_path, vloc, kind));
    }

    fn add_errors(
        &mut self,
        errors: Vec<ValidationError>,
        kw_path: &str,
        vloc: &JsonPointer,
        kind: ErrorKind,
    ) {
        if errors.len() == 1 {
            self.errors.extend(errors);
        } else {
            let mut err = self.error(kw_path, vloc, kind);
            err.causes = errors;
            self.errors.push(err);
        }
    }
}

// Uneval --

#[derive(Default)]
struct Uneval<'v> {
    props: HashSet<&'v String>,
    items: HashSet<usize>,
}

impl<'v> Uneval<'v> {
    fn merge(&mut self, other: &Uneval) {
        self.props.retain(|p| other.props.contains(p));
        self.items.retain(|i| other.items.contains(i));
    }
}

impl<'v> From<&'v Value> for Uneval<'v> {
    fn from(v: &'v Value) -> Self {
        let mut uneval = Self::default();
        match v {
            Value::Object(obj) => uneval.props = obj.keys().collect(),
            Value::Array(arr) => uneval.items = (0..arr.len()).collect(),
            _ => (),
        }
        uneval
    }
}

// Scope ---

#[derive(Debug, Default)]
struct Scope<'a> {
    sch: SchemaIndex,
    kw_path: Cow<'static, str>,
    /// unique id of value being validated
    // if two scope validate same value, they will have same vid
    vid: usize,
    parent: Option<&'a Scope<'a>>,
}

impl<'a> Scope<'a> {
    fn child(sch: SchemaIndex, kw_path: Cow<'static, str>, vid: usize, parent: &'a Scope) -> Self {
        Self {
            sch,
            kw_path,
            vid,
            parent: Some(parent),
        }
    }

    fn kw_loc(&self, kw_path: &str) -> String {
        let mut loc = kw_path.to_string();
        let mut scope = self;
        loop {
            if !loc.is_empty() {
                loc.insert(0, '/');
            }
            loc.insert_str(0, scope.kw_path.as_ref());
            if let Some(parent) = scope.parent {
                scope = parent;
            } else {
                break;
            }
        }
        loc
    }

    fn has_cycle(&self) -> bool {
        let mut scope = self.parent;
        while let Some(scp) = scope {
            if scp.vid != self.vid {
                break;
            }
            if scp.sch == self.sch {
                return true;
            }
            scope = scp.parent;
        }
        false
    }
}

// JsonPointer --

struct JsonPointer<'a> {
    str: &'a mut String,
    len: usize,
}

impl<'a> JsonPointer<'a> {
    fn new(str: &'a mut String) -> Self {
        let len = str.len();
        Self { str, len }
    }

    fn as_str(&self) -> &str {
        &self.str[..self.len]
    }

    fn copy(&mut self) -> JsonPointer {
        JsonPointer {
            str: self.str,
            len: self.len,
        }
    }

    fn prop(&mut self, name: &str) -> JsonPointer {
        self.str.truncate(self.len);
        self.str.push('/');
        self.str.push_str(&escape(name));
        JsonPointer::new(self.str)
    }

    fn item(&mut self, i: usize) -> JsonPointer {
        self.str.truncate(self.len);
        self.str.push('/');
        write!(self.str, "{i}").expect("write to String should never fail"); // todo: can itoa create better perform
        JsonPointer::new(self.str)
    }
}

impl<'a> ToString for JsonPointer<'a> {
    fn to_string(&self) -> String {
        self.as_str().to_string()
    }
}
