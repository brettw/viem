//! Vim9 class metadata and objects; methods execute as ordinary runtime frames.
use super::types::Type;
use super::{Dict, EvalResult, Funcref, Runtime, ScriptHost, VStr, Value, VimError};
use std::collections::BTreeMap;
use std::sync::Arc;

#[derive(Clone, Debug)]
pub struct Field {
    pub sid: usize,
    pub name: String,
    pub ty: Type,
    pub initializer: Option<VStr>,
    pub public: bool,
    pub constant: bool,
    pub deep_constant: bool,
}

#[derive(Debug)]
pub struct Class {
    pub identity: u64,
    pub name: String,
    pub sid: usize,
    pub base: Option<Arc<Class>>,
    pub interfaces: Vec<Arc<Class>>,
    pub fields: Vec<Field>,
    pub methods: BTreeMap<String, VStr>,
    pub static_methods: BTreeMap<String, VStr>,
    pub statics: Dict,
    pub static_fields: Vec<Field>,
    pub abstract_class: bool,
    pub interface: bool,
    pub enumeration: bool,
    pub abstract_methods: std::collections::BTreeSet<String>,
}

#[derive(Debug)]
pub struct Object {
    pub class: Arc<Class>,
    pub fields: Dict,
}

impl Class {
    pub(crate) fn type_name(&self) -> String {
        format!("<SNR>{}_{}#{}", self.sid, self.name, self.identity)
    }
    pub fn is_a(&self, name: &str) -> bool {
        let mut pending = vec![self];
        let mut seen = std::collections::HashSet::new();
        while let Some(class) = pending.pop() {
            if !seen.insert(class.identity) {
                continue;
            }
            if class.name == name || class.type_name() == name {
                return true;
            }
            pending.extend(class.base.as_deref());
            pending.extend(class.interfaces.iter().map(AsRef::as_ref));
        }
        false
    }
    pub fn method(&self, name: &str) -> Option<VStr> {
        let mut class = Some(self);
        while let Some(current) = class {
            if let Some(reference) = current.methods.get(name) {
                return Some(reference.clone());
            }
            class = current.base.as_deref();
        }
        None
    }
}

fn inside(rt: &Runtime, class: &Class) -> bool {
    rt.current_class()
        .is_some_and(|owner| owner.is_a(&class.type_name()) || class.is_a(&owner.type_name()))
}

pub(crate) fn member(rt: &Runtime, value: &Value, name: &str) -> EvalResult<Value> {
    match value {
        Value::Class(class) => {
            if name.starts_with("new") && (name == "new" || class.methods.contains_key(name)) {
                if class.enumeration {
                    return Err(VimError::new(1422, "Enum cannot be instantiated").into());
                }
                return Ok(Value::Funcref(constructor_reference(rt, class, name)));
            }
            if name.starts_with('_') && !inside(rt, class) {
                return Err(VimError::new(1333, "Cannot access protected member").into());
            }
            if let Some(name) = class.static_methods.get(name) {
                let reference = Funcref::named(name.clone());
                return Ok(Value::Funcref(
                    rt.state
                        .function(name.as_bytes())
                        .map_or(reference.clone(), |f| {
                            reference.with_type(super::compiler::signature(&f))
                        }),
                ));
            }
            class.statics.get(name.as_bytes()).ok_or_else(|| {
                VimError::new(
                    1338,
                    format!("Member not found on class {}: {name}", class.name),
                )
                .into()
            })
        }
        Value::Object(object) => {
            if name.starts_with('_') && !inside(rt, &object.class) {
                return Err(VimError::new(1333, "Cannot access protected member").into());
            }
            if let Some(method) = object.class.method(name) {
                let reference = Funcref::object_method(method.clone(), object.clone());
                return Ok(Value::Funcref(
                    rt.state
                        .function(method.as_bytes())
                        .map_or(reference.clone(), |f| {
                            reference.with_type(super::compiler::signature(&f))
                        }),
                ));
            }
            object.fields.get(name.as_bytes()).ok_or_else(|| {
                VimError::new(
                    1326,
                    format!("Member not found on object {}: {name}", object.class.name),
                )
                .into()
            })
        }
        _ => Err(VimError::new(1203, "Dot requires an object or class").into()),
    }
}

