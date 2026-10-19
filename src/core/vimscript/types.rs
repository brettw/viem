//! Vim9 types shared by declarations, calls, casts and mutable containers.
use super::{Value, VimError};

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Type {
    Any,
    Void,
    Bool,
    Number,
    Float,
    String,
    Blob,
    Job,
    Channel,
    List(Box<Type>),
    Dict(Box<Type>),
    Tuple(Vec<Type>, Option<Box<Type>>),
    Func {
        args: Vec<Type>,
        optional: usize,
        rest: Option<Box<Type>>,
        result: Box<Type>,
    },
    Named(String),
    Class(String),
    Alias(String),
    Module,
}

impl std::fmt::Display for Type {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Job => write!(f, "job"),
            Self::Channel => write!(f, "channel"),
            Self::Any => write!(f, "any"),
            Self::Void => write!(f, "void"),
            Self::Bool => write!(f, "bool"),
            Self::Number => write!(f, "number"),
            Self::Float => write!(f, "float"),
            Self::String => write!(f, "string"),
            Self::Blob => write!(f, "blob"),
            Self::List(t) => write!(f, "list<{t}>"),
            Self::Dict(t) => write!(f, "dict<{t}>"),
            Self::Named(n) => write!(f, "{n}"),
            Self::Class(n) => write!(f, "class<{n}>"),
            Self::Alias(n) => write!(f, "typealias<{n}>"),
            Self::Module => write!(f, "import"),
            Self::Tuple(items, rest) => {
                write!(f, "tuple<")?;
                for (i, t) in items.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{t}")?;
                }
                if let Some(t) = rest {
                    if !items.is_empty() {
                        write!(f, ", ")?;
                    }
                    write!(f, "...list<{t}>")?;
                }
                write!(f, ">")
            }
            Self::Func {
                args, rest, result, ..
            } => {
                write!(f, "func(")?;
                for (i, t) in args.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{t}")?;
                }
                if let Some(t) = rest {
                    if !args.is_empty() {
                        write!(f, ", ")?;
                    }
                    write!(f, "...{t}")?;
                }
                write!(f, ")")?;
                if **result != Self::Void {
                    write!(f, ": {result}")?;
                }
                Ok(())
            }
        }
    }
}

