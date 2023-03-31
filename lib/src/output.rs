use std::{
    borrow::Cow,
    fmt::{Display, Formatter, Write},
};

use serde::{
    ser::{SerializeMap, SerializeSeq},
    Serialize,
};

use crate::{util::*, ErrorKind, InstanceLocation, ValidationError};

impl<'s, 'v> ValidationError<'s, 'v> {
    fn absolute_keyword_location(&self) -> AbsoluteKeywordLocation<'s> {
        if let ErrorKind::Reference { url, .. } = &self.kind {
            AbsoluteKeywordLocation {
                schema_url: url,
                keyword_path: None,
            }
        } else {
            AbsoluteKeywordLocation {
                schema_url: self.schema_url,
                keyword_path: self.kind.keyword_path(),
            }
        }
    }

    fn skip(&self) -> bool {
        self.causes.len() == 1 && matches!(self.kind, ErrorKind::Reference { .. })
    }

    /// The `Flag` output format, merely the boolean result.
    pub fn flag_output(&self) -> FlagOutput {
        FlagOutput { valid: false }
    }

    /// The `Basic` structure, a flat list of output units.
    pub fn basic_output(&self) -> OutputUnit {
        let mut outputs = vec![];

        let mut in_ref = InRef::default();
        let mut kw_loc = KeywordLocation::default();
        for node in DfsIterator::new(self) {
            match node {
                DfsItem::Pre(e) => {
                    in_ref.pre(e);
                    kw_loc.pre(e);
                    if e.skip() || matches!(e.kind, ErrorKind::Schema { .. }) {
                        continue;
                    }
                    let absolute_keyword_location = if in_ref.get() {
                        Some(e.absolute_keyword_location())
                    } else {
                        None
                    };
                    outputs.push(OutputUnit {
                        valid: false,
                        keyword_location: kw_loc.get(e),
                        absolute_keyword_location,
                        instance_location: &e.instance_location,
                        error: OutputError::Leaf(&e.kind),
                    });
                }
                DfsItem::Post(e) => {
                    in_ref.post();
                    kw_loc.post();
                    if e.skip() || matches!(e.kind, ErrorKind::Schema { .. }) {
                        continue;
                    }
                }
            }
        }

        let error = if outputs.is_empty() {
            OutputError::Leaf(&self.kind)
        } else {
            OutputError::Branch(outputs)
        };
        OutputUnit {
            valid: false,
            keyword_location: String::new(),
            absolute_keyword_location: None,
            instance_location: &self.instance_location,
            error,
        }
    }

    /// The `Detailed` structure, based on the schema.
    pub fn detailed_output(&self) -> OutputUnit {
        let mut root = None;
        let mut stack: Vec<OutputUnit> = vec![];

        let mut in_ref = InRef::default();
        let mut kw_loc = KeywordLocation::default();
        for node in DfsIterator::new(self) {
            match node {
                DfsItem::Pre(e) => {
                    in_ref.pre(e);
                    kw_loc.pre(e);
                    if e.skip() {
                        continue;
                    }
                    let absolute_keyword_location = if in_ref.get() {
                        Some(e.absolute_keyword_location())
                    } else {
                        None
                    };
                    stack.push(OutputUnit {
                        valid: false,
                        keyword_location: kw_loc.get(e),
                        absolute_keyword_location,
                        instance_location: &e.instance_location,
                        error: OutputError::Leaf(&e.kind),
                    });
                }
                DfsItem::Post(e) => {
                    in_ref.post();
                    kw_loc.post();
                    if e.skip() {
                        continue;
                    }
                    let output = stack.pop().unwrap();
                    if let Some(parent) = stack.last_mut() {
                        match &mut parent.error {
                            OutputError::Leaf(_) => {
                                parent.error = OutputError::Branch(vec![output]);
                            }
                            OutputError::Branch(v) => v.push(output),
                        }
                    } else {
                        root.replace(output);
                    }
                }
            }
        }
        root.unwrap()
    }
}

// DfsIterator --

