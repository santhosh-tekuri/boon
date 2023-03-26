use std::{borrow::Cow, cmp::min, collections::HashSet, fmt::Write};

use serde_json::{Map, Value};

use crate::{util::*, *};

macro_rules! prop {
    ($prop:expr) => {
        InstanceToken::Prop(Cow::Borrowed($prop))
    };
}

macro_rules! item {
    ($item:expr) => {
        InstanceToken::Item($item)
    };
}

pub(crate) fn validate<'s, 'v>(
    v: &'v Value,
    schema: &'s Schema,
    schemas: &'s Schemas,
) -> Result<(), ValidationError<'s, 'v>> {
    let scope = Scope {
        sch: schema.idx,
        ref_kw: None,
        vid: 0,
        parent: None,
    };
    let mut vloc = Vec::with_capacity(8);
    let result = Validator {
        v,
        vloc: &mut JsonPointer::new(&mut vloc),
        schema,
        schemas,
        scope,
        uneval: Uneval::from(v, schema, false),
        errors: vec![],
        bool_result: false,
    }
    .validate();
    match result {
        Err(err) => {
            let mut e = ValidationError {
                schema_url: &schema.loc,
                instance_location: InstanceLocation::new(),
                kind: ErrorKind::Schema { url: &schema.loc },
                causes: vec![],
            };
            if let ErrorKind::Group = err.kind {
                e.causes = err.causes;
            } else {
                e.causes.push(err);
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

struct Validator<'v, 's, 'd, 'e, 'f> {
    v: &'v Value,
    vloc: &'f mut JsonPointer<'e, 'v>,
    schema: &'s Schema,
    schemas: &'s Schemas,
    scope: Scope<'d>,
    uneval: Uneval<'v>,
    errors: Vec<ValidationError<'s, 'v>>,
    bool_result: bool,
}

impl<'v, 's, 'd, 'e, 'f> Validator<'v, 's, 'd, 'e, 'f> {
    fn validate(mut self) -> Result<Uneval<'v>, ValidationError<'s, 'v>> {
        let s = self.schema;
        let v = self.v;

        // boolean --
        if let Some(b) = s.boolean {
            return match b {
                false => Err(self.error(kind!(FalseSchema))),
                true => Ok(self.uneval),
            };
        }

        if let Some(scp) = self.scope.check_cycle() {
            let kind = ErrorKind::RefCycle {
                url: &self.schema.loc,
                kw_loc1: self.kw_loc(&self.scope),
                kw_loc2: self.kw_loc(scp),
            };
            return Err(self.error(kind));
        }

        // type --
        if !s.types.is_empty() {
            let v_type = Type::of(v);
            let matched =
                s.types.contains(v_type) || (s.types.contains(Type::Integer) && is_integer(v));
            if !matched {
                return Err(self.error(kind!(Type, v_type, s.types)));
            }
        }

        // enum --
        if let Some(Enum { types, values }) = &s.enum_ {
            if !types.contains(Type::of(v)) || !values.iter().any(|e| equals(e, v)) {
                return Err(self.error(kind!(Enum, want: values)));
            }
        }

        // constant --
        if let Some(c) = &s.constant {
            if !equals(v, c) {
                return Err(self.error(kind!(Const, want: c)));
            }
        }

        // $ref --
        if let Some(ref_) = s.ref_ {
            let result = self.validate_ref(ref_, "$ref");
            if s.draft_version < 2019 {
                return result.map(|_| self.uneval);
            }
            self.errors.extend(result.err());
        }

        // format --
        if let Some(format) = &s.format {
            if let Err(e) = (format.func)(v) {
                self.add_error(kind!(Format, v.clone(), format.name, e));
            }
        }

        match v {
            Value::Object(obj) => self.obj_validate(obj),
            Value::Array(arr) => self.arr_validate(arr),
            Value::String(str) => self.str_validate(str),
            Value::Number(num) => self.num_validate(num),
            _ => {}
        }

        if !self.bool_result || self.errors.is_empty() {
            if s.draft_version >= 2019 {
                self.refs_validate();
            }
            self.cond_validate();
            if s.draft_version >= 2019 {
                self.uneval_validate();
            }
        }

        match self.errors.len() {
            0 => Ok(self.uneval),
            1 => Err(self.errors.remove(0)),
            _ => {
                let mut e = self.error(kind!(Group));
                e.causes = self.errors;
                Err(e)
            }
        }
    }
}

// type specific validations
impl<'v, 's, 'd, 'e, 'f> Validator<'v, 's, 'd, 'e, 'f> {
    fn obj_validate(&mut self, obj: &'v Map<String, Value>) {
        let s = self.schema;
        macro_rules! add_err {
            ($result:expr) => {
                if let Err(e) = $result {
                    self.errors.push(e);
                }
            };
        }

        // minProperties --
        if let Some(min) = s.min_properties {
            if obj.len() < min {
                self.add_error(kind!(MinProperties, obj.len(), min));
            }
        }

        // maxProperties --
        if let Some(max) = s.max_properties {
            if obj.len() > max {
                self.add_error(kind!(MaxProperties, obj.len(), max));
            }
        }

        // required --
        if !s.required.is_empty() {
            if let Some(missing) = self.find_missing(obj, &s.required) {
                self.add_error(kind!(Required, want: missing));
            }
        }

        if self.bool_result && !self.errors.is_empty() {
            return;
        }

        // dependencies --
        for (prop, dependency) in &s.dependencies {
            if obj.contains_key(prop) {
                match dependency {
                    Dependency::Props(required) => {
                        if let Some(missing) = self.find_missing(obj, required) {
                            self.add_error(ErrorKind::Dependency { prop, missing });
                        }
                    }
                    Dependency::SchemaRef(sch) => {
                        add_err!(self.validate_self(*sch));
                    }
                }
            }
        }

        for (pname, pvalue) in obj {
            if self.bool_result && !self.errors.is_empty() {
                return;
            }
            let mut evaluated = false;

            // properties --
            if let Some(sch) = s.properties.get(pname) {
                match self.validate_val(*sch, pvalue, prop!(pname)) {
                    Ok(_) => evaluated = true,
                    Err(e) => self.errors.push(e),
                }
            }

            // patternProperties --
            for (regex, sch) in &s.pattern_properties {
                if regex.is_match(pname) {
                    match self.validate_val(*sch, pvalue, prop!(pname)) {
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
                                self.add_error(kind!(AdditionalProperty, got: pname.clone()));
                            }
                        }
                        Additional::SchemaRef(sch) => {
                            add_err!(self.validate_val(*sch, pvalue, prop!(pname)));
                        }
                    }
                    evaluated = true;
                }
            }

            if evaluated {
                self.uneval.props.remove(pname);
            }
        }

        if s.draft_version == 4 {
            return;
        }

        // propertyNames --
        if let Some(sch) = &s.property_names {
            for pname in obj.keys() {
                let v = Value::String(pname.to_owned());
                let mut vec = Vec::with_capacity(self.vloc.len);
                let mut vloc = self.vloc.clone_static(&mut vec);

                let scope = self.scope.child(*sch, None, self.scope.vid + 1);
                let schema = &self.schemas.get(*sch);
                let result = Validator {
                    v: &v,
                    vloc: &mut vloc,
                    schema,
                    schemas: self.schemas,
                    scope,
                    uneval: Uneval::default(),
                    errors: vec![],
                    bool_result: self.bool_result,
                }
                .validate();

                if let Err(e) = result {
                    self.errors.push(e.clone_static());
                }
            }
        }

        if s.draft_version == 6 {
            return;
        }

        // dependentSchemas --
        for (pname, sch) in &s.dependent_schemas {
            if obj.contains_key(pname) {
                add_err!(self.validate_self(*sch));
            }
        }

        // dependentRequired --
        for (prop, required) in &s.dependent_required {
            if obj.contains_key(prop) {
                if let Some(missing) = self.find_missing(obj, required) {
                    self.add_error(ErrorKind::DependentRequired { prop, missing });
                }
            }
        }
    }

    fn arr_validate(&mut self, arr: &'v Vec<Value>) {
        let s = self.schema;
        macro_rules! add_err {
            ($result:expr) => {
                if let Err(e) = $result {
                    self.errors.push(e);
                }
            };
        }

        // minItems --
        if let Some(min) = s.min_items {
            if arr.len() < min {
                self.add_error(kind!(MinItems, arr.len(), min));
            }
        }

        // maxItems --
        if let Some(max) = s.max_items {
            if arr.len() > max {
                self.add_error(kind!(MaxItems, arr.len(), max));
            }
        }

        // uniqueItems --
        if s.unique_items {
            'outer: for i in 1..arr.len() {
                for j in 0..i {
                    if equals(&arr[i], &arr[j]) {
                        self.add_error(kind!(UniqueItems, got: [j, i]));
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
                            add_err!(self.validate_val(*sch, item, item!(i)));
                        }
                        evaluated = arr.len();
                        debug_assert!(self.uneval.items.is_empty());
                    }
                    Items::SchemaRefs(list) => {
                        for (i, (item, sch)) in arr.iter().zip(list).enumerate() {
                            self.uneval.items.remove(&i);
                            add_err!(self.validate_val(*sch, item, item!(i)));
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
                            self.add_error(kind!(AdditionalItems, got: arr.len() - evaluated));
                        }
                    }
                    Additional::SchemaRef(sch) => {
                        for (i, item) in arr[evaluated..].iter().enumerate() {
                            add_err!(self.validate_val(*sch, item, item!(i)));
                        }
                    }
                }
                debug_assert!(self.uneval.items.is_empty());
            }
        } else {
            // prefixItems --
            for (i, (sch, item)) in s.prefix_items.iter().zip(arr).enumerate() {
                self.uneval.items.remove(&i);
                add_err!(self.validate_val(*sch, item, item!(i)));
            }

            // items2020 --
            if let Some(sch) = &s.items2020 {
                let evaluated = min(s.prefix_items.len(), arr.len());
                for (i, item) in arr[evaluated..].iter().enumerate() {
                    add_err!(self.validate_val(*sch, item, item!(i)));
                }
                debug_assert!(self.uneval.items.is_empty());
            }
        }

        // contains --
        if let Some(sch) = &s.contains {
            let mut contains_matched = vec![];
            let mut contains_errors = vec![];

            for (i, item) in arr.iter().enumerate() {
                if let Err(e) = self.validate_val(*sch, item, item!(i)) {
                    contains_errors.push(e);
                } else {
                    contains_matched.push(i);
                    if s.draft_version >= 2020 {
                        self.uneval.items.remove(&i);
                    }
                }
            }

            // minContains --
            if let Some(min) = s.min_contains {
                if contains_matched.len() < min {
                    let mut e = self.error(kind!(MinContains, contains_matched.clone(), min));
                    e.causes = contains_errors;
                    self.errors.push(e);
                }
            } else if contains_matched.is_empty() {
                let mut e = self.error(kind!(Contains));
                e.causes = contains_errors;
                self.errors.push(e);
            }

            // maxContains --
            if let Some(max) = s.max_contains {
                if contains_matched.len() > max {
                    self.add_error(kind!(MaxContains, contains_matched, max));
                }
            }
        }
    }

    fn str_validate(&mut self, str: &'v String) {
        let s = self.schema;
        let mut len = None;

        // minLength --
        if let Some(min) = s.min_length {
            let len = len.get_or_insert_with(|| str.chars().count());
            if *len < min {
                self.add_error(kind!(MinLength, *len, min));
            }
        }

        // maxLength --
        if let Some(max) = s.max_length {
            let len = len.get_or_insert_with(|| str.chars().count());
            if *len > max {
                self.add_error(kind!(MaxLength, *len, max));
            }
        }

        // pattern --
        if let Some(regex) = &s.pattern {
            if !regex.is_match(str) {
                self.add_error(kind!(Pattern, str.clone(), regex.as_str()));
            }
        }

        if s.draft_version >= 7 {
            // contentEncoding --
            let mut decoded = Cow::from(str.as_bytes());
            if let Some(decoder) = &s.content_encoding {
                match (decoder.func)(str) {
                    Ok(bytes) => decoded = Cow::from(bytes),
                    Err(e) => self.add_error(kind!(ContentEncoding, str.clone(), decoder.name, e)),
                }
            }

            // contentMediaType --
            let mut deserialized = None;
            if let Some(mt) = &s.content_media_type {
                match (mt.func)(decoded.as_ref(), s.content_schema.is_some()) {
                    Ok(des) => deserialized = des,
                    Err(e) => {
                        self.add_error(kind!(ContentMediaType, decoded.into(), mt.name, e));
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
    }

    fn num_validate(&mut self, num: &'v Number) {
        let s = self.schema;

        // minimum --
        if let Some(min) = &s.minimum {
            if let (Some(minf), Some(numf)) = (min.as_f64(), num.as_f64()) {
                if numf < minf {
                    self.add_error(kind!(Minimum, num.clone(), min.clone()));
                }
            }
        }

        // maximum --
        if let Some(max) = &s.maximum {
            if let (Some(maxf), Some(numf)) = (max.as_f64(), num.as_f64()) {
                if numf > maxf {
                    self.add_error(kind!(Maximum, num.clone(), max.clone()));
                }
            }
        }

        // exclusiveMinimum --
        if let Some(ex_min) = &s.exclusive_minimum {
            if let (Some(ex_minf), Some(numf)) = (ex_min.as_f64(), num.as_f64()) {
                if numf <= ex_minf {
                    self.add_error(kind!(ExclusiveMinimum, num.clone(), ex_min.clone()));
                }
            }
        }

        // exclusiveMaximum --
        if let Some(ex_max) = &s.exclusive_maximum {
            if let (Some(ex_maxf), Some(numf)) = (ex_max.as_f64(), num.as_f64()) {
                if numf >= ex_maxf {
                    self.add_error(kind!(ExclusiveMaximum, num.clone(), ex_max.clone()));
                }
            }
        }

        // multipleOf --
        if let Some(mul) = &s.multiple_of {
            if let (Some(mulf), Some(numf)) = (mul.as_f64(), num.as_f64()) {
                if (numf / mulf).fract() != 0.0 {
                    self.add_error(kind!(MultipleOf, num.clone(), mul.clone()));
                }
            }
        }
    }
}

// references validation
impl<'v, 's, 'd, 'e, 'f> Validator<'v, 's, 'd, 'e, 'f> {
    fn refs_validate(&mut self) {
        let s = self.schema;
        macro_rules! add_err {
            ($result:expr) => {
                if let Err(e) = $result {
                    self.errors.push(e);
                }
            };
        }

        // $recursiveRef --
        if let Some(mut sch) = s.recursive_ref {
            if self.schemas.get(sch).recursive_anchor {
                sch = self.resolve_recursive_anchor().unwrap_or(sch);
            }
            add_err!(self.validate_ref(sch, "$recursiveRef"));
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
            add_err!(self.validate_ref(sch, "$dynamicRef"));
        }
    }

    fn validate_ref(
        &mut self,
        sch: SchemaIndex,
        kw: &'static str,
    ) -> Result<(), ValidationError<'s, 'v>> {
        if let Err(err) = self._validate_self(sch, kw.into(), false) {
            let url = &self.schemas.get(sch).loc;
            let mut ref_err = self.error(ErrorKind::Reference { kw, url });
            if let ErrorKind::Group = err.kind {
                ref_err.causes = err.causes;
            } else {
                ref_err.causes.push(err);
            }
            return Err(ref_err);
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
impl<'v, 's, 'd, 'e, 'f> Validator<'v, 's, 'd, 'e, 'f> {
    fn cond_validate(&mut self) {
        let s = self.schema;
        macro_rules! add_err {
            ($result:expr) => {
                if let Err(e) = $result {
                    self.errors.push(e);
                }
            };
        }

        // not --
        if let Some(not) = s.not {
            if self._validate_self(not, None, true).is_ok() {
                self.add_error(kind!(Not));
            }
        }

        // allOf --
        if !s.all_of.is_empty() {
            let mut allof_errors = vec![];
            for sch in &s.all_of {
                if let Err(e) = self.validate_self(*sch) {
                    allof_errors.push(e);
                    if self.bool_result {
                        break;
                    }
                }
            }
            if !allof_errors.is_empty() {
                self.add_errors(allof_errors, kind!(AllOf));
            }
        }

        // anyOf --
        if !s.any_of.is_empty() {
            let mut matched = false;
            let mut anyof_errors = vec![];
            for sch in &s.any_of {
                match self.validate_self(*sch) {
                    Ok(_) => {
                        matched = true;
                        // for uneval, all schemas must be checked
                        if self.uneval.is_empty() {
                            break;
                        }
                    }
                    Err(e) => anyof_errors.push(e),
                }
            }
            if !matched {
                self.add_errors(anyof_errors, kind!(AnyOf));
            }
        }

        // oneOf --
        if !s.one_of.is_empty() {
            let (mut matched, mut oneof_errors) = (None, vec![]);
            for (i, sch) in s.one_of.iter().enumerate() {
                if let Err(e) = self._validate_self(*sch, None, matched.is_some()) {
                    if matched.is_none() {
                        oneof_errors.push(e);
                    }
                } else {
                    match matched {
                        None => _ = matched.replace(i),
                        Some(prev) => {
                            self.add_error(ErrorKind::OneOf(Some((prev, i))));
                        }
                    }
                }
            }
            if matched.is_none() {
                self.add_errors(oneof_errors, ErrorKind::OneOf(None));
            }
        }

        // if, then, else --
        if let Some(if_) = s.if_ {
            if self._validate_self(if_, None, true).is_ok() {
                if let Some(then) = s.then {
                    add_err!(self.validate_self(then));
                }
            } else if let Some(else_) = s.else_ {
                add_err!(self.validate_self(else_));
            }
        }
    }
}

// uneval validation
impl<'v, 's, 'd, 'e, 'f> Validator<'v, 's, 'd, 'e, 'f> {
    fn uneval_validate(&mut self) {
        let s = self.schema;
        let v = self.v;
        macro_rules! add_err {
            ($result:expr) => {
                if let Err(e) = $result {
                    self.errors.push(e);
                }
            };
        }

        // unevaluatedProps --
        if let (Some(sch), Value::Object(obj)) = (s.unevaluated_properties, v) {
            let uneval = std::mem::take(&mut self.uneval);
            for pname in &uneval.props {
                if let Some(pvalue) = obj.get(*pname) {
                    add_err!(self.validate_val(sch, pvalue, prop!(pname)));
                }
            }
            self.uneval.props.clear();
        }

        // unevaluatedItems --
        if let (Some(sch), Value::Array(arr)) = (s.unevaluated_items, v) {
            let uneval = std::mem::take(&mut self.uneval);
            for i in &uneval.items {
                if let Some(pvalue) = arr.get(*i) {
                    add_err!(self.validate_val(sch, pvalue, item!(*i)));
                }
            }
            self.uneval.items.clear();
        }
    }
}

// validation helpers
impl<'v, 's, 'd, 'e, 'f> Validator<'v, 's, 'd, 'e, 'f> {
    fn validate_val(
        &mut self,
        sch: SchemaIndex,
        v: &'v Value,
        token: InstanceToken<'v>,
    ) -> Result<(), ValidationError<'s, 'v>> {
        let mut vloc = self.vloc.token(token);
        let scope = self.scope.child(sch, None, self.scope.vid + 1);
        let schema = &self.schemas.get(sch);
        Validator {
            v,
            vloc: &mut vloc,
            schema,
            schemas: self.schemas,
            scope,
            uneval: Uneval::from(v, schema, false),
            errors: vec![],
            bool_result: self.bool_result,
        }
        .validate()
        .map(|_| ())
    }

    fn _validate_self(
        &mut self,
        sch: SchemaIndex,
        ref_kw: Option<&'static str>,
        bool_result: bool,
    ) -> Result<(), ValidationError<'s, 'v>> {
        let scope = self.scope.child(sch, ref_kw, self.scope.vid);
        let schema = &self.schemas.get(sch);
        let result = Validator {
            v: self.v,
            vloc: self.vloc,
            schema,
            schemas: self.schemas,
            scope,
            uneval: Uneval::from(self.v, schema, !self.uneval.is_empty()),
            errors: vec![],
            bool_result: self.bool_result || bool_result,
        }
        .validate();
        if let Ok(reply) = &result {
            self.uneval.merge(reply);
        }
        result.map(|_| ())
    }

    #[inline(always)]
    fn validate_self(&mut self, sch: SchemaIndex) -> Result<(), ValidationError<'s, 'v>> {
        self._validate_self(sch, None, false)
    }
}

// error helpers
impl<'v, 's, 'd, 'e, 'f> Validator<'v, 's, 'd, 'e, 'f> {
    #[inline(always)]
    fn error(&self, kind: ErrorKind<'s>) -> ValidationError<'s, 'v> {
        if self.bool_result {
            return ValidationError {
                schema_url: &self.schema.loc,
                instance_location: InstanceLocation::new(),
                kind: ErrorKind::Group,
                causes: vec![],
            };
        }
        ValidationError {
            schema_url: &self.schema.loc,
            instance_location: (&*self.vloc).into(),
            kind,
            causes: vec![],
        }
    }

    #[inline(always)]
    fn add_error(&mut self, kind: ErrorKind<'s>) {
        self.errors.push(self.error(kind));
    }

    #[inline(always)]
    fn add_errors(&mut self, errors: Vec<ValidationError<'s, 'v>>, kind: ErrorKind<'s>) {
        if errors.len() == 1 {
            self.errors.extend(errors);
        } else {
            let mut err = self.error(kind);
            err.causes = errors;
            self.errors.push(err);
        }
    }

    fn kw_loc(&self, mut scope: &Scope) -> String {
        let mut loc = String::new();
        while let Some(parent) = scope.parent {
            if let Some(kw) = scope.ref_kw {
                loc.insert_str(0, kw);
                loc.insert(0, '/');
            } else {
                let cur = &self.schemas.get(scope.sch).loc;
                let parent = &self.schemas.get(parent.sch).loc;
                loc.insert_str(0, &cur[parent.len()..]);
            };
            scope = parent;
        }
        loc
    }

    fn find_missing(
        &self,
        obj: &'v Map<String, Value>,
        required: &'s [String],
    ) -> Option<Vec<&'s str>> {
        let mut missing = required
            .iter()
            .filter(|p| !obj.contains_key(p.as_str()))
            .map(|p| p.as_str());
        if self.bool_result {
            missing.next().map(|_| Vec::new())
        } else {
            let missing = missing.collect::<Vec<_>>();
            if missing.is_empty() {
                None
            } else {
                Some(missing)
            }
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
    // if None, compute from self.sch and self.parent.sh
    // not None only when there is jump i.e $ref, $XXXRef
    ref_kw: Option<&'static str>,
    /// unique id of value being validated
    // if two scope validate same value, they will have same vid
    vid: usize,
    parent: Option<&'a Scope<'a>>,
}

impl<'a> Scope<'a> {
    fn child<'x>(
        &'x self,
        sch: SchemaIndex,
        ref_kw: Option<&'static str>,
        vid: usize,
    ) -> Scope<'x> {
        Scope {
            sch,
            ref_kw,
            vid,
            parent: Some(self),
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

/// Token in InstanceLocation json-pointer.
#[derive(Debug, Clone)]
pub enum InstanceToken<'v> {
    /// Token for property.
    Prop(Cow<'v, str>),
    /// Token for array item.
    Item(usize),
}

impl<'v> InstanceToken<'v> {
    fn to_string(tokens: &[InstanceToken]) -> String {
        use InstanceToken::*;
        let mut r = String::new();
        for tok in tokens {
            r.push('/');
            match tok {
                Prop(s) => r.push_str(&escape(s)),
                Item(i) => write!(&mut r, "{i}").expect("write to String should never fail"),
            }
        }
        r
    }
}

impl<'v> From<String> for InstanceToken<'v> {
    fn from(prop: String) -> Self {
        InstanceToken::Prop(prop.into())
    }
}

impl<'v> From<&'v str> for InstanceToken<'v> {
    fn from(prop: &'v str) -> Self {
        InstanceToken::Prop(prop.into())
    }
}

impl<'v> From<usize> for InstanceToken<'v> {
    fn from(index: usize) -> Self {
        InstanceToken::Item(index)
    }
}

struct JsonPointer<'a, 'v> {
    vec: &'a mut Vec<InstanceToken<'v>>,
    len: usize,
}

impl<'a, 'v> JsonPointer<'a, 'v> {
    fn new(vec: &'a mut Vec<InstanceToken<'v>>) -> Self {
        let len = vec.len();
        Self { vec, len }
    }

    fn token<'x>(&'x mut self, token: InstanceToken<'v>) -> JsonPointer<'x, 'v> {
        self.vec.truncate(self.len);
        self.vec.push(token);
        JsonPointer::new(self.vec)
    }

    fn clone_static<'aa, 'vv>(
        &self,
        vec: &'aa mut Vec<InstanceToken<'vv>>,
    ) -> JsonPointer<'aa, 'vv> {
        for tok in self.vec[..self.len].iter() {
            match tok {
                InstanceToken::Prop(p) => vec.push(p.as_ref().to_owned().into()),
                InstanceToken::Item(i) => vec.push((*i).into()),
            }
        }
        JsonPointer::new(vec)
    }
}

impl<'a, 'v> ToString for JsonPointer<'a, 'v> {
    fn to_string(&self) -> String {
        InstanceToken::to_string(&self.vec[..self.len])
    }
}

/// The location of the JSON value within the instance being validated
#[derive(Debug, Default)]
pub struct InstanceLocation<'v> {
    pub tokens: Vec<InstanceToken<'v>>,
}

impl<'v> InstanceLocation<'v> {
    fn new() -> Self {
        Self::default()
    }

    fn clone_static(self) -> InstanceLocation<'static> {
        let mut tokens = Vec::with_capacity(self.tokens.len());
        for tok in self.tokens {
            let tok = match tok {
                InstanceToken::Prop(p) => InstanceToken::Prop(p.into_owned().into()),
                InstanceToken::Item(i) => InstanceToken::Item(i),
            };
            tokens.push(tok);
        }
        InstanceLocation { tokens }
    }
}

impl<'a, 'v> From<&JsonPointer<'a, 'v>> for InstanceLocation<'v> {
    fn from(value: &JsonPointer<'a, 'v>) -> Self {
        let mut tokens = Vec::with_capacity(value.len);
        for tok in &value.vec[..value.len] {
            tokens.push(tok.clone());
        }
        Self { tokens }
    }
}

impl<'v> ToString for InstanceLocation<'v> {
    fn to_string(&self) -> String {
        InstanceToken::to_string(&self.tokens)
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