impl Type {
    pub(crate) fn accepts(&self, actual: &Type) -> bool {
        if matches!(actual, Self::Class(_) | Self::Alias(_) | Self::Module) {
            return self == actual;
        }
        if *actual == Self::Void {
            return *self == Self::Void;
        }
        if matches!(self, Self::Any) || matches!(actual, Self::Any) {
            return true;
        }
        match (self, actual) {
            (Self::Float, Self::Number) | (Self::Bool, Self::Number) => true,
            (Self::List(a), Self::List(b)) | (Self::Dict(a), Self::Dict(b)) => {
                a.container_accepts(b)
            }
            (
                Self::Func {
                    args: a,
                    optional: ao,
                    rest: ar,
                    result: av,
                },
                Self::Func {
                    args: b,
                    optional: bo,
                    rest: br,
                    result: bv,
                },
            ) => {
                av.accepts(bv)
                    && (ar.as_deref() == Some(&Self::Any) && a.is_empty()
                        || (a.len() == b.len()
                            && ao <= bo
                            && a.iter().zip(b).all(|(a, b)| b.accepts(a))
                            && ar == br))
            }
            _ => self == actual,
        }
    }
    fn container_accepts(&self, other: &Type) -> bool {
        if *self == Type::Any || *other == Type::Any {
            return true;
        }
        match (self, other) {
            (Type::List(a), Type::List(b)) | (Type::Dict(a), Type::Dict(b)) => {
                a.container_accepts(b)
            }
            _ => self == other,
        }
    }
    pub(crate) fn require(&self, actual: &Type) -> Result<(), VimError> {
        actual.require_value()?;
        if self.accepts(actual) {
            Ok(())
        } else {
            Err(VimError::new(
                1012,
                format!("Type mismatch; expected {self} but got {actual}"),
            ))
        }
    }
    pub(crate) fn require_value(&self) -> Result<(), VimError> {
        match self {
            Self::Class(name) => Err(VimError::new(
                1405,
                format!("Class \"{name}\" cannot be used as a value"),
            )),
            Self::Alias(name) => Err(VimError::new(
                1403,
                format!("Type alias \"{name}\" cannot be used as a value"),
            )),
            Self::Module => Err(VimError::new(1094, "Import can only be used with a member")),
            _ => Ok(()),
        }
    }
    pub(crate) fn require_in(&self, actual: &Type, rt: &super::Runtime) -> Result<(), VimError> {
        match (self, actual) {
            (Self::Named(expected), Self::Named(actual)) => {
                if class_for_type(rt, actual).is_some_and(|class| class.is_a(expected)) {
                    return Ok(());
                }
            }
            (Self::List(_), Self::List(_)) | (Self::Dict(_), Self::Dict(_)) => {
                return self.require(actual)
            }
            (Self::Tuple(a, ar), Self::Tuple(b, br)) => {
                if b.len() >= a.len()
                    && (ar.is_some() || a.len() == b.len())
                    && (br.is_none() || ar.is_some())
                {
                    for (i, b) in b.iter().enumerate() {
                        a.get(i).or(ar.as_deref()).unwrap().require_in(b, rt)?;
                    }
                    if let (Some(a), Some(b)) = (ar, br) {
                        a.require_in(b, rt)?;
                    }
                    return Ok(());
                }
            }
            _ => {}
        }
        self.require(actual)
    }
    pub fn infer(value: &Value) -> Self {
        Self::infer_inner(value, 0, &mut 1_000_001, &mut Default::default())
    }
    fn infer_inner(
        value: &Value,
        depth: usize,
        remaining: &mut usize,
        cache: &mut std::collections::HashMap<usize, Type>,
    ) -> Self {
        if depth > 20 || *remaining == 0 {
            return Self::Any;
        }
        *remaining -= 1;
        let identity = match value {
            Value::List(list) => Some(list.id()),
            Value::Dict(dict) => Some(dict.id()),
            Value::Tuple(items) => Some(std::sync::Arc::as_ptr(items) as usize),
            _ => None,
        };
        if let Some(id) = identity {
            if let Some(ty) = cache.get(&id) {
                return ty.clone();
            }
            // A cycle has no more specific finite inferred member type.
            cache.insert(id, Self::Any);
        }
        fn common(
            values: Vec<Value>,
            depth: usize,
            remaining: &mut usize,
            cache: &mut std::collections::HashMap<usize, Type>,
        ) -> Type {
            let mut types = values
                .iter()
                .map(|v| Type::infer_inner(v, depth + 1, remaining, cache));
            let first = types.next().unwrap_or(Type::Any);
            if types.all(|t| t == first) {
                first
            } else {
                Type::Any
            }
        }
        let ty = match value {
            Value::NullTyped(t) | Value::Type(t) => t.as_ref().clone(),
            Value::Void => Self::Void,
            Value::TypeAlias(a) => Self::Alias(a.name.to_str_lossy().into_owned()),
            Value::Module(_) => Self::Module,
            Value::Tuple(items) => Self::Tuple(
                items
                    .iter()
                    .map(|v| Self::infer_inner(v, depth + 1, remaining, cache))
                    .collect(),
                None,
            ),
            Value::Class(c) => Self::Class(c.type_name()),
            Value::Object(o) => Self::Named(o.class.type_name()),
            Value::Number(_) => Self::Number,
            Value::Float(_) => Self::Float,
            Value::Bool(_) => Self::Bool,
            Value::String(_) => Self::String,
            Value::Blob(_) => Self::Blob,
            Value::List(list) => {
                let declared = list.data().member_type.clone();
                Self::List(Box::new(declared.unwrap_or_else(|| {
                    common(list.snapshot(), depth, remaining, cache)
                })))
            }
            Value::Dict(dict) => {
                let declared = dict.data().member_type.clone();
                Self::Dict(Box::new(declared.unwrap_or_else(|| {
                    common(
                        dict.snapshot().into_iter().map(|(_, v)| v).collect(),
                        depth,
                        remaining,
                        cache,
                    )
                })))
            }
            Value::Funcref(f) => f.declared_type().unwrap_or(Self::Func {
                args: vec![],
                optional: 0,
                rest: Some(Box::new(Self::Any)),
                result: Box::new(Self::Any),
            }),
            Value::None | Value::Null => Self::Any,
        };
        if let Some(id) = identity {
            cache.insert(id, ty.clone());
        }
        ty
    }

