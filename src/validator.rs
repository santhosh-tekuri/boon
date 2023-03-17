use std::{borrow::Cow, cmp::min, collections::HashSet, fmt::Write};

use serde_json::{Map, Value};

use crate::{util::*, *};

pub(crate) fn validate(
    v: &Value,
    schema: &Schema,
    schemas: &Schemas,
) -> Result<(), ValidationError> {
    let scope = Scope {
        sch: schema.idx,
        kw_path: None,
        vid: 0,
        parent: None,
    };
    let mut vloc = String::new();
    let result = Validator {
        v,
        schema,
        schemas,
        scope,
        uneval: Uneval::from(v, schema, false),
        errors: vec![],
    }
    .validate(JsonPointer::new(&mut vloc));
    match result {
        Err(mut e) => {
            if e.keyword_location.is_empty()
                && e.instance_location.is_empty()
                && matches!(e.kind, ErrorKind::Group)
            {
                e.kind = ErrorKind::Schema {
                    url: schema.loc.clone(),
                };
            } else {
                e = ValidationError {
                    keyword_location: String::new(),
                    absolute_keyword_location: schema.loc.clone(),
                    instance_location: String::new(),
                    kind: ErrorKind::Schema {
                        url: schema.loc.clone(),
                    },
                    causes: vec![e],
                };
            }
            Err(e)
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
    ($kind:ident, $got:expr, $want:expr, $err:expr) => {
        ErrorKind::$kind {
            got: $got,
            want: $want,
            err: $err,
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

        if let Some(scp) = self.scope.check_cycle() {
            let kind = ErrorKind::RefCycle {
                url: self.schema.loc.clone(),
                kw_loc1: self.kw_loc(&self.scope, ""),
                kw_loc2: self.kw_loc(scp, ""),
            };
            return Err(self.error("", &vloc, kind));
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
            let matched = s.types.contains(v_type) || {
                if let Value::Number(n) = v {
                    s.types.contains(Type::Integer) && is_integer(n)
                } else {
                    false
                }
            };
            if !matched {
                self.add_error("/type", &vloc, kind!(Type, v_type, s.types));
            }
        }

        // enum --
        if !s.enum_.is_empty() && !s.enum_.iter().any(|e| equals(e, v)) {
            self.add_error("/enum", &vloc, kind!(Enum, v.clone(), s.enum_.clone()));
        }

        // constant --
        if let Some(c) = &s.constant {
            if !equals(v, c) {
                self.add_error("/const", &vloc, kind!(Const, v.clone(), c.clone()));
            }
        }

        // format --
        if let Some(format) = &s.format {
            if let Err(e) = (format.func)(v) {
                let kind = kind!(Format, v.clone(), format.name, e);
                self.add_error("/format", &vloc, kind);
            }
        }

        match v {
            Value::Object(obj) => self.obj_validate(obj, vloc.copy()),
            Value::Array(arr) => self.arr_validate(arr, vloc.copy()),
            Value::String(str) => self.str_validate(str, vloc.copy()),
            Value::Number(num) => self.num_validate(num, vloc.copy()),
            _ => {}
        }

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
    fn obj_validate(&mut self, obj: &Map<String, Value>, mut vloc: JsonPointer) {
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
                let kind = kind!(MinProperties, obj.len(), min);
                self.add_error("/minProperties", &vloc, kind);
            }
        }

        // maxProperties --
        if let Some(max) = s.max_properties {
            if obj.len() > max {
                let kind = kind!(MaxProperties, obj.len(), max);
                self.add_error("/maxProperties", &vloc, kind);
            }
        }

        // propertyNames --
        if let Some(sch) = &s.property_names {
            for pname in obj.keys() {
                let v = Value::String(pname.to_owned());
                add_err!(self.validate_val(*sch, &v, vloc.prop(pname)));
            }
        }

        let find_missing = |required: &Vec<String>| {
            required
                .iter()
                .filter(|p| !obj.contains_key(p.as_str()))
                .cloned()
                .collect::<Vec<String>>()
        };

        // required --
        let missing = find_missing(&s.required);
        if !missing.is_empty() {
            self.add_error("/required", &vloc, kind!(Required, want: missing));
        }

        // dependencies --
        for (pname, dependency) in &s.dependencies {
            if obj.contains_key(pname) {
                match dependency {
                    Dependency::Props(required) => {
                        let missing = find_missing(required);
                        if !missing.is_empty() {
                            let kw_path = format!("/dependencies/{}", escape(pname));
                            let kind = kind!(Dependency, pname.clone(), missing);
                            self.add_error(&kw_path, &vloc, kind);
                        }
                    }
                    Dependency::SchemaRef(sch) => {
                        if let Err(e) = self.validate_self(*sch, None, vloc.copy()) {
                            if let ErrorKind::Group = e.kind {
                                let kw_path = format!("/dependencies/{}", escape(pname));
                                let kind = kind!(Dependency, pname.clone(), vec![]);
                                self.add_errors(e.causes, &kw_path, &vloc, kind);
                            } else {
                                self.errors.push(e);
                            };
                        }
                    }
                }
            }
        }

        // dependentSchemas --
        for (pname, sch) in &s.dependent_schemas {
            if obj.contains_key(pname) {
                if let Err(e) = self.validate_self(*sch, None, vloc.copy()) {
                    if let ErrorKind::Group = e.kind {
                        let kw_path = format!("/dependentSchemas/{}", escape(pname));
                        let kind = kind!(DependentSchemas, got:pname.clone());
                        self.add_errors(e.causes, &kw_path, &vloc, kind);
                    } else {
                        self.errors.push(e);
                    };
                }
            }
        }

        // dependentRequired --
        for (pname, required) in &s.dependent_required {
            if obj.contains_key(pname) {
                let missing = find_missing(required);
                if !missing.is_empty() {
                    let kw_path = format!("/dependentRequired/{}", escape(pname));
                    let kind = kind!(DependentRequired, pname.clone(), missing);
                    self.add_error(&kw_path, &vloc, kind);
                }
            }
        }

        for (pname, pvalue) in obj {
            let mut evaluated = false;

            // properties --
            if let Some(&sch) = s.properties.get(pname) {
                match self.validate_val(sch, pvalue, vloc.prop(pname)) {
                    Ok(_) => evaluated = true,
                    Err(e) => self.errors.push(e),
                }
            }

            // patternProperties --
            for (regex, sch) in &s.pattern_properties {
                if regex.is_match(pname) {
                    match self.validate_val(*sch, pvalue, vloc.prop(pname)) {
                        Ok(_) => evaluated = true,
                        Err(e) => self.errors.push(e),
                    }
                }
            }

            if !evaluated {
                // additionalProperties --
                if let Some(additional) = &s.additional_properties {
                    match additional {
                        Additional::Bool(allowed) => {
                            if !allowed {
                                let kind = kind!(AdditionalProperties, got: self.uneval.props.iter().cloned().cloned().collect());
                                self.add_error("/additionalProperties", &vloc, kind);
                            }
                        }
                        Additional::SchemaRef(sch) => {
                            add_err!(self.validate_val(*sch, pvalue, vloc.prop(pname)));
                        }
                    }
                    evaluated = true;
                }
            }

            if evaluated {
                self.uneval.props.remove(pname);
            }
        }
    }

    fn arr_validate(&mut self, arr: &Vec<Value>, mut vloc: JsonPointer) {
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
                self.add_error("/minItems", &vloc, kind!(MinItems, arr.len(), min));
            }
        }

        // maxItems --
        if let Some(max) = s.max_items {
            if arr.len() > max {
                self.add_error("/maxItems", &vloc, kind!(MaxItems, arr.len(), max));
            }
        }

        // uniqueItems --
        if s.unique_items {
            'outer: for i in 1..arr.len() {
                for j in 0..i {
                    if equals(&arr[i], &arr[j]) {
                        self.add_error("/uniqueItems", &vloc, kind!(UniqueItems, got: [j, i]));
                        break 'outer;
                    }
                }
            }
        }

        if s.draft_version < 2020 {
            let mut evaluated = 0;

            // items --
            if let Some(items) = &s.items {
                match items {
                    Items::SchemaRef(sch) => {
                        for (i, item) in arr.iter().enumerate() {
                            if let Err(mut e) = self.validate_val(*sch, item, vloc.item(i)) {
                                if let ErrorKind::Group = e.kind {
                                    e.kind = kind!(Items);
                                }
                                self.errors.push(e);
                            }
                        }
                        evaluated = arr.len();
                        debug_assert!(self.uneval.items.is_empty());
                    }
                    Items::SchemaRefs(list) => {
                        for (i, (item, sch)) in arr.iter().zip(list).enumerate() {
                            self.uneval.items.remove(&i);
                            add_err!(self.validate_val(*sch, item, vloc.item(i)));
                        }
                        evaluated = min(list.len(), arr.len());
                    }
                }
            }

            // additionalItems --
            if let Some(additional) = &s.additional_items {
                match additional {
                    Additional::Bool(allowed) => {
                        if !allowed && evaluated != arr.len() {
                            let kind = kind!(AdditionalItems, got: arr.len() - evaluated);
                            self.add_error("/additionalItems", &vloc, kind);
                        }
                    }
                    Additional::SchemaRef(sch) => {
                        for (i, item) in arr[evaluated..].iter().enumerate() {
                            add_err!(self.validate_val(*sch, item, vloc.item(i)));
                        }
                    }
                }
                debug_assert!(self.uneval.items.is_empty());
            }
        } else {
            // prefixItems --
            for (i, (sch, item)) in s.prefix_items.iter().zip(arr).enumerate() {
                self.uneval.items.remove(&i);
                add_err!(self.validate_val(*sch, item, vloc.item(i)));
            }

            // items2020 --
            if let Some(sch) = &s.items2020 {
                let evaluated = min(s.prefix_items.len(), arr.len());
                for (i, item) in arr[evaluated..].iter().enumerate() {
                    if let Err(mut e) = self.validate_val(*sch, item, vloc.item(i)) {
                        if let ErrorKind::Group = e.kind {
                            e.kind = kind!(Items);
                        }
                        self.errors.push(e);
                    }
                }
                debug_assert!(self.uneval.items.is_empty());
            }
        }

        // contains --
        let mut contains_matched = vec![];
        let mut contains_errors = vec![];
        if let Some(sch) = &s.contains {
            for (i, item) in arr.iter().enumerate() {
                if let Err(e) = self.validate_val(*sch, item, vloc.item(i)) {
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
                let mut e = self.error("/minContains", &vloc, kind);
                e.causes = contains_errors;
                self.errors.push(e);
            }
        } else if s.contains.is_some() && contains_matched.is_empty() {
            let mut e = self.error("/contains", &vloc, kind!(Contains));
            e.causes = contains_errors;
            self.errors.push(e);
        }

        // maxContains --
        if let Some(max) = s.max_contains {
            if contains_matched.len() > max {
                let kind = kind!(MaxContains, contains_matched, max);
                self.add_error("/maxContains", &vloc, kind);
            }
        }
    }

    fn str_validate(&mut self, str: &String, vloc: JsonPointer) {
        let s = self.schema;
        let mut len = None;

        // minLength --
        if let Some(min) = s.min_length {
            let len = len.get_or_insert_with(|| str.chars().count());
            if *len < min {
                self.add_error("/minLength", &vloc, kind!(MinLength, *len, min));
            }
        }

        // maxLength --
        if let Some(max) = s.max_length {
            let len = len.get_or_insert_with(|| str.chars().count());
            if *len > max {
                self.add_error("/maxLength", &vloc, kind!(MaxLength, *len, max));
            }
        }

        // pattern --
        if let Some(regex) = &s.pattern {
            if !regex.is_match(str) {
                let kind = kind!(Pattern, str.clone(), regex.as_str().to_string());
                self.add_error("/pattern", &vloc, kind);
            }
        }

        // contentEncoding --
        let mut decoded = Cow::from(str.as_bytes());
        if let Some(decoder) = &s.content_encoding {
            match (decoder.func)(str) {
                Ok(bytes) => decoded = Cow::from(bytes),
                Err(e) => {
                    let kind = kind!(ContentEncoding, str.clone(), decoder.name, e);
                    self.add_error("/contentEncoding", &vloc, kind)
                }
            }
        }

        // contentMediaType --
        let mut deserialized = None;
        if let Some(mt) = &s.content_media_type {
            match (mt.func)(decoded.as_ref(), s.content_schema.is_some()) {
                Ok(des) => deserialized = des,
                Err(e) => {
                    let kind = kind!(ContentMediaType, decoded.into(), mt.name, e);
                    self.add_error("/contentMediaType", &vloc, kind);
                }
            }
        }

        // contentSchema --
        if let (Some(sch), Some(v)) = (s.content_schema, deserialized) {
            if let Err(mut e) = self.schemas.validate(&v, sch) {
                e.kind = kind!(ContentSchema);
                self.errors.push(e);
            }
        }
    }

    fn num_validate(&mut self, num: &Number, vloc: JsonPointer) {
        let s = self.schema;

        // minimum --
        if let Some(min) = &s.minimum {
            if let (Some(minf), Some(numf)) = (min.as_f64(), num.as_f64()) {
                if numf < minf {
                    self.add_error("/minimum", &vloc, kind!(Minimum, num.clone(), min.clone()));
                }
            }
        }

        // maximum --
        if let Some(max) = &s.maximum {
            if let (Some(maxf), Some(numf)) = (max.as_f64(), num.as_f64()) {
                if numf > maxf {
                    self.add_error("/maximum", &vloc, kind!(Maximum, num.clone(), max.clone()));
                }
            }
        }

        // exclusiveMinimum --
        if let Some(ex_min) = &s.exclusive_minimum {
            if let (Some(ex_minf), Some(numf)) = (ex_min.as_f64(), num.as_f64()) {
                if numf <= ex_minf {
                    let kind = kind!(ExclusiveMinimum, num.clone(), ex_min.clone());
                    self.add_error("/exclusiveMinimum", &vloc, kind);
                }
            }
        }

        // exclusiveMaximum --
        if let Some(ex_max) = &s.exclusive_maximum {
            if let (Some(ex_maxf), Some(numf)) = (ex_max.as_f64(), num.as_f64()) {
                if numf >= ex_maxf {
                    let kind = kind!(ExclusiveMaximum, num.clone(), ex_max.clone());
                    self.add_error("/exclusiveMaximum", &vloc, kind);
                }
            }
        }

        // multipleOf --
        if let Some(mul) = &s.multiple_of {
            if let (Some(mulf), Some(numf)) = (mul.as_f64(), num.as_f64()) {
                if (numf / mulf).fract() != 0.0 {
                    let kind = kind!(MultipleOf, num.clone(), mul.clone());
                    self.add_error("/multipleOf", &vloc, kind);
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
            add_err!(self.validate_ref(ref_, "/$ref", vloc.copy()));
        }

        // $recursiveRef --
        if let Some(mut sch) = s.recursive_ref {
            if self.schemas.get(sch).recursive_anchor {
                sch = self.resolve_recursive_anchor().unwrap_or(sch);
            }
            add_err!(self.validate_ref(sch, "/$recursiveRef", vloc.copy()));
        }

        // $dynamicRef --
        if let Some(dref) = &s.dynamic_ref {
            let mut sch = dref.sch; // initial target
            if let Some(anchor) = &dref.anchor {
                // $dynamicRef includes anchor
                if self.schemas.get(sch).dynamic_anchor == dref.anchor {
                    // initial target has matching $dynamicAnchor
                    sch = self.resolve_dynamic_anchor(anchor).unwrap_or(sch);
                }
            }
            add_err!(self.validate_ref(sch, "/$dynamicRef", vloc.copy()));
        }
    }

    fn validate_ref(
        &mut self,
        sch: SchemaIndex,
        kw: &'static str,
        mut vloc: JsonPointer,
    ) -> Result<(), ValidationError> {
        if let Err(ref_err) = self.validate_self(sch, kw.into(), vloc.copy()) {
            let url = self.schemas.get(sch).loc.clone();
            let mut err = self.error(kw, &vloc, ErrorKind::Reference { url });
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
            if self.validate_self(not, None, vloc.copy()).is_ok() {
                self.add_error("/not", &vloc, kind!(Not));
            }
        }

        // allOf --
        if !s.all_of.is_empty() {
            let mut allof_errors = vec![];
            for (i, sch) in s.all_of.iter().enumerate() {
                if let Err(mut e) = self.validate_self(*sch, None, vloc.copy()) {
                    if let ErrorKind::Group = e.kind {
                        e.kind = ErrorKind::AllOf { subschema: Some(i) };
                    }
                    allof_errors.push(e);
                }
            }
            if !allof_errors.is_empty() {
                let kind = ErrorKind::AllOf { subschema: None };
                self.add_errors(allof_errors, "/allOf", &vloc, kind);
            }
        }

        // anyOf --
        if !s.any_of.is_empty() {
            // NOTE: all schemas must be checked
            let mut anyof_errors = vec![];
            for (i, sch) in s.any_of.iter().enumerate() {
                if let Err(mut e) = self.validate_self(*sch, None, vloc.copy()) {
                    if let ErrorKind::Group = e.kind {
                        e.kind = ErrorKind::AnyOf { subschema: Some(i) };
                    }
                    anyof_errors.push(e);
                }
            }
            if anyof_errors.len() == s.any_of.len() {
                let kind = ErrorKind::AnyOf { subschema: None };
                self.add_errors(anyof_errors, "/anyOf", &vloc, kind);
            }
        }

        // oneOf --
        if !s.one_of.is_empty() {
            let (mut matched, mut oneof_errors) = (None, vec![]);
            for (i, sch) in s.one_of.iter().enumerate() {
                if let Err(mut e) = self.validate_self(*sch, None, vloc.copy()) {
                    if let ErrorKind::Group = e.kind {
                        e.kind = ErrorKind::OneOf(OneOf::Subschema(i));
                    }
                    oneof_errors.push(e);
                } else {
                    match matched {
                        None => _ = matched.replace(i),
                        Some(j) => {
                            let kind = ErrorKind::OneOf(OneOf::MultiMatch(j, i));
                            self.add_error("/oneOf", &vloc, kind);
                        }
                    }
                }
            }
            if matched.is_none() {
                let kind = ErrorKind::OneOf(OneOf::NoneMatch);
                self.add_errors(oneof_errors, "/oneOf", &vloc, kind);
            }
        }

        // if, then, else --
        if let Some(if_) = s.if_ {
            if self.validate_self(if_, None, vloc.copy()).is_ok() {
                if let Some(then) = s.then {
                    add_err!(self.validate_self(then, None, vloc.copy()));
                }
            } else if let Some(else_) = s.else_ {
                add_err!(self.validate_self(else_, None, vloc.copy()));
            }
        }
    }
}

// uneval validation
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
                    add_err!(self.validate_val(sch, pvalue, vloc.prop(pname)));
                }
            }
            self.uneval.props.clear();
        }

        // unevaluatedItems --
        if let (Some(sch), Value::Array(arr)) = (s.unevaluated_items, v) {
            for i in &self.uneval.items {
                if let Some(pvalue) = arr.get(*i) {
                    add_err!(self.validate_val(sch, pvalue, vloc.item(*i)));
                }
            }
            self.uneval.items.clear();
        }
    }
}

