//! One builtin signature registry for Vim9 analysis and runtime validation.
//! The committed table is extracted from pinned upstream metadata; builds are
//! offline and do not require an installed Vim. Legacy execution keeps its own
//! coercion rules, while both dialects share names and method argument order.
use crate::vimscript::{types::Type, Runtime, Value, VimError};

struct Signature {
    name: &'static str,
    min: usize,
    max: usize,
    method: Option<usize>,
    meta: bool,
    args: &'static [Rule],
    result: Return,
}

include!("signatures_data.rs");

fn find(name: &str) -> Option<&'static Signature> {
    SIGNATURES
        .binary_search_by_key(&name, |signature| signature.name)
        .ok()
        .map(|i| &SIGNATURES[i])
}

pub(super) fn contains(name: &str) -> bool {
    find(name).is_some()
}

pub(super) fn method(name: &str) -> Result<usize, VimError> {
    match find(name) {
        Some(signature) => signature
            .method
            .ok_or_else(|| VimError::new(276, format!("Cannot use function as a method: {name}"))),
        None => Ok(0), // User functions receive their base as the first argument.
    }
}

/// Check arguments without running callbacks or modifying a value. Unknown
/// static types defer their checks to this same routine at the runtime call.
pub(crate) fn check(
    rt: &Runtime,
    name: &str,
    args: &[Value],
    analysis: bool,
) -> Result<Type, VimError> {
    let Some(signature) = find(name) else {
        return Ok(Type::Any);
    };
    if args.len() < signature.min {
        return Err(VimError::argument(119, name));
    }
    if args.len() > signature.max {
        return Err(VimError::argument(118, name));
    }
    let types: Vec<_> = args.iter().map(Type::infer).collect();
    for (index, actual) in types.iter().enumerate() {
        if *actual == Type::Void {
            return Err(VimError::new(1031, "Cannot use void value"));
        }
        if !signature.meta {
            actual.require_value()?;
        }
        let rule = signature.args.get(index).or_else(|| {
            (signature.max == usize::MAX)
                .then(|| signature.args.last())
                .flatten()
        });
        if let Some(rule) = rule {
            rule.check(rt, args, &types, index, analysis)?;
        }
    }
    Ok(signature.result.resolve(&types))
}

fn member(ty: &Type) -> Type {
    match ty {
        Type::List(t) | Type::Dict(t) => *t.clone(),
        Type::String => Type::String,
        Type::Blob => Type::Number,
        _ => Type::Any,
    }
}