    pub fn attach(&self, value: &Value) {
        self.attach_inner(value, &mut std::collections::HashSet::new());
    }
    fn attach_inner(&self, value: &Value, seen: &mut std::collections::HashSet<usize>) {
        match (self, value) {
            (Self::List(t), Value::List(list)) if **t != Self::Any => {
                if seen.insert(list.id()) {
                    list.data().member_type = Some((**t).clone());
                    for value in list.snapshot() {
                        t.attach_inner(&value, seen);
                    }
                }
            }
            (Self::Dict(t), Value::Dict(dict)) if **t != Self::Any => {
                if seen.insert(dict.id()) {
                    dict.data().member_type = Some((**t).clone());
                    for (_, value) in dict.snapshot() {
                        t.attach_inner(&value, seen);
                    }
                }
            }
            _ => {}
        }
    }

    pub fn default_value(&self) -> Value {
        match self {
            Self::Tuple(items, _) => Value::Tuple(std::sync::Arc::new(
                items.iter().map(Self::default_value).collect(),
            )),
            Self::Bool => Value::Bool(false),
            Self::Float => Value::Float(0.0),
            Self::String => Value::str(""),
            Self::List(_) => Value::list(vec![]),
            Self::Dict(_) => Value::Dict(super::Dict::new()),
            Self::Blob => Value::Blob(super::value::Blob::new(vec![])),
            Self::Func { .. } | Self::Named(_) | Self::Job | Self::Channel => {
                Value::null_typed(self.clone())
            }
            _ => Value::Number(0),
        }
    }