// validation helpers
impl<'v, 'a, 'b, 'd> Validator<'v, 'a, 'b, 'd> {
    fn validate_val(
        &self,
        sch: SchemaIndex,
        v: &Value,
        vloc: JsonPointer,
    ) -> Result<(), ValidationError> {
        let scope = Scope::child(sch, None, self.scope.vid + 1, &self.scope);
        let schema = &self.schemas.get(sch);
        Validator {
            v,
            schema,
            schemas: self.schemas,
            scope,
            uneval: Uneval::from(v, schema, false),
            errors: vec![],
        }
        .validate(vloc)
        .map(|_| ())
    }

    fn validate_self(
        &mut self,
        sch: SchemaIndex,
        kw_path: Option<&'static str>,
        vloc: JsonPointer,
    ) -> Result<(), ValidationError> {
        let scope = Scope::child(sch, kw_path, self.scope.vid, &self.scope);
        let schema = &self.schemas.get(sch);
        let result = Validator {
            v: self.v,
            schema,
            schemas: self.schemas,
            scope,
            uneval: Uneval::from(self.v, schema, !self.uneval.is_empty()),
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
            keyword_location: self.kw_loc(&self.scope, kw_path),
            absolute_keyword_location: format!("{}{kw_path}", self.schema.loc),
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

    fn kw_loc(&self, mut scope: &Scope, kw_path: &str) -> String {
        let mut loc = kw_path.to_string();
        while let Some(parent) = scope.parent {
            let kw_path = scope.kw_path.unwrap_or_else(|| {
                let cur = &self.schemas.get(scope.sch).loc;
                let parent = &self.schemas.get(parent.sch).loc;
                &cur[parent.len()..]
            });
            loc.insert_str(0, kw_path);
            scope = parent;
        }
        loc
    }
}

// Uneval --

#[derive(Default)]
struct Uneval<'v> {
    props: HashSet<&'v String>,
    items: HashSet<usize>,
}

impl<'v> Uneval<'v> {
    fn is_empty(&self) -> bool {
        self.props.is_empty() && self.items.is_empty()
    }