pub(crate) fn assign(rt: &Runtime, value: &Value, name: &str, item: Value) -> EvalResult<()> {
    let (class, fields, declarations) = match value {
        Value::Class(class) => (class, &class.statics, &class.static_fields),
        Value::Object(object) => (&object.class, &object.fields, &object.class.fields),
        _ => return Err(VimError::new(1203, "Not an object").into()),
    };
    let field = declarations
        .iter()
        .find(|f| f.name == name)
        .ok_or_else(|| VimError::new(1326, format!("Member not found: {name}")))?;
    if !field.public && !inside(rt, class) {
        return Err(VimError::new(1335, format!("Variable is not writable: {name}")).into());
    }
    let constructor = inside(rt, class)
        && rt
            .current_frame()
            .and_then(|f| f.function.as_ref())
            .is_some_and(|f| {
                f.name
                    .to_str_lossy()
                    .rsplit('.')
                    .next()
                    .is_some_and(|name| name.starts_with("new"))
            });
    if field.constant && !constructor {
        return Err(
            VimError::new(1409, format!("Cannot change read-only variable: {name}")).into(),
        );
    }
    let item = fields.checked_value(name.as_bytes(), item)?;
    fields.insert(name, item);
    Ok(())
}

fn constructor_reference(rt: &Runtime, class: &Arc<Class>, name: &str) -> Funcref {
    let reference = Funcref::constructor(class.clone(), name);
    class
        .methods
        .get(name)
        .and_then(|name| rt.state.function(name.as_bytes()))
        .map_or(reference.clone(), |function| {
            reference.specialized(function)
        })
}

/// Object builtin hooks use the ordinary typed call path, including budgets,
/// compilation, exceptions and deferred calls. No value lock spans execution.
pub(crate) fn builtin(
    rt: &mut Runtime,
    host: &mut dyn ScriptHost,
    value: &Value,
    name: &str,
) -> Option<EvalResult<Value>> {
    let Value::Object(object) = value else {
        return None;
    };
    let method = object.class.method(name)?;
    let reference = Funcref::object_method(method, object.clone());
    Some(rt.call_funcref(host, &reference, vec![], None))
}

pub(crate) fn display(
    rt: &mut Runtime,
    host: &mut dyn ScriptHost,
    value: &Value,
    repr: bool,
) -> EvalResult<Vec<u8>> {
    let vim9 = rt.vim9();
    value.display_with_hook(vim9, repr, &mut |value, bytes| {
        rt.charge(1)?;
        if bytes > super::builtins::MAX_STRING_BYTES {
            return Err(VimError::new(342, "Value display exceeds the string budget").into());
        }
        builtin(rt, host, value, "string")
            .map(|result| result.and_then(|value| Ok(value.to_vstr()?.as_bytes().to_vec())))
            .transpose()
    })
}

pub(crate) fn construct(
    rt: &mut Runtime,
    host: &mut dyn ScriptHost,
    class: Arc<Class>,
    constructor: &str,
    args: Vec<Value>,
    specialized: Option<Arc<super::state::UserFunction>>,
) -> EvalResult<Value> {
    if class.interface || class.abstract_class {
        return Err(
            VimError::new(1325, "Cannot instantiate an abstract class or interface").into(),
        );
    }
    let function = specialized.or_else(|| {
        class
            .methods
            .get(constructor)
            .and_then(|name| rt.state.function(name.as_bytes()))
    });
    if let Some(function) = &function {
        if function
            .signature
            .as_ref()
            .is_some_and(|s| !s.generics.is_empty())
        {
            return Err(VimError::new(1559, "Generic function requires type arguments").into());
        }
        super::compiler::ensure(rt, host, function)?;
    }
    let object = Arc::new(Object {
        class: class.clone(),
        fields: Dict::new(),
    });
    rt.state.track_scope(&object.fields)?;
    // Fields exist at their default values before any initializer executes.
    for field in &class.fields {
        object
            .fields
            .insert(field.name.clone(), field.ty.default_value());
        object
            .fields
            .data()
            .declarations
            .insert(field.name.clone().into(), field.ty.clone());
    }
    for field in &class.fields {
        if let Some(expression) = &field.initializer {
            let scope = Arc::new(super::state::FunctionScope {
                locals: Dict::new(),
                args: Dict::new(),
                self_dict: None,
                self_object: Some(object.clone()),
                outer: None,
                function: VStr::empty(),
            });
            let value = rt.with_context(Some(scope), field.sid, |rt| {
                rt.current_frame_mut().unwrap().class_context = Some(class.clone());
                super::expr::Eval::new(rt, host, expression.as_bytes(), 0, true).eval1()
            })?;
            let ty = if field.ty == Type::Any {
                Type::infer(&value)
            } else {
                field.ty.clone()
            };
            let value = ty.check(value)?;
            ty.attach(&value);
            object.fields.insert(field.name.clone(), value);
            object
                .fields
                .data()
                .declarations
                .insert(field.name.clone().into(), ty);
        }
    }
    if let Some(function) = function {
        rt.call_user_function_receiver(host, function, args, Some(object.clone()))?;
    } else {
        if args.len() > class.fields.len() {
            return Err(VimError::new(118, "Too many arguments for new()").into());
        }
        for (field, value) in class.fields.iter().zip(args) {
            if matches!(value, Value::None) {
                continue;
            }
            let value = object.fields.checked_value(field.name.as_bytes(), value)?;
            object.fields.insert(field.name.clone(), value);
        }
    }
    for field in &class.fields {
        if field.deep_constant {
            if let Some(value) = object.fields.get(field.name.as_bytes()) {
                super::let_cmd::lock_value(&value, i64::MAX, true)?;
            }
        }
    }
    Ok(Value::Object(object))
}