impl<'s, 'v> Display for ValidationError<'s, 'v> {
    /// Formats error hierarchy. Use `#` to show the schema location.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut indent = Indent::default();
        let mut sloc = SchemaLocation::default();
        // let mut kw_loc = KeywordLocation::default();
        for node in DfsIterator::new(self) {
            match node {
                DfsItem::Pre(e) => {
                    // kw_loc.pre(e);
                    if e.skip() {
                        continue;
                    }
                    indent.pre(f)?;
                    if f.alternate() {
                        sloc.pre(e);
                    }
                    if let ErrorKind::Schema { .. } = &e.kind {
                        write!(f, "jsonschema {}", e.kind)?;
                    } else {
                        write!(f, "at {}", quote(&e.instance_location.to_string()))?;
                        if f.alternate() {
                            write!(f, " [{}]", sloc)?;
                            // write!(f, " [{}]", kw_loc.get(e))?;
                            // write!(f, " [{}]", e.absolute_keyword_location())?;
                        }
                        write!(f, ": {}", e.kind)?;
                    }
                }
                DfsItem::Post(e) => {
                    // kw_loc.post();
                    if e.skip() {
                        continue;
                    }
                    indent.post();
                    sloc.post();
                }
            }
        }
        Ok(())
    }
}

struct DfsIterator<'a, 'v, 's> {
    root: Option<&'a ValidationError<'v, 's>>,
    stack: Vec<Frame<'a, 'v, 's>>,
}

impl<'a, 'v, 's> DfsIterator<'a, 'v, 's> {
    fn new(err: &'a ValidationError<'v, 's>) -> Self {
        DfsIterator {
            root: Some(err),
            stack: vec![],
        }
    }
}

impl<'a, 'v, 's> Iterator for DfsIterator<'a, 'v, 's> {
    type Item = DfsItem<&'a ValidationError<'v, 's>>;

    fn next(&mut self) -> Option<Self::Item> {
        let Some(mut frame) = self.stack.pop() else {
            if let Some(err) = self.root.take() {
                self.stack.push(Frame::from(err));
                return Some(DfsItem::Pre(err));
            } else {
                return None;
            }
        };

        if frame.causes.is_empty() {
            return Some(DfsItem::Post(frame.err));
        }

        let err = &frame.causes[0];
        frame.causes = &frame.causes[1..];
        self.stack.push(frame);
        self.stack.push(Frame::from(err));
        Some(DfsItem::Pre(err))
    }
}

struct Frame<'a, 'v, 's> {
    err: &'a ValidationError<'v, 's>,
    causes: &'a [ValidationError<'v, 's>],
}

impl<'a, 'v, 's> Frame<'a, 'v, 's> {
    fn from(err: &'a ValidationError<'v, 's>) -> Self {
        Self {
            err,
            causes: &err.causes,
        }
    }
}

enum DfsItem<T> {
    Pre(T),
    Post(T),
}

// Indent --

#[derive(Default)]
struct Indent {
    n: usize,
}

impl Indent {
    fn pre(&mut self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        if self.n > 0 {
            writeln!(f)?;
            for _ in 0..self.n - 1 {
                write!(f, "  ")?;
            }
            write!(f, "- ")?;
        }
        self.n += 1;
        Ok(())
    }

    fn post(&mut self) {
        self.n -= 1;
    }
}

// SchemaLocation

#[derive(Default)]
struct SchemaLocation<'a, 's, 'v> {
    stack: Vec<&'a ValidationError<'s, 'v>>,
}

impl<'a, 's, 'v> SchemaLocation<'a, 's, 'v> {
    fn pre(&mut self, e: &'a ValidationError<'s, 'v>) {
        self.stack.push(e);
    }

    fn post(&mut self) {
        self.stack.pop();
    }
}

impl<'a, 's, 'v> Display for SchemaLocation<'a, 's, 'v> {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        let mut iter = self.stack.iter().cloned();
        let cur = iter.next_back().unwrap();
        let cur: Cow<str> = match &cur.kind {
            ErrorKind::Schema { url } => Cow::Borrowed(url),
            ErrorKind::Reference { url, .. } => Cow::Borrowed(url),
            _ => Cow::Owned(cur.absolute_keyword_location().to_string()),
        };

        let Some(prev) = iter.next_back() else {
            return write!(f, "{cur}")
        };

        let p = match &prev.kind {
            ErrorKind::Schema { url } => {
                let (p, _) = split(url);
                p
            }
            ErrorKind::Reference { url, .. } => {
                let (p, _) = split(url);
                p
            }
            _ => {
                let (p, _) = split(prev.schema_url);
                p
            }
        };
        let (c, frag) = split(cur.as_ref());
        if c == p {
            write!(f, "S#{frag}")
        } else {
            write!(f, "{cur}")
        }
    }
}