impl Rule {
    fn check(
        self,
        rt: &Runtime,
        args: &[Value],
        types: &[Type],
        index: usize,
        analysis: bool,
    ) -> Result<(), VimError> {
        use Rule::*;
        use Type::*;
        let actual = &types[index];
        if *actual == Any {
            return Ok(());
        }
        let list = || List(Box::new(Any));
        let dict = || Dict(Box::new(Any));
        let tuple = || Tuple(vec![], Some(Box::new(Any)));
        let function = || Func {
            args: vec![],
            optional: 0,
            rest: Some(Box::new(Any)),
            result: Box::new(Any),
        };
        let prev = index
            .checked_sub(1)
            .and_then(|i| types.get(i))
            .unwrap_or(&Any);
        let choices = match self {
            ArgAny => return Ok(()),
            ArgNumber => vec![Number],
            ArgString => vec![String],
            ArgBlob => vec![Blob],
            ArgBool => {
                if let Value::Number(n) = args[index] {
                    if n != 0 && n != 1 {
                        return Err(VimError::new(
                            1023,
                            "Using a Number as a Bool: expected 0 or 1",
                        ));
                    }
                }
                vec![Bool, Number]
            }
            ArgBoolOrNr => vec![Bool, Number],
            ArgFloatOrNr => vec![Float, Number],
            ArgJob => vec![Job],
            ArgChanOrJob => vec![Channel, Job],
            ArgObject => {
                if matches!(actual, Named(_)) {
                    return Ok(());
                }
                vec![Named("object".into())]
            }
            VarargsClass => {
                if matches!(&args[index], Value::Class(_)) {
                    return Ok(());
                }
                if let Value::TypeAlias(alias) = &args[index] {
                    if let Named(name) = &alias.ty {
                        if super::super::types::class_for_type(rt, name).is_some() {
                            return Ok(());
                        }
                    }
                }
                return Err(VimError::new(
                    693,
                    format!("Class or typealias required for argument {}", index + 1),
                ));
            }
            ArgListAny | ArgListAnyMod => vec![list()],
            ArgDictAny => vec![dict()],
            ArgTupleAny => vec![tuple()],
            ArgListNumber => vec![List(Box::new(Number))],
            ArgListString => vec![List(Box::new(String))],
            ArgListOrBlob | ArgListOrBlobMod => vec![list(), Blob],
            ArgListOrTuple => vec![list(), tuple()],
            ArgListOrTupleOrBlob => vec![list(), tuple(), Blob],
            ArgListOrDict | ArgListOrDictMod => vec![list(), dict()],
            ArgListOrTupleOrDict => vec![list(), tuple(), dict()],
            ArgListOrDictOrBlobMod => vec![list(), dict(), Blob],
            ArgListOrDictOrBlobOrString | ArgListOrDictOrBlobOrStringMod => {
                vec![list(), dict(), Blob, String]
            }
            ArgListTupleDictBlobOrString => vec![list(), tuple(), dict(), Blob, String],
            ArgStringListTupleOrBlob | ArgSlice1 | ArgReverse => {
                vec![String, list(), tuple(), Blob]
            }
            ArgStringListTupleOrDict => vec![String, list(), tuple(), dict()],
            ArgRepeat1 => vec![String, Number, list(), tuple(), Blob],
            ArgLen1 => {
                if matches!(actual, Named(_)) {
                    return Ok(());
                }
                vec![String, Number, Blob, list(), tuple(), dict()]
            }
            ArgGet1 => vec![Blob, list(), tuple(), dict(), function()],
            ArgBuffer | ArgLnum | ArgStringOrNr => vec![String, Number],
            ArgBufferOrDictAny => vec![String, Number, dict()],
            ArgCursor1 | ArgStrOrNrOrList => vec![String, Number, list()],
            ArgStringOrListAny => vec![String, list()],
            ArgStringOrListString => vec![String, List(Box::new(String))],
            ArgStringOrDictAny | ArgDictAnyOrString => vec![String, dict()],
            ArgStringOrBlob => vec![String, Blob],
            ArgStringOrFunc => vec![String, function(), Bool, Number],
            ArgItemOfPrev => match prev {
                List(t) => vec![*t.clone()],
                Blob => vec![Number],
                _ => return Ok(()),
            },
            ArgSameAsPrev => vec![prev.clone()],
            ArgSameStructAsPrev => match prev {
                List(_) => vec![list()],
                Dict(_) => vec![dict()],
                _ => vec![prev.clone()],
            },
            ArgExtend3 => match types.first() {
                Some(List(_)) => vec![Number],
                Some(Dict(_)) => vec![String],
                _ => return Ok(()),
            },
            ArgRemove2 => match prev {
                List(_) | Blob => vec![Number],
                Dict(_) => vec![String, Number],
                _ => return Ok(()),
            },
            ArgFilterFunc | ArgMapFunc | ArgForeachFunc | ArgSortHow => {
                if let Func {
                    args: parameters,
                    rest,
                    result,
                    ..
                } = actual
                {
                    if parameters.is_empty() && rest.as_deref() == Some(&Any) {
                        return Ok(());
                    }
                    let first = types.first().unwrap_or(&Any);
                    let item = member(first);
                    let key = if matches!(first, Dict(_)) {
                        String
                    } else {
                        Number
                    };
                    let call_args = if matches!(self, ArgSortHow) {
                        vec![
                            Value::type_value(item.clone()),
                            Value::type_value(item.clone()),
                        ]
                    } else {
                        vec![Value::type_value(key), Value::type_value(item.clone())]
                    };
                    super::super::compiler::invoke(rt, actual, &call_args).map_err(|failure| {
                        match failure {
                            super::super::Fail::Error(error) => error,
                            _ => VimError::new(1012, "Invalid callback type"),
                        }
                    })?;
                    let expected = match self {
                        ArgFilterFunc => Bool,
                        ArgMapFunc => item,
                        ArgSortHow => Number,
                        _ => Any,
                    };
                    if !matches!(self, ArgForeachFunc) {
                        expected.require_in(result, rt)?;
                    }
                    return Ok(());
                }
                vec![String, function()]
            }
        };
        let accepts = |expected: &Type| {
            // Builtin scalar arguments do not have assignment's numeric casts.
            match (expected, actual) {
                (Any, _) => true,
                (Tuple(_, _), Tuple(_, _)) => true,
                (List(_) | Dict(_) | Func { .. }, _) => expected.accepts(actual),
                _ => expected == actual,
            }
        };
        if choices.iter().any(accepts) {
            return Ok(());
        }
        if !analysis {
            if matches!(self, ArgItemOfPrev | ArgSameAsPrev) {
                return choices[0].require(actual);
            }
            if matches!(self, ArgString) {
                return Err(VimError::new(
                    1174,
                    format!("String required for argument {}", index + 1),
                ));
            }
            if matches!(self, ArgNumber) {
                return Err(VimError::new(
                    1210,
                    format!("Number required for argument {}", index + 1),
                ));
            }
        }
        let expected = choices
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join(" or ");
        Err(VimError::new(
            1013,
            format!(
                "Argument {}: type mismatch, expected {expected} but got {actual}",
                index + 1
            ),
        ))
    }
}