pub(crate) fn define(
    rt: &mut Runtime,
    host: &mut dyn ScriptHost,
    header: &[u8],
    kind: &[u8],
    executing: bool,
) -> super::exec::CommandOutcome {
    use super::exec::{CommandOutcome, Next};
    let terminator = match kind {
        b"interface" => "endinterface",
        b"enum" => "endenum",
        _ => "endclass",
    };
    let Some(frame) = rt.current_frame() else {
        return CommandOutcome {
            next: Next::NextLine,
            result: Err(VimError::new(1318, "Class requires a script").into()),
        };
    };
    let start = frame.line + 1;
    let end = match super::source::definition_end(&frame.lines, start - 1, terminator) {
        Ok(end) => end,
        Err(error) => {
            return CommandOutcome {
                next: Next::NextLine,
                result: Err(error.into()),
            }
        }
    };
    let lines = frame.lines[start..end].to_vec();
    let next = Next::Position(end + 1, 0);
    CommandOutcome {
        next,
        result: if executing {
            build(rt, host, header, kind, lines)
        } else {
            Ok(())
        },
    }
}

fn build(
    rt: &mut Runtime,
    host: &mut dyn ScriptHost,
    header: &[u8],
    kind: &[u8],
    lines: Vec<super::state::SourceLine>,
) -> EvalResult<()> {
    use super::expr::{skip_white, Eval};
    if rt.current_scope().is_some() {
        return Err(VimError::new(1357, "Class can only be defined at script level").into());
    }
    let header = String::from_utf8_lossy(header);
    let mut words = header.split_whitespace();
    let abstract_class = kind == b"abstract";
    if abstract_class && words.next() != Some("class") {
        return Err(VimError::new(475, "Expected abstract class").into());
    }
    let name = words.next().unwrap_or("");
    if !name.as_bytes().first().is_some_and(u8::is_ascii_uppercase)
        || !name.bytes().all(super::expr::is_name_char)
    {
        return Err(VimError::new(1314, "Class name must start with an uppercase letter").into());
    }
    if rt.lookup_variable(host, name.as_bytes(), false).is_ok() {
        return Err(VimError::new(1041, "Redefining script item").into());
    }
    let mut base = None;
    let mut interfaces = vec![];
    while let Some(keyword) = words.next() {
        match keyword {
            "extends" => {
                let parent = words
                    .next()
                    .ok_or_else(|| VimError::new(1315, "Missing base class"))?;
                let value = Eval::new(rt, host, parent.as_bytes(), 0, true).eval1()?;
                let Value::Class(class) = value else {
                    return Err(VimError::new(1354, "Not a class").into());
                };
                if base.is_some()
                    || class.enumeration
                    || class.interface != (kind == b"interface")
                    || kind == b"enum"
                {
                    return Err(VimError::new(1354, "Invalid base class").into());
                }
                base = Some(class);
            }
            "implements" if kind != b"interface" => {
                let names = words.by_ref().collect::<Vec<_>>().join(" ");
                for name in names.split(',').map(str::trim) {
                    let Value::Class(class) =
                        Eval::new(rt, host, name.as_bytes(), 0, true).eval1()?
                    else {
                        return Err(VimError::new(1346, "Not an interface").into());
                    };
                    if !class.interface
                        || interfaces
                            .iter()
                            .any(|c: &Arc<Class>| Arc::ptr_eq(c, &class))
                    {
                        return Err(VimError::new(1346, "Invalid or duplicate interface").into());
                    }
                    interfaces.push(class);
                }
            }
            _ => return Err(VimError::trailing(keyword.as_bytes()).into()),
        }
    }
    let sid = rt.current_sid();
    let mut class = Class {
        identity: rt.state.next_class_identity()?,
        name: name.into(),
        sid,
        fields: base.as_ref().map_or_else(Vec::new, |b| b.fields.clone()),
        abstract_methods: base
            .as_ref()
            .map_or_else(Default::default, |b| b.abstract_methods.clone()),
        base,
        interfaces,
        methods: BTreeMap::new(),
        static_methods: BTreeMap::new(),
        statics: Dict::new(),
        static_fields: vec![],
        abstract_class,
        interface: kind == b"interface",
        enumeration: kind == b"enum",
    };
    let mut functions = vec![];
    let mut enum_values = vec![];
    let mut at = 0;
    while at < lines.len() {
        let text = lines[at].text.to_str_lossy();
        let mut text = super::source::vim9_comment_prefix(&text).trim();
        let lnum = lines[at].lnum;
        at += 1;
        if text.is_empty() {
            continue;
        }
        let public = text.starts_with("public ");
        if public {
            text = text[7..].trim_start();
        }
        let static_member = text.starts_with("static ");
        if static_member {
            text = text[7..].trim_start();
        }
        let abstract_method = text.starts_with("abstract ");
        if abstract_method {
            text = text[9..].trim_start();
        }
        if let Some(header) = text.strip_prefix("def ") {
            if public || (abstract_method && (!abstract_class || static_member)) {
                return Err(VimError::new(1370, "Invalid method modifier").into());
            }
            let method_name = header.split(['(', '<']).next().unwrap_or("").trim();
            if !method_name.starts_with("new")
                && !matches!(method_name, "empty" | "len" | "string")
                && !method_name
                    .strip_prefix('_')
                    .unwrap_or(method_name)
                    .as_bytes()
                    .first()
                    .is_some_and(u8::is_ascii_uppercase)
            {
                return Err(VimError::new(1267, "Function name must start with a capital").into());
            }
            if method_name.starts_with("__")
                || static_member && method_name.starts_with("new")
                || class.interface && (static_member || method_name.starts_with('_'))
            {
                return Err(
                    VimError::new(1331, "Invalid method name or constructor modifier").into(),
                );
            }
            let mut body = vec![];
            if !class.interface && !abstract_method {
                let end = super::source::definition_end(&lines, at - 1, "enddef")?;
                body = lines[at..end].to_vec();
                at = end + 1;
            }
            let (header, body) = constructor_parameters(header, body, &class.fields, lnum)?;
            let mut function =
                super::vim9::parse_function(rt, host, header.as_bytes(), body, lnum)?;
            if method_name.starts_with("new") {
                if class.abstract_class {
                    return Err(VimError::new(
                        1359,
                        "Cannot define a constructor in an abstract class",
                    )
                    .into());
                }
                if function.signature.as_ref().unwrap().result != Type::Void {
                    return Err(
                        VimError::new(1365, "Constructor cannot declare a return type").into(),
                    );
                }
            }
            if let Some(result) = match method_name {
                "empty" => Some(Type::Bool),
                "len" => Some(Type::Number),
                "string" => Some(Type::String),
                _ => None,
            } {
                if static_member {
                    return Err(
                        VimError::new(1413, "Builtin object method cannot be static").into(),
                    );
                }
                let signature = function.signature.as_ref().unwrap();
                if !signature.parameters.is_empty()
                    || signature.rest.is_some()
                    || !signature.generics.is_empty()
                    || signature.result != result
                {
                    return Err(
                        VimError::new(1383, "Invalid builtin object method signature").into(),
                    );
                }
            }
            let internal: VStr = format!("{}.{method_name}", class.type_name()).into();
            function.name = internal.clone();
            let methods = if static_member {
                &mut class.static_methods
            } else {
                &mut class.methods
            };
            if methods.insert(method_name.into(), internal).is_some() {
                return Err(VimError::new(1355, "Duplicate method").into());
            }
            if abstract_method || class.interface {
                class.abstract_methods.insert(method_name.into());
            } else {
                class.abstract_methods.remove(method_name);
            }
            functions.push(function);
        } else if text.starts_with("var ")
            || text.starts_with("const ")
            || text.starts_with("final ")
        {
            if abstract_method {
                return Err(VimError::new(1370, "Invalid field modifier").into());
            }
            let constant = !text.starts_with("var ");
            let deep_constant = text.starts_with("const ");
            let src = text[text.find(' ').unwrap() + 1..].as_bytes();
            let mut pos = 0;
            while src.get(pos).is_some_and(|b| super::expr::is_name_char(*b)) {
                pos += 1;
            }
            let field_name = String::from_utf8_lossy(&src[..pos]).into_owned();
            if field_name.is_empty() || (public && field_name.starts_with('_')) {
                return Err(VimError::new(1332, "Invalid field name").into());
            }
            let ty = if src.get(pos) == Some(&b':') {
                pos += 1;
                super::types::parse_active(src, &mut pos, rt, host)?
            } else {
                Type::Any
            };
            pos = skip_white(src, pos);
            let initializer = if src.get(pos) == Some(&b'=') {
                Some(VStr::from(&src[skip_white(src, pos + 1)..]))
            } else if pos == src.len() {
                None
            } else {
                return Err(VimError::trailing(&src[pos..]).into());
            };
            if class.interface
                && (initializer.is_some() || public || static_member || field_name.starts_with('_'))
            {
                return Err(VimError::new(1348, "Invalid interface variable").into());
            }
            let fields = if static_member {
                &mut class.static_fields
            } else {
                &mut class.fields
            };
            if fields.iter().any(|f| f.name == field_name) {
                return Err(VimError::new(1369, "Duplicate field").into());
            }
            fields.push(Field {
                sid,
                name: field_name,
                ty,
                initializer,
                public,
                constant,
                deep_constant,
            });
        } else if class.enumeration && functions.is_empty() && class.fields.is_empty() {
            enum_values.push(text.to_string());
        } else {
            return Err(VimError::new(1318, format!("Invalid class declaration: {text}")).into());
        }
    }
    if let Some(base) = &class.base {
        for (name, reference) in &class.methods {
            if name.starts_with("new") {
                continue;
            }
            if let Some(parent) = base
                .method(name)
                .and_then(|n| rt.state.function(n.as_bytes()))
            {
                let actual = functions.iter().find(|f| &f.name == reference).unwrap();
                let a = actual.signature.as_ref().unwrap();
                let b = parent.signature.as_ref().unwrap();
                let (parameters, rest, result) = matching_generics(a, b)?;
                let covariant = a.result.require_in(&result, rt).is_ok() && a.result == result
                    || result.require_in(&a.result, rt).is_ok()
                    || matches!((&a.result, &result), (Type::Named(a), Type::Named(b)) if a == &class.name && base.is_a(b));
                if a.parameters != parameters
                    || a.rest.as_ref().map(|(_, ty)| ty) != rest.as_ref()
                    || actual
                        .params
                        .iter()
                        .map(|p| p.default.is_some())
                        .ne(parent.params.iter().map(|p| p.default.is_some()))
                    || !covariant
                {
                    return Err(VimError::new(
                        1358,
                        format!("Method signature differs from base class: {name}"),
                    )
                    .into());
                }
            } else {
                let other = if name.starts_with('_') {
                    name[1..].to_string()
                } else {
                    format!("_{name}")
                };
                if base.method(&other).is_some() {
                    return Err(VimError::new(1377, "Method access differs from base class").into());
                }
            }
        }
    }
    if !class.abstract_class && !class.interface && !class.abstract_methods.is_empty() {
        return Err(VimError::new(1373, "Abstract method not implemented").into());
    }
    for interface in &class.interfaces {
        for field in &interface.fields {
            if !class
                .fields
                .iter()
                .any(|f| f.name == field.name && f.ty == field.ty)
            {
                return Err(VimError::new(
                    1349,
                    format!(
                        "Interface variable missing or has different type: {}",
                        field.name
                    ),
                )
                .into());
            }
        }
        for method in &interface.abstract_methods {
            let reference = interface.method(method).unwrap();
            let required = rt.state.function(reference.as_bytes()).unwrap();
            let actual = functions
                .iter()
                .find(|f| class.method(method).as_ref() == Some(&f.name))
                .cloned()
                .or_else(|| {
                    class
                        .method(method)
                        .and_then(|name| rt.state.function(name.as_bytes()).map(|f| (*f).clone()))
                });
            let Some(actual) = actual else {
                return Err(
                    VimError::new(1349, format!("Interface method missing: {method}")).into(),
                );
            };
            let a = actual.signature.as_ref().unwrap();
            let b = required.signature.as_ref().unwrap();
            let (parameters, rest, result) = matching_generics(a, b)?;
            if a.parameters != parameters
                || a.result != result
                || a.rest.as_ref().map(|(_, ty)| ty) != rest.as_ref()
            {
                return Err(VimError::new(
                    1383,
                    format!("Interface method type mismatch: {method}"),
                )
                .into());
            }
        }
    }
    let bindings = [(class.name.clone(), Type::Named(class.type_name()))]
        .into_iter()
        .collect();
    for field in class.fields.iter_mut().chain(&mut class.static_fields) {
        field.ty = super::types::substitute(&field.ty, &bindings);
    }
    for function in &mut functions {
        let signature = function.signature.as_mut().unwrap();
        signature.parameters = signature
            .parameters
            .iter()
            .map(|t| super::types::substitute(t, &bindings))
            .collect();
        signature.result = super::types::substitute(&signature.result, &bindings);
        if let Some((_, ty)) = &mut signature.rest {
            *ty = super::types::substitute(ty, &bindings);
        }
    }
    let mut class = Arc::new(class);
    let mut context = super::compiler::Context::for_class(class.clone(), &functions);
    for static_member in [false, true] {
        let count = if static_member {
            class.static_fields.len()
        } else {
            class.fields.len()
        };
        for index in 0..count {
            let field = if static_member {
                &class.static_fields[index]
            } else {
                &class.fields[index]
            };
            let Some(expression) = field.initializer.clone().filter(|_| field.ty == Type::Any)
            else {
                continue;
            };
            let value = Eval::analyze(rt, host, expression.as_bytes(), 0, &context).eval1()?;
            let inferred = Type::infer(&value);
            inferred.require_value()?;
            if inferred == Type::Void {
                return Err(VimError::new(1031, "Cannot use void value").into());
            }
            drop(value);
            context.owner = None;
            let candidate =
                Arc::get_mut(&mut class).expect("unpublished class metadata has one owner");
            let field = if static_member {
                &mut candidate.static_fields[index]
            } else {
                &mut candidate.fields[index]
            };
            field.ty = inferred;
            context.owner = Some(class.clone());
        }
    }
    drop(context);
    rt.state.register_class(class.clone())?;
    for field in class.fields.iter().chain(&class.static_fields) {
        super::types::validate_storage(&field.ty, rt)?;
    }
    rt.state.track_scope(&class.statics)?;
    for mut function in functions {
        function.signature.as_mut().unwrap().owner = Some(class.clone());
        rt.state.define_function(function)?;
    }
    let vars = rt.declaration_scope();
    vars.insert(name, Value::Class(class.clone()));
    vars.data().items.get_mut(name.as_bytes()).unwrap().lock = super::value::Lock::Fixed;
    for field in &class.static_fields {
        let value = if let Some(expression) = &field.initializer {
            rt.with_context(None, sid, |rt| {
                rt.current_frame_mut().unwrap().class_context = Some(class.clone());
                Eval::new(rt, host, expression.as_bytes(), 0, true).eval1()
            })?
        } else {
            field.ty.default_value()
        };
        let ty = if field.ty == Type::Any {
            Type::infer(&value)
        } else {
            field.ty.clone()
        };
        let value = ty.check(value)?;
        ty.attach(&value);
        class.statics.insert(field.name.clone(), value);
        class
            .statics
            .data()
            .declarations
            .insert(field.name.clone().into(), ty);
        if field.deep_constant {
            super::let_cmd::lock_value(
                &class.statics.get(field.name.as_bytes()).unwrap(),
                i64::MAX,
                true,
            )?;
        }
    }
    if class.enumeration {
        let src = enum_values.join("\n");
        let src = src.as_bytes();
        let mut pos = 0;
        let mut values = vec![];
        while pos < src.len() {
            pos = skip_white(src, pos);
            let start = pos;
            while src.get(pos).is_some_and(|b| super::expr::is_name_char(*b)) {
                pos += 1;
            }
            if start == pos {
                return Err(VimError::new(1418, "Invalid enum value").into());
            }
            let name = String::from_utf8_lossy(&src[start..pos]).into_owned();
            if class.statics.contains(name.as_bytes()) {
                return Err(VimError::new(1418, "Duplicate enum value").into());
            }
            pos = skip_white(src, pos);
            let mut constructor = constructor_reference(rt, &class, "new");
            if src.get(pos) == Some(&b'<') {
                let mut eval = Eval::new(rt, host, src, pos, true);
                let types = eval.type_arguments()?;
                pos = skip_white(src, eval.pos);
                constructor = super::vim9::specialize(rt, &constructor, types)?;
            }
            let mut args = vec![];
            if src.get(pos) == Some(&b'(') {
                pos += 1;
                while src.get(skip_white(src, pos)) != Some(&b')') {
                    let mut eval = Eval::new(rt, host, src, skip_white(src, pos), true);
                    args.push(eval.eval1()?);
                    pos = skip_white(src, eval.pos);
                    if src.get(pos) != Some(&b',') {
                        break;
                    }
                    pos += 1;
                }
                pos = skip_white(src, pos);
                if src.get(pos) != Some(&b')') {
                    return Err(VimError::new(110, "Missing )").into());
                }
                pos += 1;
            }
            let value = rt.call_funcref(host, &constructor, args, None)?;
            if let Value::Object(object) = &value {
                object.fields.insert("name", Value::str(&name));
                object
                    .fields
                    .insert("ordinal", Value::Number(values.len() as i64));
            }
            class.statics.insert(name, value.clone());
            values.push(value);
            pos = skip_white(src, pos);
            if pos == src.len() {
                break;
            }
            if src[pos] != b',' {
                return Err(VimError::new(1418, "Missing comma in enum").into());
            }
            pos += 1;
        }
        let values = Value::list(values);
        super::let_cmd::lock_value(&values, 1, true)?;
        class.statics.insert("values", values);
    }
    Ok(())
}