// KeywordLocation --

#[derive(Default)]
struct KeywordLocation<'a> {
    loc: String,
    stack: Vec<(&'a str, usize)>, // (schema_url, len)
}

impl<'a> KeywordLocation<'a> {
    fn pre(&mut self, e: &'a ValidationError) {
        let cur = match &e.kind {
            ErrorKind::Schema { url } => url,
            ErrorKind::Reference { url, .. } => url,
            _ => e.schema_url,
        };

        if let Some((prev, _)) = self.stack.last() {
            self.loc.push_str(&e.schema_url[prev.len()..]); // todo: url-decode
            if let ErrorKind::Reference { kw, .. } = &e.kind {
                self.loc.push('/');
                self.loc.push_str(kw);
            }
        }
        self.stack.push((cur, self.loc.len()));
    }

    fn post(&mut self) {
        self.stack.pop();
        if let Some((_, len)) = self.stack.last() {
            self.loc.truncate(*len);
        }
    }

    fn get(&mut self, cur: &'a ValidationError) -> String {
        if let ErrorKind::Reference { .. } = &cur.kind {
            self.loc.clone()
        } else if let Some(kw_path) = &cur.kind.keyword_path() {
            let len = self.loc.len();
            self.loc.push('/');
            write!(self.loc, "{}", kw_path).expect("write kw_path to String should not fail");
            let loc = self.loc.clone();
            self.loc.truncate(len);
            loc
        } else {
            self.loc.clone()
        }
    }
}

#[derive(Default)]
struct InRef {
    stack: Vec<bool>,
}

impl InRef {
    fn pre(&mut self, e: &ValidationError) {
        let in_ref: bool = self.get() || matches!(e.kind, ErrorKind::Reference { .. });
        self.stack.push(in_ref);
    }

    fn post(&mut self) {
        self.stack.pop();
    }

    fn get(&self) -> bool {
        self.stack.last().cloned().unwrap_or_default()
    }
}

// output formats --

/// Simplest output format, merely the boolean result.
pub struct FlagOutput {
    pub valid: bool,
}

impl Serialize for FlagOutput {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        let mut map = serializer.serialize_map(Some(1))?;
        map.serialize_entry("valid", &self.valid)?;
        map.end()
    }
}

impl Display for FlagOutput {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write_json_to_fmt(f, self)
    }
}

/// Single OutputUnit used in Basic/Detailed output formats.
pub struct OutputUnit<'e, 's, 'v> {
    pub valid: bool,
    pub keyword_location: String,
    /// The absolute, dereferenced location of the validating keyword
    pub absolute_keyword_location: Option<AbsoluteKeywordLocation<'s>>,
    /// The location of the JSON value within the instance being validated
    pub instance_location: &'e InstanceLocation<'v>,
    pub error: OutputError<'e, 's, 'v>,
}

impl<'e, 's, 'v> Serialize for OutputUnit<'e, 's, 'v> {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        let n = 4 + self.absolute_keyword_location.as_ref().map_or(0, |_| 1);
        let mut map = serializer.serialize_map(Some(n))?;
        map.serialize_entry("valid", &self.valid)?;
        map.serialize_entry("keywordLocation", &self.keyword_location.to_string())?;
        if let Some(s) = &self.absolute_keyword_location {
            map.serialize_entry("absoluteKeywordLocation", &s.to_string())?;
        }
        map.serialize_entry("instanceLocation", &self.instance_location.to_string())?;
        let pname = match self.error {
            OutputError::Leaf(_) => "error",
            OutputError::Branch(_) => "errors",
        };
        map.serialize_entry(pname, &self.error)?;
        map.end()
    }
}

impl<'e, 's, 'v> Display for OutputUnit<'e, 's, 'v> {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write_json_to_fmt(f, self)
    }
}

/// Error of [`OutputUnit`].
pub enum OutputError<'e, 's, 'v> {
    /// Single.
    Leaf(&'e ErrorKind<'s, 'v>),
    /// Nested.
    Branch(Vec<OutputUnit<'e, 's, 'v>>),
}

impl<'e, 's, 'v> Serialize for OutputError<'e, 's, 'v> {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        match self {
            OutputError::Leaf(kind) => serializer.serialize_str(&kind.to_string()),
            OutputError::Branch(units) => {
                let mut seq = serializer.serialize_seq(Some(units.len()))?;
                for unit in units {
                    seq.serialize_element(unit)?;
                }
                seq.end()
            }
        }
    }
}

