use std::{
    borrow::Cow,
    fmt::{Display, Formatter, Write},
};

use serde::{
    ser::{SerializeMap, SerializeSeq},
    Serialize,
};

use crate::{
    util::*, validator::AbsoluteKeywordLocation, ErrorKind, InstanceLocation, ValidationError,
};

impl<'s, 'v> ValidationError<'s, 'v> {
    fn skip(&self) -> bool {
        self.causes.len() == 1 && matches!(self.kind, ErrorKind::Reference { .. })
    }

    pub fn flag_output(&self) -> FlagOutput {
        FlagOutput { valid: false }
    }

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
                        if let ErrorKind::Reference { url } = &e.kind {
                            Some(Cow::Owned(AbsoluteKeywordLocation {
                                schema_url: url,
                                keyword_path: None,
                            }))
                        } else {
                            Some(Cow::Borrowed(&e.absolute_keyword_location))
                        }
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
                        if let ErrorKind::Reference { url } = &e.kind {
                            Some(Cow::Owned(AbsoluteKeywordLocation {
                                schema_url: url,
                                keyword_path: None,
                            }))
                        } else {
                            Some(Cow::Borrowed(&e.absolute_keyword_location))
                        }
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
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut indent = Indent::default();
        let mut sloc = SchemaLocation::default();
        let mut kw_loc = KeywordLocation::default();
        for node in DfsIterator::new(self) {
            match node {
                DfsItem::Pre(e) => {
                    kw_loc.pre(e);
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
                            write!(f, " [{}]", kw_loc.get(e))?;
                            write!(f, " [{}]", e.absolute_keyword_location)?;
                        }
                        write!(f, ": {}", e.kind)?;
                    }
                }
                DfsItem::Post(e) => {
                    kw_loc.post();
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
            ErrorKind::Reference { url } => Cow::Borrowed(url),
            _ => Cow::Owned(cur.absolute_keyword_location.to_string()),
        };

        let Some(prev) = iter.next_back() else {
            return write!(f, "{cur}")
        };

        let p = match &prev.kind {
            ErrorKind::Schema { url } => {
                let (p, _) = split(url);
                p
            }
            ErrorKind::Reference { url } => {
                let (p, _) = split(url);
                p
            }
            _ => {
                let (p, _) = split(prev.absolute_keyword_location.schema_url);
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
            ErrorKind::Reference { url } => url,
            _ => e.absolute_keyword_location.schema_url,
        };

        if let Some((prev, _)) = self.stack.last() {
            self.loc
                .push_str(&e.absolute_keyword_location.schema_url[prev.len()..]); // todo: url-decode
            if let ErrorKind::Reference { .. } = &e.kind {
                let ref_keyword = e
                    .absolute_keyword_location
                    .keyword_path
                    .as_ref()
                    .map(|p| p.keyword)
                    .unwrap_or_default();
                self.loc.push('/');
                self.loc.push_str(ref_keyword);
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
        } else if let Some(kw_path) = &cur.absolute_keyword_location.keyword_path {
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

pub struct OutputUnit<'e, 's, 'v> {
    pub valid: bool,
    pub keyword_location: String,
    pub absolute_keyword_location: Option<Cow<'e, AbsoluteKeywordLocation<'s>>>,
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

pub enum OutputError<'e, 's, 'v> {
    Leaf(&'e ErrorKind<'s>),
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