impl Return {
    fn resolve(self, args: &[Type]) -> Type {
        use Type::*;
        let first = args.first().cloned().unwrap_or(Any);
        let list = |t| List(Box::new(t));
        let dict = |t| Dict(Box::new(t));
        match self {
            Self::Void => Void,
            Self::Any => Any,
            Self::Bool => Bool,
            Self::Number | Self::NumberBool => Number,
            Self::Float => Float,
            Self::String => String,
            Self::Blob => Blob,
            Self::Job => Job,
            Self::Channel => Channel,
            Self::ListAny => list(Any),
            Self::ListString => list(String),
            Self::ListNumber => list(Number),
            Self::ListDictAny => list(dict(Any)),
            Self::ListItems => list(list(Any)),
            Self::ListStringItems => list(list(String)),
            Self::ListRegionpos => list(list(list(Number))),
            Self::DictAny => dict(Any),
            Self::DictNumber => dict(Number),
            Self::DictString => dict(String),
            Self::TupleAny => Tuple(vec![], Some(Box::new(Any))),
            Self::FuncAny | Self::FuncUnknown => Func {
                args: vec![],
                optional: 0,
                rest: Some(Box::new(Any)),
                result: Box::new(Any),
            },
            Self::FirstArg | Self::Slice => first,
            Self::Copy => match first {
                List(_) => list(Any),
                Dict(_) => dict(Any),
                _ => first,
            },
            Self::FirstCont => match first {
                List(_) => list(Any),
                Dict(_) => dict(Any),
                Blob => Blob,
                _ => Any,
            },
            Self::Extend => {
                if args.get(1) == Some(&first) {
                    first
                } else {
                    match first {
                        List(_) => list(Any),
                        Dict(_) => dict(Any),
                        _ => first,
                    }
                }
            }
            Self::Repeat => {
                if first == Number {
                    String
                } else {
                    first
                }
            }
            Self::Getline => {
                if args.len() == 1 {
                    String
                } else {
                    list(String)
                }
            }
            Self::Getreg => {
                if args.len() == 3 {
                    list(String)
                } else {
                    String
                }
            }
            Self::Finddir => {
                if args.len() < 3 {
                    String
                } else {
                    Any
                }
            }
            Self::Argv => {
                if args.is_empty() {
                    list(String)
                } else {
                    Any
                }
            }
            Self::ListMember => list(member(&first)),
            Self::MaxMin => member(&first),
            Self::ListOrDict0 => {
                if args.is_empty() {
                    list(dict(Any))
                } else {
                    dict(Any)
                }
            }
            Self::ListOrDict1 => {
                if args.len() == 1 {
                    list(dict(Any))
                } else {
                    dict(Any)
                }
            }
            Self::JobInfo => {
                if args.is_empty() {
                    list(Job)
                } else {
                    dict(Any)
                }
            }
            Self::Remove => {
                if args.len() == 3 && matches!(first, List(_) | Dict(_)) {
                    first
                } else {
                    member(&first)
                }
            }
            Self::Virtcol => {
                if args.len() > 1 {
                    list(Number)
                } else {
                    Number
                }
            }
            Self::Maparg => {
                if args.len() == 4 {
                    dict(Any)
                } else {
                    String
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_implemented_builtin_has_one_upstream_signature() {
        for names in [
            super::super::text::NAMES,
            super::super::collections::NAMES,
            super::super::script::NAMES,
            super::super::system::NAMES,
        ] {
            for name in names {
                assert!(contains(name), "missing builtin metadata: {name}");
            }
        }
        for pair in SIGNATURES.windows(2) {
            assert!(pair[0].name < pair[1].name);
        }
    }
}