    /// Validate before publishing a binding or a collection mutation. Traversal
    /// snapshots containers before descending, and bounds cyclic input.
    pub fn check(&self, value: Value) -> Result<Value, VimError> {
        self.check_inner(value, 0, &mut 1_000_001, &mut Default::default())
    }
    fn check_inner(
        &self,
        value: Value,
        depth: usize,
        remaining: &mut usize,
        seen: &mut std::collections::HashSet<(usize, Type)>,
    ) -> Result<Value, VimError> {
        if *remaining == 0 {
            return Err(VimError::new(342, "Type validation budget exhausted"));
        }
        *remaining -= 1;
        let identity = match &value {
            Value::List(list) => Some(list.id()),
            Value::Dict(dict) => Some(dict.id()),
            Value::Tuple(items) => Some(std::sync::Arc::as_ptr(items) as usize),
            _ => None,
        };
        if let Some(id) = identity {
            if !seen.insert((id, self.clone())) {
                return Ok(value);
            }
        }
        if depth > 100 {
            return Err(VimError::new(132, "Type nesting too deep"));
        }
        if depth > 0 && matches!((self, &value), (Self::Float | Self::Bool, Value::Number(_))) {
            return Err(VimError::new(
                1012,
                format!("Type mismatch; expected {self} but got number"),
            ));
        }
        let valid = match (self, &value) {
            (_, Value::Class(class)) => {
                return Err(VimError::new(
                    1405,
                    format!("Class \"{}\" cannot be used as a value", class.name),
                ))
            }
            (_, Value::TypeAlias(alias)) => {
                return Err(VimError::new(
                    1403,
                    format!(
                        "Type alias \"{}\" cannot be used as a value",
                        alias.name.to_str_lossy()
                    ),
                ))
            }
            (_, Value::Void) => return Err(VimError::new(1031, "Cannot use void value")),
            (_, Value::Module(_)) => false,
            (expected, Value::NullTyped(actual)) => {
                expected.accepts(actual)
                    || matches!(
                        (expected, actual.as_ref()),
                        (Self::Named(_), Self::Named(_))
                    )
            }
            (Self::Any, _) => true,
            (Self::Tuple(types, rest), Value::Tuple(items)) => {
                if items.len() < types.len() || (rest.is_none() && items.len() != types.len()) {
                    false
                } else {
                    for (i, v) in items.iter().enumerate() {
                        types.get(i).or(rest.as_deref()).unwrap().check_inner(
                            v.clone(),
                            depth + 1,
                            remaining,
                            seen,
                        )?;
                    }
                    true
                }
            }
            (Self::Named(name), Value::Object(object)) => object.class.is_a(name),
            (Self::Named(_), Value::Null) => true,
            (Self::Number, Value::Number(_))
            | (Self::Float, Value::Float(_))
            | (Self::Bool, Value::Bool(_))
            | (Self::String, Value::String(_))
            | (Self::Blob, Value::Blob(_))
            | (Self::Func { .. }, Value::Null) => true,
            (Self::Func { .. }, Value::Funcref(f)) => f
                .declared_type()
                .as_ref()
                .is_none_or(|actual| self.accepts(actual)),
            (Self::Float, Value::Number(n)) => return Ok(Value::Float(*n as f64)),
            (Self::Bool, Value::Number(n @ (0 | 1))) => return Ok(Value::Bool(*n != 0)),
            (Self::List(member), Value::List(list)) => {
                let previous = list.data().member_type.clone();
                if let Some(previous) = previous
                    .filter(|t| *t != Self::Any && **member != Self::Any && t != member.as_ref())
                {
                    return Err(VimError::new(
                        1012,
                        format!("Type mismatch; expected list<{member}> but got list<{previous}>"),
                    ));
                }
                for item in list.snapshot() {
                    member.check_inner(item, depth + 1, remaining, seen)?;
                }
                true
            }
            (Self::Dict(member), Value::Dict(dict)) => {
                let previous = dict.data().member_type.clone();
                if let Some(previous) = previous
                    .filter(|t| *t != Self::Any && **member != Self::Any && t != member.as_ref())
                {
                    return Err(VimError::new(
                        1012,
                        format!("Type mismatch; expected dict<{member}> but got dict<{previous}>"),
                    ));
                }
                for (_, item) in dict.snapshot() {
                    member.check_inner(item, depth + 1, remaining, seen)?;
                }
                true
            }
            _ => false,
        };
        if valid {
            Ok(value)
        } else {
            Err(VimError::new(
                1012,
                format!(
                    "Type mismatch; expected {self} but got {}",
                    Self::infer(&value)
                ),
            ))
        }
    }
}

pub(crate) fn class_for_type(
    rt: &super::Runtime,
    name: &str,
) -> Option<std::sync::Arc<super::objects::Class>> {
    if let Some(class) = rt.state.class(name) {
        return Some(class);
    }
    let (sid, name) = name
        .strip_prefix("<SNR>")
        .and_then(|s| s.split_once('_'))
        .and_then(|(sid, name)| sid.parse::<usize>().ok().map(|sid| (sid, name)))
        .unwrap_or((rt.current_sid(), name));
    rt.state
        .script_vars(sid)
        .and_then(|d| d.get(name.as_bytes()))
        .and_then(|v| {
            if let Value::Class(c) = v {
                Some(c)
            } else {
                None
            }
        })
}

