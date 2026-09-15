//! Bounded function setup. Only local values and emitted declarations are effects.
use super::{
    identifier, BTreeMap, Parser, Setup, Value, MAX_BINDINGS, MAX_DEPTH, MAX_EXPRESSION_BYTES,
    MAX_VALUE_BYTES,
};

pub(super) enum Statement {
    Return(String),
    Bind(String),
    Emit(String),
    Call(String),
    If(String, Vec<Statement>, Vec<Statement>),
    For(String, String, Vec<Statement>),
    While(String, Vec<Statement>),
    Break,
    Continue,
    Unsupported(String),
}

enum Flow {
    Next,
    Return(Value),
    Break,
    Continue,
}

pub(super) fn parse(
    signature: &str,
    body: &[String],
) -> Result<(String, Vec<String>, bool, Vec<Statement>), String> {
    if body.len() > 1024 || body.iter().map(String::len).sum::<usize>() > MAX_EXPRESSION_BYTES {
        return Err("syntax setup function source budget exceeded".into());
    }
    let (name, signature) = signature
        .split_once('(')
        .ok_or("invalid setup function signature")?;
    let name = name.trim();
    if !identifier(name)
        || !(name.starts_with("s:")
            || (!name.contains(':')
                && (name.as_bytes()[0].is_ascii_uppercase() || name.contains('#'))))
    {
        return Err("invalid setup function name".into());
    }
    let (arguments, attributes) = signature
        .split_once(')')
        .ok_or("invalid setup function signature")?;
    if !matches!(attributes.trim(), "" | "abort") {
        return Err("unsupported setup function attributes".into());
    }
    let mut arguments: Vec<String> = arguments
        .split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
        .collect();
    let variadic = arguments.last().is_some_and(|s| s == "...");
    if variadic {
        arguments.pop();
    }
    if arguments.len() > 16 || arguments.iter().any(|s| !identifier(s) || s.contains(':')) {
        return Err("invalid setup function parameters".into());
    }
    let mut at = 0;
    let statements = block(body, &mut at, 0)?;
    if at != body.len() {
        return Err("unmatched setup function control flow".into());
    }
    Ok((name.into(), arguments, variadic, statements))
}

fn block(body: &[String], at: &mut usize, depth: usize) -> Result<Vec<Statement>, String> {
    if depth >= MAX_DEPTH {
        return Err("syntax setup function depth budget exceeded".into());
    }
    let mut statements = Vec::new();
    while let Some(line) = body.get(*at) {
        if matches!(line.as_str(), "else" | "endif" | "endfor" | "endwhile")
            || line.starts_with("elseif ")
        {
            break;
        }
        *at += 1;
        let (command, rest) = line
            .split_once(char::is_whitespace)
            .map_or((line.as_str(), ""), |(command, rest)| {
                (command, rest.trim())
            });
        statements.push(match command {
            "return" | "retu" => Statement::Return(if rest.is_empty() { "0" } else { rest }.into()),
            "let" => Statement::Bind(rest.into()),
            "execute" | "exec" | "exe" => Statement::Emit(rest.into()),
            "call" | "cal" => Statement::Call(rest.into()),
            "if" => conditional(rest.into(), body, at, depth + 1)?,
            "for" => {
                let (name, values) = rest
                    .split_once(" in ")
                    .ok_or("invalid setup function for loop")?;
                let statements = block(body, at, depth + 1)?;
                if body.get(*at).map(String::as_str) != Some("endfor") {
                    return Err("unterminated setup function for loop".into());
                }
                *at += 1;
                Statement::For(name.trim().into(), values.trim().into(), statements)
            }
            "while" => {
                let statements = block(body, at, depth + 1)?;
                if body.get(*at).map(String::as_str) != Some("endwhile") {
                    return Err("unterminated setup function while loop".into());
                }
                *at += 1;
                Statement::While(rest.into(), statements)
            }
            "break" => Statement::Break,
            "continue" => Statement::Continue,
            _ => Statement::Unsupported(line.clone()),
        });
    }
    Ok(statements)
}

fn conditional(
    condition: String,
    body: &[String],
    at: &mut usize,
    depth: usize,
) -> Result<Statement, String> {
    let yes = block(body, at, depth)?;
    let Some(line) = body.get(*at) else {
        return Err("unterminated setup function conditional".into());
    };
    *at += 1;
    let no = if let Some(expression) = line.strip_prefix("elseif ") {
        vec![conditional(expression.into(), body, at, depth + 1)?]
    } else if line == "else" {
        let no = block(body, at, depth)?;
        if body.get(*at).map(String::as_str) != Some("endif") {
            return Err("unterminated setup function conditional".into());
        }
        *at += 1;
        no
    } else if line == "endif" {
        Vec::new()
    } else {
        return Err("invalid setup function conditional".into());
    };
    Ok(Statement::If(condition, yes, no))
}

pub(super) fn run(
    setup: &mut Setup<'_>,
    body: &[Statement],
    mut arguments: BTreeMap<String, Value>,
    depth: usize,
) -> Result<Value, String> {
    match run_block(setup, body, &mut arguments, depth)? {
        Flow::Return(value) => Ok(value),
        Flow::Next => Ok(Value::Number(0)),
        _ => Err("setup function loop control outside loop".into()),
    }
}

fn evaluate(
    setup: &mut Setup<'_>,
    source: &str,
    arguments: &BTreeMap<String, Value>,
    depth: usize,
) -> Result<Value, String> {
    setup.charge(source.len())?;
    setup.eval(&Parser::parse(source)?, arguments, depth + 1)
}