/// Generic parameter names are local to each method. Compare corresponding
/// positions, including nested container/function types and variadic types.
fn matching_generics(
    actual: &super::vim9::Signature,
    required: &super::vim9::Signature,
) -> EvalResult<(Vec<Type>, Option<Type>, Type)> {
    if actual.generics.len() != required.generics.len() {
        let code = if actual.generics.is_empty() {
            1432
        } else if required.generics.is_empty() {
            1433
        } else {
            1434
        };
        return Err(VimError::new(
            code,
            "Generic method override has different type parameters",
        )
        .into());
    }
    let bindings = required
        .generics
        .iter()
        .cloned()
        .zip(actual.generics.iter().cloned().map(Type::Named))
        .collect();
    Ok((
        required
            .parameters
            .iter()
            .map(|t| super::types::substitute(t, &bindings))
            .collect(),
        required
            .rest
            .as_ref()
            .map(|(_, t)| super::types::substitute(t, &bindings)),
        super::types::substitute(&required.result, &bindings),
    ))
}

fn constructor_parameters(
    header: &str,
    mut body: Vec<super::state::SourceLine>,
    fields: &[Field],
    lnum: usize,
) -> EvalResult<(String, Vec<super::state::SourceLine>)> {
    if !header.starts_with("new(") && !header.starts_with("new") {
        return Ok((header.into(), body));
    }
    let mut header = header.to_string();
    let mut assignments = vec![];
    while let Some(start) = header.find("this.") {
        let end = header[start + 5..]
            .find(|c: char| !c.is_ascii_alphanumeric() && c != '_')
            .map_or(header.len(), |i| start + 5 + i);
        let name = header[start + 5..end].to_string();
        let field = fields
            .iter()
            .find(|f| f.name == name)
            .ok_or_else(|| VimError::new(1326, format!("Unknown constructor field: {name}")))?;
        let optional = header[end..].trim_start().starts_with("= v:none");
        header.replace_range(
            start..end,
            &format!(
                "{name}: {}",
                if optional {
                    Type::Any
                } else {
                    field.ty.clone()
                }
            ),
        );
        if optional {
            assignments.push(super::state::SourceLine {
                text: format!("if {name} isnot v:none").into(),
                lnum,
            });
        }
        assignments.push(super::state::SourceLine {
            text: format!("this.{name} = {name}").into(),
            lnum,
        });
        if optional {
            assignments.push(super::state::SourceLine {
                text: "endif".into(),
                lnum,
            });
        }
    }
    assignments.append(&mut body);
    Ok((header, assignments))
}