/// Parse one type, leaving expression punctuation to the caller.
pub fn parse(src: &[u8], at: &mut usize) -> Result<Type, VimError> {
    parse_inner(src, at, 0)
}
fn parse_inner(src: &[u8], at: &mut usize, depth: usize) -> Result<Type, VimError> {
    if depth > 100 {
        return Err(VimError::new(132, "Type nesting too deep"));
    }
    if *at > 0
        && src.get(*at - 1) == Some(&b':')
        && !src.get(*at).is_some_and(u8::is_ascii_whitespace)
    {
        return Err(VimError::new(1069, "White space required after colon"));
    }
    *at = super::expr::skip_white(src, *at);
    let start = *at;
    while src
        .get(*at)
        .is_some_and(|c| c.is_ascii_alphanumeric() || matches!(c, b'_' | b'.'))
    {
        *at += 1;
    }
    let name = String::from_utf8_lossy(&src[start..*at]);
    let scalar = match name.as_ref() {
        "any" | "unknown" => Some(Type::Any),
        "void" => Some(Type::Void),
        "bool" => Some(Type::Bool),
        "number" => Some(Type::Number),
        "float" => Some(Type::Float),
        "job" => Some(Type::Job),
        "channel" => Some(Type::Channel),
        "string" => Some(Type::String),
        "blob" => Some(Type::Blob),
        _ => None,
    };
    if let Some(t) = scalar {
        return Ok(t);
    }
    if matches!(name.as_ref(), "list" | "dict") {
        if src.get(*at) != Some(&b'<') {
            return Err(VimError::new(1008, "Missing <type>"));
        }
        *at += 1;
        let member = parse_inner(src, at, depth + 1)?;
        *at = super::expr::skip_white(src, *at);
        if src.get(*at) != Some(&b'>') {
            return Err(VimError::new(1009, "Missing > after type"));
        }
        *at += 1;
        return Ok(if name == "list" {
            Type::List(Box::new(member))
        } else {
            Type::Dict(Box::new(member))
        });
    }
    if name == "object" {
        if src.get(*at) != Some(&b'<') {
            return Err(VimError::new(1008, "Missing <type>"));
        }
        *at += 1;
        let ty = parse_inner(src, at, depth + 1)?;
        *at = super::expr::skip_white(src, *at);
        if src.get(*at) != Some(&b'>') || !matches!(ty, Type::Named(_)) {
            return Err(VimError::new(1010, "Class type required"));
        }
        *at += 1;
        return Ok(ty);
    }
    if name == "tuple" {
        if src.get(*at) != Some(&b'<') {
            return Err(VimError::new(1008, "Missing <type>"));
        }
        *at += 1;
        let mut items = vec![];
        let mut rest = None;
        while src.get(super::expr::skip_white(src, *at)) != Some(&b'>') {
            *at = super::expr::skip_white(src, *at);
            if rest.is_some() {
                return Err(VimError::new(1539, "Variadic type must be last").into());
            }
            if src.get(*at..).is_some_and(|s| s.starts_with(b"...")) {
                *at += 3;
                let Type::List(t) = parse_inner(src, at, depth + 1)? else {
                    return Err(VimError::new(1539, "Variadic tuple needs list type"));
                };
                rest = Some(t);
            } else {
                items.push(parse_inner(src, at, depth + 1)?);
            }
            *at = super::expr::skip_white(src, *at);
            if src.get(*at) == Some(&b',') {
                *at += 1;
            } else if src.get(*at) != Some(&b'>') {
                return Err(VimError::new(1009, "Missing >"));
            }
        }
        *at = super::expr::skip_white(src, *at) + 1;
        return Ok(Type::Tuple(items, rest));
    }
    if name == "func" {
        if src.get(*at) != Some(&b'(') {
            *at = super::expr::skip_white(src, *at);
            let result = if src.get(*at) == Some(&b':') {
                *at += 1;
                parse_inner(src, at, depth + 1)?
            } else {
                Type::Any
            };
            return Ok(Type::Func {
                args: vec![],
                optional: 0,
                rest: Some(Box::new(Type::Any)),
                result: Box::new(result),
            });
        }
        *at += 1;
        let mut args = vec![];
        let mut rest = None;
        let mut optional = 0;
        loop {
            *at = super::expr::skip_white(src, *at);
            if src.get(*at) == Some(&b')') {
                *at += 1;
                break;
            }
            let variadic = src.get(*at..).is_some_and(|s| s.starts_with(b"..."));
            if variadic {
                *at += 3;
            }
            let opt = src.get(*at) == Some(&b'?');
            if opt {
                *at += 1;
                optional += 1;
            }
            let t = parse_inner(src, at, depth + 1)?;
            if variadic {
                rest = Some(Box::new(t));
            } else {
                args.push(t);
            }
            *at = super::expr::skip_white(src, *at);
            if src.get(*at) == Some(&b',') {
                *at += 1;
            } else if src.get(*at) != Some(&b')') {
                return Err(VimError::new(110, "Missing ')'"));
            }
        }
        *at = super::expr::skip_white(src, *at);
        let result = if src.get(*at) == Some(&b':') {
            *at += 1;
            parse_inner(src, at, depth + 1)?
        } else {
            Type::Void
        };
        return Ok(Type::Func {
            args,
            optional,
            rest,
            result: Box::new(result),
        });
    }
    if !name.is_empty() && (name.as_bytes()[0].is_ascii_uppercase() || name.contains('.')) {
        Ok(Type::Named(name.into_owned()))
    } else {
        Err(VimError::new(1010, format!("Type not recognized: {name}")))
    }
}