fn run_block(
    setup: &mut Setup<'_>,
    body: &[Statement],
    arguments: &mut BTreeMap<String, Value>,
    depth: usize,
) -> Result<Flow, String> {
    if depth >= MAX_DEPTH {
        return Err("syntax setup function depth budget exceeded".into());
    }
    for statement in body {
        setup.charge(1)?;
        match statement {
            Statement::Return(source) => {
                return Ok(Flow::Return(evaluate(setup, source, arguments, depth)?))
            }
            Statement::Bind(source) => {
                let (target, source) = source
                    .split_once('=')
                    .ok_or("invalid setup local assignment")?;
                let mut value = evaluate(setup, source.trim(), arguments, depth)?;
                let target = target.trim();
                let (target, operator) = ["..", ".", "+", "-", "*", "/", "%"]
                    .into_iter()
                    .find_map(|operator| {
                        target
                            .strip_suffix(operator)
                            .map(|target| (target.trim(), Some(operator)))
                    })
                    .unwrap_or((target, None));
                if let Some(operator) = operator {
                    value = setup.binary(operator, setup.variable(target, arguments)?, value)?;
                }
                bind(arguments, target, value)?;
            }
            Statement::Emit(source) => {
                if setup.emitted.is_none() {
                    return Err("syntax-generating setup functions require a call statement".into());
                }
                setup.charge(source.len())?;
                let values =
                    setup.eval_values(&Parser::parse_many(source)?, arguments, depth + 1)?;
                let command = values
                    .iter()
                    .map(Value::text)
                    .collect::<Result<Vec<_>, _>>()?
                    .join(" ");
                let emitted = setup.emitted.as_mut().unwrap();
                if emitted.len() >= MAX_BINDINGS
                    || emitted
                        .iter()
                        .map(String::len)
                        .sum::<usize>()
                        .saturating_add(command.len())
                        > MAX_VALUE_BYTES
                {
                    return Err("setup generated-command budget exceeded".into());
                }
                emitted.push(command);
            }
            Statement::Call(source) => {
                evaluate(setup, source, arguments, depth)?;
            }
            Statement::If(source, yes, no) => {
                let branch = if evaluate(setup, source, arguments, depth)?.truth()? {
                    yes
                } else {
                    no
                };
                let flow = run_block(setup, branch, arguments, depth + 1)?;
                if !matches!(flow, Flow::Next) {
                    return Ok(flow);
                }
            }
            Statement::For(name, source, body) => {
                let Value::List(values) = evaluate(setup, source, arguments, depth)? else {
                    return Err("setup function for loop requires a list".into());
                };
                if values.len() > MAX_BINDINGS {
                    return Err("setup function loop item budget exceeded".into());
                }
                for value in values {
                    setup.charge(1)?;
                    bind(arguments, name, value)?;
                    match run_block(setup, body, arguments, depth + 1)? {
                        Flow::Return(value) => return Ok(Flow::Return(value)),
                        Flow::Break => break,
                        _ => {}
                    }
                }
            }
            Statement::While(source, body) => {
                let mut iterations = 0;
                while evaluate(setup, source, arguments, depth)?.truth()? {
                    iterations += 1;
                    if iterations > MAX_BINDINGS {
                        return Err("setup function loop iteration budget exceeded".into());
                    }
                    match run_block(setup, body, arguments, depth + 1)? {
                        Flow::Return(value) => return Ok(Flow::Return(value)),
                        Flow::Break => break,
                        _ => {}
                    }
                }
            }
            Statement::Break => return Ok(Flow::Break),
            Statement::Continue => return Ok(Flow::Continue),
            Statement::Unsupported(command) => {
                return Err(format!(
                    "unsupported active setup function command: {command}"
                ))
            }
        }
        let bytes = arguments
            .iter()
            .map(|(key, value)| key.len() + value.bytes())
            .sum::<usize>();
        if bytes > MAX_VALUE_BYTES {
            return Err("setup function local byte budget exceeded".into());
        }
        setup.charge(bytes)?;
    }
    Ok(Flow::Next)
}

fn bind(arguments: &mut BTreeMap<String, Value>, name: &str, value: Value) -> Result<(), String> {
    if let Some(targets) = name
        .strip_prefix('[')
        .and_then(|name| name.strip_suffix(']'))
    {
        let Value::List(values) = value else {
            return Err("setup destructuring requires a list".into());
        };
        let (targets, tail) = targets
            .split_once(';')
            .map_or((targets, None), |(targets, tail)| {
                (targets, Some(tail.trim()))
            });
        let targets: Vec<_> = targets.split(',').map(str::trim).collect();
        if values.len() < targets.len() || tail.is_none() && values.len() != targets.len() {
            return Err("setup destructuring list length mismatch".into());
        }
        for (target, value) in targets.iter().zip(values.iter()) {
            bind(arguments, target, value.clone())?;
        }
        if let Some(tail) = tail {
            bind(
                arguments,
                tail,
                Value::List(values[targets.len()..].to_vec()),
            )?;
        }
        return Ok(());
    }
    let name = name.strip_prefix("l:").unwrap_or(name);
    if !identifier(name) || name.contains(':') {
        return Err("setup functions can assign only local variables".into());
    }
    if arguments.len() >= MAX_BINDINGS && !arguments.contains_key(&format!("l:{name}")) {
        return Err("setup function local binding budget exceeded".into());
    }
    arguments.insert(format!("l:{name}"), value);
    Ok(())
}
