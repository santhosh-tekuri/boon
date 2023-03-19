use std::{borrow::Cow, cmp::min, collections::HashSet, fmt::Write};

use serde_json::{Map, Value};

use crate::{util::*, *};

pub(crate) fn validate<'s, 'v>(
    v: &'v Value,
    schema: &'s Schema,
    schemas: &'s Schemas,
) -> Result<(), ValidationError<'s, 'v>> {
    let scope = Scope {
        sch: schema.idx,
        sloc_len: 0,
        vid: 0,
        parent: None,
    };
    let mut vloc = Vec::with_capacity(8);
    let mut sloc = Vec::with_capacity(10);
    let result = Validator {
        v,
        schema,
        schemas,
        scope,
        uneval: Uneval::from(v, schema, false),
        errors: vec![],
    }
    .validate(SchemaPointer::new(&mut sloc), JsonPointer::new(&mut vloc));
    match result {
        Err(mut e) => {
            if e.keyword_location.is_empty()
                && e.instance_location.is_empty()
                && matches!(e.kind, ErrorKind::Group)
            {
                e.kind = ErrorKind::Schema { url: &schema.loc };
            } else {
                e = ValidationError {
                    keyword_location: KeywordLocation::new(),
                    absolute_keyword_location: AbsoluteKeywordLocation::new(schema),
                    instance_location: InstanceLocation::new(),
                    kind: ErrorKind::Schema { url: &schema.loc },
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

struct Validator<'v, 's, 'd> {
    v: &'v Value,
    schema: &'s Schema,
    schemas: &'s Schemas,
    scope: Scope<'d>,
    uneval: Uneval<'v>,
    errors: Vec<ValidationError<'s, 'v>>,
}

impl<'v, 's, 'd> Validator<'v, 's, 'd> {
    fn validate(
        mut self,
        mut sloc: SchemaPointer<'_, 's>,
        mut vloc: JsonPointer<'_, 'v>,
    ) -> Result<Uneval<'v>, ValidationError<'s, 'v>> {
        let s = self.schema;
        let v = self.v;

        if let Some(scp) = self.scope.check_cycle() {
            let kind = ErrorKind::RefCycle {
                url: &self.schema.loc,
                kw_loc1: (&sloc).into(),
                kw_loc2: (&sloc.with_len(scp.sloc_len)).into(),
            };
            return Err(self.error(&sloc, &vloc, kind));
        }

        // boolean --
        if let Some(b) = s.boolean {
            if !b {
                return Err(self.error(&sloc, &vloc, kind!(FalseSchema)));
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
                self.add_error(&sloc.kw("type"), &vloc, kind!(Type, v_type, s.types));
            }
        }

        // enum --
        if !s.enum_.is_empty() && !s.enum_.iter().any(|e| equals(e, v)) {
            let kind = kind!(Enum, v.clone(), &s.enum_);
            self.add_error(&sloc.kw("enum"), &vloc, kind);
        }

        // constant --
        if let Some(c) = &s.constant {
            if !equals(v, c) {
                self.add_error(&sloc.kw("const"), &vloc, kind!(Const, v.clone(), c));
            }
        }

        // format --
        if let Some(format) = &s.format {
            if let Err(e) = (format.func)(v) {
                let kind = kind!(Format, v.clone(), format.name, e);
                self.add_error(&sloc.kw("format"), &vloc, kind);
            }
        }

        match v {
            Value::Object(obj) => self.obj_validate(obj, sloc.copy(), vloc.copy()),
            Value::Array(arr) => self.arr_validate(arr, sloc.copy(), vloc.copy()),
            Value::String(str) => self.str_validate(str, sloc.copy(), vloc.copy()),
            Value::Number(num) => self.num_validate(num, sloc.copy(), vloc.copy()),
            _ => {}
        }

        self.refs_validate(sloc.copy(), vloc.copy());
        self.cond_validate(sloc.copy(), vloc.copy());
        self.uneval_validate(sloc.copy(), vloc.copy());

        match self.errors.len() {
            0 => Ok(self.uneval),
            1 => Err(self.errors.remove(0)),
            _ => {
                let mut e = self.error(&sloc, &vloc, kind!(Group));
                e.causes = self.errors;
                Err(e)
            }
        }
    }
}

// type specific validations
impl<'v, 's, 'd> Validator<'v, 's, 'd> {
    fn obj_validate(
        &mut self,
        obj: &'v Map<String, Value>,
        mut sloc: SchemaPointer<'_, 's>,
        mut vloc: JsonPointer<'_, 'v>,
    ) {
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
                self.add_error(&sloc.kw("minProperties"), &vloc, kind);
            }
        }

        // maxProperties --
        if let Some(max) = s.max_properties {
            if obj.len() > max {
                let kind = kind!(MaxProperties, obj.len(), max);
                self.add_error(&sloc.kw("maxProperties"), &vloc, kind);
            }
        }

        // propertyNames --
        if let Some(sch) = &s.property_names {
            for pname in obj.keys() {
                //todo: use pname as value(tip: use enum{PropName|Value})
                let v = Value::String(pname.to_owned());
                let mut vec = Vec::with_capacity(vloc.len);
                let vloc = vloc.clone_static(&mut vec);
                if let Err(e) = self.validate_val(*sch, sloc.kw("propertyNames"), &v, vloc) {
                    self.errors.push(e.clone_static());
                }
            }
        }

        let find_missing = |required: &'s Vec<String>| -> Vec<&'s str> {
            required
                .iter()
                .filter(|p| !obj.contains_key(p.as_str()))
                .map(|p| p.as_str())
                .collect()
        };

        // required --
        let missing = find_missing(&s.required);
        if !missing.is_empty() {
            self.add_error(&sloc.kw("required"), &vloc, kind!(Required, want: missing));
        }

        // dependencies --
        for (pname, dependency) in &s.dependencies {
            if obj.contains_key(pname) {
                match dependency {
                    Dependency::Props(required) => {
                        let missing = find_missing(required);
                        if !missing.is_empty() {
                            let kind = kind!(Dependency, pname, missing);
                            self.add_error(&sloc.kw("dependencies").prop(pname), &vloc, kind);
                        }
                    }
                    Dependency::SchemaRef(sch) => {
                        if let Err(e) = self.validate_self(
                            *sch,
                            sloc.kw("dependencies").prop(pname),
                            vloc.copy(),
                        ) {
                            if let ErrorKind::Group = e.kind {
                                let kind = kind!(Dependency, pname, vec![]);
                                self.add_errors(
                                    e.causes,
                                    &sloc.kw("dependencies").prop(pname),
                                    &vloc,
                                    kind,
                                );
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
                if let Err(e) =
                    self.validate_self(*sch, sloc.kw("dependentSchemas").prop(pname), vloc.copy())
                {
                    if let ErrorKind::Group = e.kind {
                        let kind = kind!(DependentSchemas, got: pname);
                        self.add_errors(
                            e.causes,
                            &sloc.kw("dependentSchemas").prop(pname),
                            &vloc,
                            kind,
                        );
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
                    let kind = kind!(DependentRequired, pname, missing);
                    self.add_error(&sloc.kw("dependentRequired").prop(pname), &vloc, kind);
                }
            }
        }

        for (pname, pvalue) in obj {
            let mut evaluated = false;

            // properties --
            if let Some((key, sch)) = s.properties.get_key_value(pname) {
                match self.validate_val(
                    *sch,
                    sloc.kw("properties").prop(key),
                    pvalue,
                    vloc.prop(pname),
                ) {
                    Ok(_) => evaluated = true,
                    Err(e) => self.errors.push(e),
                }
            }

            // patternProperties --
            for (regex, sch) in &s.pattern_properties {
                if regex.is_match(pname) {
                    match self.validate_val(
                        *sch,
                        sloc.kw("patternProperties").prop(regex.as_str()),
                        pvalue,
                        vloc.prop(pname),
                    ) {
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
                                self.add_error(&sloc.kw("additionalProperties"), &vloc, kind);
                            }
                        }
                        Additional::SchemaRef(sch) => {
                            add_err!(self.validate_val(
                                *sch,
                                sloc.kw("additionalProperties"),
                                pvalue,
                                vloc.prop(pname)
                            ));
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

    fn arr_validate(
        &mut self,
        arr: &'v Vec<Value>,
        mut sloc: SchemaPointer<'_, 's>,
        mut vloc: JsonPointer<'_, 'v>,
    ) {
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
                self.add_error(&sloc.kw("minItems"), &vloc, kind!(MinItems, arr.len(), min));
            }
        }

        // maxItems --
        if let Some(max) = s.max_items {
            if arr.len() > max {
                self.add_error(&sloc.kw("maxItems"), &vloc, kind!(MaxItems, arr.len(), max));
            }
        }

        // uniqueItems --
        if s.unique_items {
            'outer: for i in 1..arr.len() {
                for j in 0..i {
                    if equals(&arr[i], &arr[j]) {
                        let kind = kind!(UniqueItems, got: [j, i]);
                        self.add_error(&sloc.kw("uniqueItems"), &vloc, kind);
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
                            if let Err(mut e) =
                                self.validate_val(*sch, sloc.kw("items"), item, vloc.item(i))
                            {
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
                            add_err!(self.validate_val(
                                *sch,
                                sloc.kw("items").item(i),
                                item,
                                vloc.item(i)
                            ));
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
                            self.add_error(&sloc.kw("additionalItems"), &vloc, kind);
                        }
                    }
                    Additional::SchemaRef(sch) => {
                        for (i, item) in arr[evaluated..].iter().enumerate() {
                            add_err!(self.validate_val(
                                *sch,
                                sloc.kw("additionalItems"),
                                item,
                                vloc.item(i)
                            ));
                        }
                    }
                }
                debug_assert!(self.uneval.items.is_empty());
            }
        } else {
            // prefixItems --
            for (i, (sch, item)) in s.prefix_items.iter().zip(arr).enumerate() {
                self.uneval.items.remove(&i);
                add_err!(self.validate_val(
                    *sch,
                    sloc.kw("prefixItems").item(i),
                    item,
                    vloc.item(i)
                ));
            }

            // items2020 --
            if let Some(sch) = &s.items2020 {
                let evaluated = min(s.prefix_items.len(), arr.len());
                for (i, item) in arr[evaluated..].iter().enumerate() {
                    if let Err(mut e) =
                        self.validate_val(*sch, sloc.kw("items"), item, vloc.item(i))
                    {
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
                if let Err(e) = self.validate_val(*sch, sloc.kw("contains"), item, vloc.item(i)) {
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
                let mut e = self.error(&sloc.kw("minContains"), &vloc, kind);
                e.causes = contains_errors;
                self.errors.push(e);
            }
        } else if s.contains.is_some() && contains_matched.is_empty() {
            let mut e = self.error(&sloc.kw("contains"), &vloc, kind!(Contains));
            e.causes = contains_errors;
            self.errors.push(e);
        }

        // maxContains --
        if let Some(max) = s.max_contains {
            if contains_matched.len() > max {
                let kind = kind!(MaxContains, contains_matched, max);
                self.add_error(&sloc.kw("maxContains"), &vloc, kind);
            }
        }
    }

    fn str_validate(
        &mut self,
        str: &'v String,
        mut sloc: SchemaPointer<'_, 's>,
        vloc: JsonPointer<'_, 'v>,
    ) {
        let s = self.schema;
        let mut len = None;

        // minLength --
        if let Some(min) = s.min_length {
            let len = len.get_or_insert_with(|| str.chars().count());
            if *len < min {
                self.add_error(&sloc.kw("minLength"), &vloc, kind!(MinLength, *len, min));
            }
        }

        // maxLength --
        if let Some(max) = s.max_length {
            let len = len.get_or_insert_with(|| str.chars().count());
            if *len > max {
                self.add_error(&sloc.kw("maxLength"), &vloc, kind!(MaxLength, *len, max));
            }
        }

        // pattern --
        if let Some(regex) = &s.pattern {
            if !regex.is_match(str) {
                let kind = kind!(Pattern, str.clone(), regex.as_str());
                self.add_error(&sloc.kw("pattern"), &vloc, kind);
            }
        }

        // contentEncoding --
        let mut decoded = Cow::from(str.as_bytes());
        if let Some(decoder) = &s.content_encoding {
            match (decoder.func)(str) {
                Ok(bytes) => decoded = Cow::from(bytes),
                Err(e) => {
                    let kind = kind!(ContentEncoding, str.clone(), decoder.name, e);
                    self.add_error(&sloc.kw("contentEncoding"), &vloc, kind)
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
                    self.add_error(&sloc.kw("contentMediaType"), &vloc, kind);
                }
            }
        }

        // contentSchema --
        if let (Some(sch), Some(v)) = (s.content_schema, deserialized) {
            // todo: check if keywordLocation is correct
            if let Err(mut e) = self.schemas.validate(&v, sch) {
                e.kind = kind!(ContentSchema);
                self.errors.push(e.clone_static());
            }
        }
    }

    fn num_validate(
        &mut self,
        num: &'v Number,
        mut sloc: SchemaPointer<'_, 's>,
        vloc: JsonPointer<'_, 'v>,
    ) {
        let s = self.schema;

        // minimum --
        if let Some(min) = &s.minimum {
            if let (Some(minf), Some(numf)) = (min.as_f64(), num.as_f64()) {
                if numf < minf {
                    let kind = kind!(Minimum, num.clone(), min.clone());
                    self.add_error(&sloc.kw("minimum"), &vloc, kind);
                }
            }
        }

        // maximum --
        if let Some(max) = &s.maximum {
            if let (Some(maxf), Some(numf)) = (max.as_f64(), num.as_f64()) {
                if numf > maxf {
                    let kind = kind!(Maximum, num.clone(), max.clone());
                    self.add_error(&sloc.kw("maximum"), &vloc, kind);
                }
            }
        }

        // exclusiveMinimum --
        if let Some(ex_min) = &s.exclusive_minimum {
            if let (Some(ex_minf), Some(numf)) = (ex_min.as_f64(), num.as_f64()) {
                if numf <= ex_minf {
                    let kind = kind!(ExclusiveMinimum, num.clone(), ex_min.clone());
                    self.add_error(&sloc.kw("exclusiveMinimum"), &vloc, kind);
                }
            }
        }

        // exclusiveMaximum --
        if let Some(ex_max) = &s.exclusive_maximum {
            if let (Some(ex_maxf), Some(numf)) = (ex_max.as_f64(), num.as_f64()) {
                if numf >= ex_maxf {
                    let kind = kind!(ExclusiveMaximum, num.clone(), ex_max.clone());
                    self.add_error(&sloc.kw("exclusiveMaximum"), &vloc, kind);
                }
            }
        }

        // multipleOf --
        if let Some(mul) = &s.multiple_of {
            if let (Some(mulf), Some(numf)) = (mul.as_f64(), num.as_f64()) {
                if (numf / mulf).fract() != 0.0 {
                    let kind = kind!(MultipleOf, num.clone(), mul.clone());
                    self.add_error(&sloc.kw("multipleOf"), &vloc, kind);
                }
            }
        }
    }
}

// references validation
impl<'v, 's, 'd> Validator<'v, 's, 'd> {
    fn refs_validate(&mut self, mut sloc: SchemaPointer<'_, 's>, mut vloc: JsonPointer<'_, 'v>) {
        let s = self.schema;
        macro_rules! add_err {
            ($result:expr) => {
                let result = $result;
                self.errors.extend(result.err().into_iter());
            };
        }

        // $ref --
        if let Some(ref_) = s.ref_ {
            add_err!(self.validate_ref(ref_, sloc.kw("$ref"), vloc.copy()));
        }

        // $recursiveRef --
        if let Some(mut sch) = s.recursive_ref {
            if self.schemas.get(sch).recursive_anchor {
                sch = self.resolve_recursive_anchor().unwrap_or(sch);
            }
            add_err!(self.validate_ref(sch, sloc.kw("$recursiveRef"), vloc.copy()));
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
            add_err!(self.validate_ref(sch, sloc.kw("$dynamicRef"), vloc.copy()));
        }
    }

    fn validate_ref(
        &mut self,
        sch: SchemaIndex,
        mut sloc: SchemaPointer<'_, 's>,
        mut vloc: JsonPointer<'_, 'v>,
    ) -> Result<(), ValidationError<'s, 'v>> {
        if let Err(ref_err) = self.validate_self(sch, sloc.copy(), vloc.copy()) {
            let url = &self.schemas.get(sch).loc;
            let mut err = self.error(&sloc, &vloc, ErrorKind::Reference { url });
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
impl<'v, 's, 'd> Validator<'v, 's, 'd> {
    fn cond_validate(&mut self, mut sloc: SchemaPointer<'_, 's>, mut vloc: JsonPointer<'_, 'v>) {
        let s = self.schema;
        macro_rules! add_err {
            ($result:expr) => {
                let result = $result;
                self.errors.extend(result.err().into_iter());
            };
        }

        // not --
        if let Some(not) = s.not {
            if self.validate_self(not, sloc.kw("not"), vloc.copy()).is_ok() {
                self.add_error(&sloc.kw("not"), &vloc, kind!(Not));
            }
        }

        // allOf --
        if !s.all_of.is_empty() {
            let mut allof_errors = vec![];
            for (i, sch) in s.all_of.iter().enumerate() {
                if let Err(mut e) = self.validate_self(*sch, sloc.kw("allOf").item(i), vloc.copy())
                {
                    if let ErrorKind::Group = e.kind {
                        e.kind = ErrorKind::AllOf { subschema: Some(i) };
                    }
                    allof_errors.push(e);
                }
            }
            if !allof_errors.is_empty() {
                let kind = ErrorKind::AllOf { subschema: None };
                self.add_errors(allof_errors, &sloc.kw("allOf"), &vloc, kind);
            }
        }

        // anyOf --
        if !s.any_of.is_empty() {
            // NOTE: all schemas must be checked for uneval
            let mut anyof_errors = vec![];
            for (i, sch) in s.any_of.iter().enumerate() {
                match self.validate_self(*sch, sloc.kw("anyOf").item(i), vloc.copy()) {
                    Ok(_) => {
                        if self.uneval.is_empty() {
                            break;
                        }
                    }
                    Err(mut e) => {
                        if let ErrorKind::Group = e.kind {
                            e.kind = ErrorKind::AnyOf { subschema: Some(i) };
                        }
                        anyof_errors.push(e);
                    }
                }
            }
            if anyof_errors.len() == s.any_of.len() {
                let kind = ErrorKind::AnyOf { subschema: None };
                self.add_errors(anyof_errors, &sloc.kw("anyOf"), &vloc, kind);
            }
        }

        // oneOf --
        if !s.one_of.is_empty() {
            let (mut matched, mut oneof_errors) = (None, vec![]);
            for (i, sch) in s.one_of.iter().enumerate() {
                if let Err(mut e) = self.validate_self(*sch, sloc.kw("oneOf").item(i), vloc.copy())
                {
                    if let ErrorKind::Group = e.kind {
                        e.kind = ErrorKind::OneOf(OneOf::Subschema(i));
                    }
                    oneof_errors.push(e);
                } else {
                    match matched {
                        None => _ = matched.replace(i),
                        Some(j) => {
                            let kind = ErrorKind::OneOf(OneOf::MultiMatch(j, i));
                            self.add_error(&sloc.kw("oneOf"), &vloc, kind);
                        }
                    }
                }
            }
            if matched.is_none() {
                let kind = ErrorKind::OneOf(OneOf::NoneMatch);
                self.add_errors(oneof_errors, &sloc.kw("oneOf"), &vloc, kind);
            }
        }

        // if, then, else --
        if let Some(if_) = s.if_ {
            if self.validate_self(if_, sloc.kw("if"), vloc.copy()).is_ok() {
                if let Some(then) = s.then {
                    add_err!(self.validate_self(then, sloc.kw("then"), vloc.copy()));
                }
            } else if let Some(else_) = s.else_ {
                add_err!(self.validate_self(else_, sloc.kw("else"), vloc.copy()));
            }
        }
    }
}

// uneval validation
impl<'v, 's, 'd> Validator<'v, 's, 'd> {
    fn uneval_validate(&mut self, mut sloc: SchemaPointer<'_, 's>, mut vloc: JsonPointer<'_, 'v>) {
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
                    add_err!(self.validate_val(
                        sch,
                        sloc.kw("unevaluatedProperties"),
                        pvalue,
                        vloc.prop(pname)
                    ));
                }
            }
            self.uneval.props.clear();
        }

        // unevaluatedItems --
        if let (Some(sch), Value::Array(arr)) = (s.unevaluated_items, v) {
            for i in &self.uneval.items {
                if let Some(pvalue) = arr.get(*i) {
                    add_err!(self.validate_val(
                        sch,
                        sloc.kw("unevaluatedItems"),
                        pvalue,
                        vloc.item(*i)
                    ));
                }
            }
            self.uneval.items.clear();
        }
    }
}

// validation helpers
impl<'v, 's, 'd> Validator<'v, 's, 'd> {
    fn validate_val(
        &self,
        sch: SchemaIndex,
        sloc: SchemaPointer<'_, 's>,
        v: &'v Value,
        vloc: JsonPointer<'_, 'v>,
    ) -> Result<(), ValidationError<'s, 'v>> {
        let scope = Scope::child(sch, sloc.len, self.scope.vid + 1, &self.scope);
        let schema = &self.schemas.get(sch);
        Validator {
            v,
            schema,
            schemas: self.schemas,
            scope,
            uneval: Uneval::from(v, schema, false),
            errors: vec![],
        }
        .validate(sloc, vloc)
        .map(|_| ())
    }

    fn validate_self(
        &mut self,
        sch: SchemaIndex,
        sloc: SchemaPointer<'_, 's>,
        vloc: JsonPointer<'_, 'v>,
    ) -> Result<(), ValidationError<'s, 'v>> {
        let scope = Scope::child(sch, sloc.len, self.scope.vid, &self.scope);
        let schema = &self.schemas.get(sch);
        let result = Validator {
            v: self.v,
            schema,
            schemas: self.schemas,
            scope,
            uneval: Uneval::from(self.v, schema, !self.uneval.is_empty()),
            errors: vec![],
        }
        .validate(sloc, vloc);
        if let Ok(reply) = &result {
            self.uneval.merge(reply);
        }
        result.map(|_| ())
    }
}

// error helpers
impl<'v, 's, 'd> Validator<'v, 's, 'd> {
    fn error(
        &self,
        sloc: &SchemaPointer<'_, 's>,
        vloc: &JsonPointer<'_, 'v>,
        kind: ErrorKind<'s>,
    ) -> ValidationError<'s, 'v> {
        ValidationError {
            keyword_location: sloc.into(),
            absolute_keyword_location: AbsoluteKeywordLocation {
                url: &self.schema.loc,
                keyword_location: sloc.kw_path(self.scope.sloc_len),
            },
            instance_location: vloc.into(),
            kind,
            causes: vec![],
        }
    }

    fn add_error(
        &mut self,
        sloc: &SchemaPointer<'_, 's>,
        vloc: &JsonPointer<'_, 'v>,
        kind: ErrorKind<'s>,
    ) {
        self.errors.push(self.error(sloc, vloc, kind));
    }

    fn add_errors(
        &mut self,
        errors: Vec<ValidationError<'s, 'v>>,
        sloc: &SchemaPointer<'_, 's>,
        vloc: &JsonPointer<'_, 'v>,
        kind: ErrorKind<'s>,
    ) {
        if errors.len() == 1 {
            self.errors.extend(errors);
        } else {
            let mut err = self.error(sloc, vloc, kind);
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
    sloc_len: usize,
    /// unique id of value being validated
    // if two scope validate same value, they will have same vid
    vid: usize,
    parent: Option<&'a Scope<'a>>,
}

impl<'a> Scope<'a> {
    fn child(sch: SchemaIndex, sloc_len: usize, vid: usize, parent: &'a Scope) -> Self {
        Self {
            sch,
            sloc_len,
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

#[derive(Debug, Clone)]
enum Token<'v> {
    Prop(Cow<'v, str>),
    Item(usize),
}

impl<'v> Token<'v> {
    fn to_string(tokens: &[Token]) -> String {
        let mut r = String::new();
        for tok in tokens {
            r.push('/');
            match tok {
                Token::Prop(s) => r.push_str(&escape(s)),
                Token::Item(i) => write!(&mut r, "{i}").expect("write to String should never fail"),
            }
        }
        r
    }
}

impl<'v> From<String> for Token<'v> {
    fn from(prop: String) -> Self {
        Token::Prop(prop.into())
    }
}

impl<'v> From<&'v str> for Token<'v> {
    fn from(prop: &'v str) -> Self {
        Token::Prop(prop.into())
    }
}

impl<'v> From<usize> for Token<'v> {
    fn from(index: usize) -> Self {
        Token::Item(index)
    }
}

struct JsonPointer<'a, 'v> {
    vec: &'a mut Vec<Token<'v>>,
    len: usize,
}

impl<'a, 'v> JsonPointer<'a, 'v> {
    fn new(vec: &'a mut Vec<Token<'v>>) -> Self {
        let len = vec.len();
        Self { vec, len }
    }

    fn copy<'x>(&'x mut self) -> JsonPointer<'x, 'v> {
        JsonPointer {
            vec: &mut *self.vec,
            len: self.len,
        }
    }

    fn prop<'x>(&'x mut self, name: &'v str) -> JsonPointer<'x, 'v> {
        self.vec.truncate(self.len);
        self.vec.push(name.into());
        JsonPointer::new(self.vec)
    }

    fn item<'x>(&'x mut self, i: usize) -> JsonPointer<'x, 'v> {
        self.vec.truncate(self.len);
        self.vec.push(i.into());
        JsonPointer::new(self.vec)
    }

    fn clone_static<'aa, 'vv>(&self, vec: &'aa mut Vec<Token<'vv>>) -> JsonPointer<'aa, 'vv> {
        for tok in self.vec[..self.len].iter() {
            match tok {
                Token::Prop(p) => vec.push(p.as_ref().to_owned().into()),
                Token::Item(i) => vec.push((*i).into()),
            }
        }
        JsonPointer::new(vec)
    }
}

impl<'a, 'v> ToString for JsonPointer<'a, 'v> {
    fn to_string(&self) -> String {
        Token::to_string(&self.vec[..self.len])
    }
}

#[derive(Debug, Default)]
pub struct InstanceLocation<'v>(Vec<Token<'v>>);

impl<'v> InstanceLocation<'v> {
    fn new() -> Self {
        Self::default()
    }

    fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    fn clone_static(self) -> InstanceLocation<'static> {
        let mut vec = Vec::with_capacity(self.0.len());
        for tok in self.0 {
            let tok = match tok {
                Token::Prop(p) => Token::Prop(p.into_owned().into()),
                Token::Item(i) => Token::Item(i),
            };
            vec.push(tok);
        }
        InstanceLocation(vec)
    }
}

impl<'a, 'v> From<&JsonPointer<'a, 'v>> for InstanceLocation<'v> {
    fn from(value: &JsonPointer<'a, 'v>) -> Self {
        let mut vec = Vec::with_capacity(value.len);
        for tok in &value.vec[..value.len] {
            vec.push(tok.clone());
        }
        Self(vec)
    }
}

impl<'v> ToString for InstanceLocation<'v> {
    fn to_string(&self) -> String {
        Token::to_string(&self.0)
    }
}

impl<'s, 'v> ValidationError<'s, 'v> {
    pub(crate) fn clone_static(self) -> ValidationError<'s, 'static> {
        let mut causes = Vec::with_capacity(self.causes.len());
        for cause in self.causes {
            causes.push(cause.clone_static());
        }
        ValidationError {
            instance_location: self.instance_location.clone_static(),
            causes,
            ..self
        }
    }
}

// SchemaPointer --

#[derive(Debug, Clone)]
pub(crate) enum SchemaToken<'s> {
    Keyword(&'static str),
    Prop(&'s str),
    Item(usize),
}

impl<'s> SchemaToken<'s> {
    fn to_string(tokens: &[SchemaToken]) -> String {
        use SchemaToken::*;
        let mut r = String::new();
        for tok in tokens {
            r.push('/');
            match tok {
                Keyword(s) | Prop(s) => r.push_str(&escape(s)),
                Item(i) => write!(&mut r, "{i}").expect("write to String should never fail"),
            }
        }
        r
    }
}

impl<'s> Display for SchemaToken<'s> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        use SchemaToken::*;
        match self {
            Keyword(s) | Prop(s) => write!(f, "{}", &escape(s)),
            Item(i) => write!(f, "{i}"),
        }
    }
}

struct SchemaPointer<'a, 's> {
    vec: &'a mut Vec<SchemaToken<'s>>,
    len: usize,
}

impl<'a, 's> SchemaPointer<'a, 's> {
    fn new(vec: &'a mut Vec<SchemaToken<'s>>) -> Self {
        let len = vec.len();
        Self { vec, len }
    }

    fn copy<'x>(&'x mut self) -> SchemaPointer<'x, 's> {
        SchemaPointer {
            vec: self.vec,
            len: self.len,
        }
    }

    fn kw<'x>(&'x mut self, name: &'static str) -> SchemaPointer<'x, 's> {
        self.vec.truncate(self.len);
        self.vec.push(SchemaToken::Keyword(name));
        SchemaPointer::new(self.vec)
    }

    fn prop<'x>(&'x mut self, name: &'s str) -> SchemaPointer<'x, 's> {
        self.vec.truncate(self.len);
        self.vec.push(SchemaToken::Prop(name));
        SchemaPointer::new(self.vec)
    }

    fn item<'x>(&'x mut self, i: usize) -> SchemaPointer<'x, 's> {
        self.vec.truncate(self.len);
        self.vec.push(SchemaToken::Item(i));
        SchemaPointer::new(self.vec)
    }

    fn with_len<'x>(&'x mut self, len: usize) -> SchemaPointer<'x, 's> {
        SchemaPointer { vec: self.vec, len }
    }

    fn kw_path<'x>(&'x self, len: usize) -> KeywordLocation<'s> {
        let mut vec = Vec::with_capacity(self.len - len);
        for tok in &self.vec[len..self.len] {
            vec.push(tok.clone());
        }
        KeywordLocation(vec)
    }
}