pub(crate) fn resolve(ty: Type, rt: &super::Runtime) -> Type {
    match ty {
        Type::Named(ref name) => {
            if let Some(value) = rt
                .current_frame()
                .and_then(|f| f.function.as_ref())
                .and_then(|f| f.signature.as_ref())
                .and_then(|s| s.bindings.get(name))
            {
                return value.clone();
            }
            if let Some((alias, member)) = name.split_once('.') {
                if let Some(path) = rt.state.import(rt.current_sid(), alias.as_bytes()) {
                    if let Some(script) = rt.state.scripts().into_iter().find(|s| s.name == path) {
                        if script.vars.exported_member(member.as_bytes()).is_ok() {
                            if let Some(value) = script.types.get(member) {
                                return value.clone();
                            }
                            if let Some(Value::Class(class)) = script.vars.get(member.as_bytes()) {
                                return Type::Named(class.type_name());
                            }
                        }
                    }
                }
            }
            if let Some(Value::Class(class)) = rt
                .state
                .script_vars(rt.current_sid())
                .and_then(|vars| vars.get(name.as_bytes()))
            {
                return Type::Named(class.type_name());
            }
            rt.state.type_alias(rt.current_sid(), name).unwrap_or(ty)
        }
        Type::List(t) => Type::List(Box::new(resolve(*t, rt))),
        Type::Dict(t) => Type::Dict(Box::new(resolve(*t, rt))),
        Type::Tuple(items, rest) => Type::Tuple(
            items.into_iter().map(|t| resolve(t, rt)).collect(),
            rest.map(|t| Box::new(resolve(*t, rt))),
        ),
        Type::Func {
            args,
            optional,
            rest,
            result,
        } => Type::Func {
            args: args.into_iter().map(|t| resolve(t, rt)).collect(),
            optional,
            rest: rest.map(|t| Box::new(resolve(*t, rt))),
            result: Box::new(resolve(*result, rt)),
        },
        ty => ty,
    }
}

pub(crate) fn parse_in(src: &[u8], at: &mut usize, rt: &super::Runtime) -> Result<Type, VimError> {
    Ok(resolve(parse(src, at)?, rt))
}

/// A type reference is a use of an explicitly imported module. Resolve it
/// through the same host-controlled loader before binding its class identity.
pub(crate) fn parse_active(
    src: &[u8],
    at: &mut usize,
    rt: &mut super::Runtime,
    host: &mut dyn super::ScriptHost,
) -> super::EvalResult<Type> {
    let ty = parse(src, at)?;
    load_type_imports(&ty, rt, host)?;
    Ok(resolve(ty, rt))
}