// AbsoluteKeywordLocation --

impl<'s, 'v> ErrorKind<'s, 'v> {
    pub fn keyword_path(&self) -> Option<KeywordPath<'s>> {
        #[inline(always)]
        fn kw(kw: &'static str) -> Option<KeywordPath> {
            Some(KeywordPath {
                keyword: kw,
                token: None,
            })
        }

        #[inline(always)]
        fn kw_prop<'s>(kw: &'static str, prop: &'s str) -> Option<KeywordPath<'s>> {
            Some(KeywordPath {
                keyword: kw,
                token: Some(SchemaToken::Prop(prop)),
            })
        }

        use ErrorKind::*;
        match self {
            Group => None,
            Schema { .. } => None,
            ContentSchema => kw("contentSchema"),
            PropertyName { .. } => kw("propertyNames"),
            Reference { kw: kword, .. } => kw(kword),
            RefCycle { .. } => None,
            FalseSchema => None,
            Type { .. } => kw("type"),
            Enum { .. } => kw("enum"),
            Const { .. } => kw("const"),
            Format { .. } => kw("format"),
            MinProperties { .. } => kw("minProperties"),
            MaxProperties { .. } => kw("maxProperties"),
            AdditionalProperties { .. } => kw("additionalProperty"),
            Required { .. } => kw("required"),
            Dependency { prop, .. } => kw_prop("dependencies", prop),
            DependentRequired { prop, .. } => kw_prop("dependentRequired", prop),
            MinItems { .. } => kw("minItems"),
            MaxItems { .. } => kw("maxItems"),
            Contains => kw("contains"),
            MinContains { .. } => kw("minContains"),
            MaxContains { .. } => kw("maxContains"),
            UniqueItems { .. } => kw("uniqueItems"),
            AdditionalItems { .. } => kw("additionalItems"),
            MinLength { .. } => kw("minLength"),
            MaxLength { .. } => kw("maxLength"),
            Pattern { .. } => kw("pattern"),
            ContentEncoding { .. } => kw("contentEncoding"),
            ContentMediaType { .. } => kw("contentMediaType"),
            Minimum { .. } => kw("minimum"),
            Maximum { .. } => kw("maximum"),
            ExclusiveMinimum { .. } => kw("exclusiveMinimum"),
            ExclusiveMaximum { .. } => kw("exclusiveMaximum"),
            MultipleOf { .. } => kw("multipleOf"),
            Not => kw("not"),
            AllOf => kw("allOf"),
            AnyOf => kw("anyOf"),
            OneOf(_) => kw("oneOf"),
        }
    }
}

/// The absolute, dereferenced location of the validating keyword
#[derive(Debug, Clone)]
pub struct AbsoluteKeywordLocation<'s> {
    /// The absolute, dereferenced schema location.
    pub schema_url: &'s str,
    /// Location within the `schema_url`.
    pub keyword_path: Option<KeywordPath<'s>>,
}

impl<'s> Display for AbsoluteKeywordLocation<'s> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.schema_url.fmt(f)?;
        if let Some(path) = &self.keyword_path {
            f.write_str("/")?;
            path.keyword.fmt(f)?;
            if let Some(token) = &path.token {
                f.write_str("/")?;
                match token {
                    SchemaToken::Prop(p) => write!(f, "{}", escape(p))?, // todo: url-encode
                    SchemaToken::Item(i) => write!(f, "{i}")?,
                }
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone)]
/// JsonPointer in schema.
pub struct KeywordPath<'s> {
    /// The first token.
    pub keyword: &'static str,
    /// Optinal token within keyword.
    pub token: Option<SchemaToken<'s>>,
}

impl<'s> Display for KeywordPath<'s> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.keyword.fmt(f)?;
        if let Some(token) = &self.token {
            f.write_str("/")?;
            token.fmt(f)?;
        }
        Ok(())
    }
}

/// Token for schema.
#[derive(Debug, Clone)]
pub enum SchemaToken<'s> {
    /// Token for property.
    Prop(&'s str),
    /// Token for array item.
    Item(usize),
}

impl<'s> Display for SchemaToken<'s> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SchemaToken::Prop(p) => write!(f, "{}", escape(p)),
            SchemaToken::Item(i) => write!(f, "{i}"),
        }
    }
}