    fn from(v: &'v Value, sch: &Schema, caller_needs: bool) -> Self {
        let mut uneval = Self::default();
        match v {
            Value::Object(obj) => {
                if !sch.all_props_evaluated
                    && (caller_needs || sch.unevaluated_properties.is_some())
                {
                    uneval.props = obj.keys().collect();
                }
            }
            Value::Array(arr) => {
                if !sch.all_items_evaluated && (caller_needs || sch.unevaluated_items.is_some()) {
                    uneval.items = (0..arr.len()).collect();
                }
            }
            _ => (),
        }
        uneval
    }

    fn merge(&mut self, other: &Uneval) {
        self.props.retain(|p| other.props.contains(p));
        self.items.retain(|i| other.items.contains(i));
    }
}

// Scope ---

#[derive(Debug)]
struct Scope<'a> {
    sch: SchemaIndex,
    // if None, compute from self.sch and self.parent.sh
    // not None only when there is jump i.e $ref, $XXXRef
    kw_path: Option<&'static str>,
    /// unique id of value being validated
    // if two scope validate same value, they will have same vid
    vid: usize,
    parent: Option<&'a Scope<'a>>,
}

impl<'a> Scope<'a> {
    fn child(
        sch: SchemaIndex,
        kw_path: Option<&'static str>,
        vid: usize,
        parent: &'a Scope,
    ) -> Self {
        Self {
            sch,
            kw_path,
            vid,
            parent: Some(parent),
        }
    }

    fn check_cycle(&self) -> Option<&Scope> {
        let mut scope = self.parent;
        while let Some(scp) = scope {
            if scp.vid != self.vid {
                break;
            }
            if scp.sch == self.sch {
                return Some(scp);
            }
            scope = scp.parent;
        }
        None
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