fn load_type_imports(
    ty: &Type,
    rt: &mut super::Runtime,
    host: &mut dyn super::ScriptHost,
) -> super::EvalResult<()> {
    match ty {
        Type::Named(name) => {
            if let Some((alias, _)) = name.split_once('.') {
                if let Some(path) = rt.state.import(rt.current_sid(), alias.as_bytes()) {
                    super::vim9::module(rt, host, &path)?;
                }
            }
        }
        Type::List(t) | Type::Dict(t) => load_type_imports(t, rt, host)?,
        Type::Tuple(items, rest) => {
            for ty in items {
                load_type_imports(ty, rt, host)?;
            }
            if let Some(rest) = rest {
                load_type_imports(rest, rt, host)?;
            }
        }
        Type::Func {
            args, rest, result, ..
        } => {
            for ty in args {
                load_type_imports(ty, rt, host)?;
            }
            if let Some(rest) = rest {
                load_type_imports(rest, rt, host)?;
            }
            load_type_imports(result, rt, host)?;
        }
        _ => {}
    }
    Ok(())
}

pub(crate) fn validate(ty: &Type, rt: &super::Runtime) -> Result<(), VimError> {
    match ty {
        Type::Named(name) if class_for_type(rt, name).is_none() => {
            let generic = rt
                .current_frame()
                .and_then(|f| f.function.as_ref())
                .and_then(|f| f.signature.as_ref())
                .is_some_and(|s| s.generics.contains(name) || s.bindings.contains_key(name));
            if !generic {
                return Err(VimError::new(1010, format!("Type not recognized: {name}")));
            }
        }
        Type::List(t) | Type::Dict(t) => validate(t, rt)?,
        Type::Tuple(items, rest) => {
            for ty in items {
                validate(ty, rt)?;
            }
            if let Some(t) = rest {
                validate(t, rt)?;
            }
        }
        Type::Func {
            args, rest, result, ..
        } => {
            for ty in args {
                validate(ty, rt)?;
            }
            if let Some(t) = rest {
                validate(t, rt)?;
            }
            validate(result, rt)?;
        }
        _ => {}
    }
    Ok(())
}

pub(crate) fn validate_storage(ty: &Type, rt: &super::Runtime) -> Result<(), VimError> {
    validate(ty, rt)?;
    match ty {
        Type::Void => return Err(VimError::new(1031, "Cannot use void value")),
        Type::List(t) | Type::Dict(t) => validate_storage(t, rt)?,
        Type::Tuple(items, rest) => {
            for ty in items {
                validate_storage(ty, rt)?;
            }
            if let Some(t) = rest {
                validate_storage(t, rt)?;
            }
        }
        Type::Func {
            args, rest, result, ..
        } => {
            for ty in args {
                validate_storage(ty, rt)?;
            }
            if let Some(t) = rest {
                validate_storage(t, rt)?;
            }
            if **result != Type::Void {
                validate_storage(result, rt)?;
            }
        }
        _ => {}
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shared_type_graphs_do_not_expand_into_exponential_walks() {
        let mut value = Value::Number(1);
        let mut ty = Type::Number;
        // Only 49 distinct nodes, with 2^48 logical paths to the leaves.
        for _ in 0..48 {
            value = Value::list(vec![value.clone(), value]);
            ty = Type::List(Box::new(ty));
        }
        assert!(ty.check(value.clone()).is_ok());
        let inferred = Type::infer(&value);
        assert!(matches!(inferred, Type::List(_)));
        Value::clear_values(vec![value]);
    }
}

pub(crate) fn substitute(ty: &Type, bindings: &std::collections::BTreeMap<String, Type>) -> Type {
    match ty {
        Type::Named(name) => bindings.get(name).cloned().unwrap_or_else(|| ty.clone()),
        Type::List(t) => Type::List(Box::new(substitute(t, bindings))),
        Type::Dict(t) => Type::Dict(Box::new(substitute(t, bindings))),
        Type::Tuple(items, rest) => Type::Tuple(
            items.iter().map(|t| substitute(t, bindings)).collect(),
            rest.as_ref().map(|t| Box::new(substitute(t, bindings))),
        ),
        Type::Func {
            args,
            optional,
            rest,
            result,
        } => Type::Func {
            args: args.iter().map(|t| substitute(t, bindings)).collect(),
            optional: *optional,
            rest: rest.as_ref().map(|t| Box::new(substitute(t, bindings))),
            result: Box::new(substitute(result, bindings)),
        },
        ty => ty.clone(),
    }
}