impl<'a, 's> ToString for SchemaPointer<'a, 's> {
    fn to_string(&self) -> String {
        SchemaToken::to_string(&self.vec[..self.len])
    }
}

#[derive(Debug, Default, Clone)]
pub struct KeywordLocation<'s>(pub(crate) Vec<SchemaToken<'s>>);

impl<'v> KeywordLocation<'v> {
    fn new() -> Self {
        Self::default()
    }

    fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

impl<'a, 's> From<&SchemaPointer<'a, 's>> for KeywordLocation<'s> {
    fn from(value: &SchemaPointer<'a, 's>) -> Self {
        let mut vec = Vec::with_capacity(value.len);
        for tok in &value.vec[..value.len] {
            vec.push(tok.clone());
        }
        Self(vec)
    }
}

impl<'s> ToString for KeywordLocation<'s> {
    fn to_string(&self) -> String {
        SchemaToken::to_string(&self.0)
    }
}

#[derive(Debug, Clone)]
pub struct AbsoluteKeywordLocation<'s> {
    pub url: &'s str,
    pub keyword_location: KeywordLocation<'s>,
}

impl<'s> AbsoluteKeywordLocation<'s> {
    fn new(sch: &'s Schema) -> Self {
        Self {
            url: &sch.loc,
            keyword_location: Default::default(),
        }
    }
}

impl<'s> Display for AbsoluteKeywordLocation<'s> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.url.fmt(f)?;
        write!(f, "{}", self.keyword_location.to_string()) // todo: url-encode
    }
}
