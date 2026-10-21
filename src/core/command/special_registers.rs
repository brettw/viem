//! Special register sources and the nested expression-register prompt.
use super::*;
use crate::vimscript::{
    EvalResult, Fail, HostCommand, Message, OptionScope, Runtime, ScriptHost, Value, VimError,
};

#[derive(Clone, Debug)]
pub(super) struct ExpressionPrompt {
    outer: Option<CommandLineState>,
    mode: Mode,
    count: Option<usize>,
    pending: Pending,
    requested: Option<char>,
    insertion: Option<command_line_register::RegisterInsertion>,
}

impl CommandInterpreter {
    /// Native file history is view-local. Capture it when a pane's binding changes.
    pub(crate) fn set_current_artifact(&mut self, path: Option<crate::document::ArtifactPath>) {
        self.current_artifact = path;
    }

    pub fn set_alternate_artifact(&mut self, path: Option<crate::document::ArtifactPath>) {
        self.alternate_artifact = path;
    }

    /// An explicit native drop supplies its exact text; reading `~` never reads
    /// the live clipboard or manufactures a drop from another input operation.
    pub fn set_drop_register(&mut self, value: RegisterValue) {
        self.drop_register = Some(value);
    }

    pub(super) fn read_special_register(
        &self,
        document: &Document,
        name: char,
    ) -> Option<Result<Option<RegisterValue>, RegisterReadError>> {
        let literal = |text: &str| {
            RegisterValue::try_new(text, RegisterKind::Characterwise, Vec::new())
                .expect("special registers contain literal Unicode text")
        };
        Some(match name {
            '/' => Ok(self.last_search.as_ref().map(|(_, text)| literal(text))),
            ':' => Ok(self.last_command_line.as_deref().map(literal)),
            '~' => Ok(self.drop_register.clone()),
            '#' => self
                .alternate_artifact
                .as_ref()
                .ok_or(RegisterReadError::NoAlternateArtifact)
                .and_then(|path| {
                    std::str::from_utf8(path.as_bytes())
                        .map(literal)
                        .map_err(|_| RegisterReadError::ArtifactPathIsNotUtf8)
                })
                .map(Some),
            '=' if self.expression_input.is_some() => Ok(self.expression_input.clone()),
            '=' => self
                .last_expression
                .as_deref()
                .map(|expression| self.evaluate_register_expression(document, expression))
                .transpose(),
            _ => return None,
        })
    }

    fn evaluate_register_expression(
        &self,
        document: &Document,
        expression: &str,
    ) -> Result<RegisterValue, RegisterReadError> {
        let mut runtime = Runtime::new(self.script.clone());
        let mut host = ExpressionHost {
            commands: self,
            document,
        };
        let value = runtime
            .eval_expression(&mut host, expression.as_bytes(), 0)
            .map_err(|error| {
                RegisterReadError::Expression(match error {
                    Fail::Error(error) => error.text(),
                    other => format!("{other:?}"),
                })
            })?;
        let convert = |value: &Value| {
            let bytes = match value {
                Value::Dict(_) => value.string_bytes(),
                _ => value
                    .to_bytes()
                    .map_err(|error| RegisterReadError::Expression(error.text()))?,
            };
            String::from_utf8(bytes).map_err(|_| {
                RegisterReadError::Expression("Expression result is not valid UTF-8".into())
            })
        };
        match value {
            Value::List(list) => {
                let lines = list
                    .snapshot()
                    .iter()
                    .map(convert)
                    .collect::<Result<Vec<_>, _>>()?;
                Ok(if lines.is_empty() {
                    RegisterValue::try_new("", RegisterKind::Linewise, Vec::new()).unwrap()
                } else {
                    RegisterValue::linewise(lines.join("\n") + "\n")
                })
            }
            value => {
                let text = convert(&value)?;
                Ok(if text.ends_with('\n') {
                    RegisterValue::linewise(text)
                } else {
                    RegisterValue::characterwise(text)
                })
            }
        }
    }

    pub(super) fn begin_expression_register(
        &mut self,
        insertion: Option<command_line_register::RegisterInsertion>,
    ) -> CommandOutput {
        if self.expression_prompt.is_some() {
            return CommandOutput::unsupported(
                "E129: Cannot use the expression register recursively",
            );
        }
        let saved = ExpressionPrompt {
            outer: self.command_line_state.take(),
            mode: self.mode,
            count: self.count,
            pending: self.pending,
            requested: self.requested_register,
            insertion,
        };
        self.enter_command_line(CommandLineKind::Expression);
        self.expression_prompt = Some(Box::new(saved));
        CommandOutput {
            mode_changed: true,
            ..CommandOutput::pending()
        }
    }

    fn restore_expression_prompt(&mut self) -> Option<ExpressionPrompt> {
        let saved = *self.expression_prompt.take()?;
        self.mode = saved.mode;
        self.count = saved.count;
        self.pending = saved.pending;
        self.requested_register = saved.requested;
        self.command_line_state = saved.outer.clone();
        Some(saved)
    }

    pub(super) fn cancel_expression_register(&mut self) -> CommandOutput {
        let saved = self.restore_expression_prompt();
        if saved.is_some_and(|saved| {
            saved.outer.is_none() && !matches!(saved.mode, Mode::Insert | Mode::Replace)
        }) {
            self.clear_pending();
        }
        self.register_pending = false;
        CommandOutput {
            mode_changed: true,
            status: CommandStatus::Cancelled,
            ..CommandOutput::complete()
        }
    }

    pub(super) fn submit_expression_register(
        &mut self,
        document: &mut Document,
        input: String,
    ) -> Result<CommandOutput, DocumentError> {
        let Some(saved) = self.restore_expression_prompt() else {
            return Ok(CommandOutput::unsupported("missing expression prompt"));
        };
        let expression = if input.is_empty() {
            self.last_expression.clone().unwrap_or_default()
        } else {
            input
        };
        let value = match self.evaluate_register_expression(document, &expression) {
            Ok(value) => value,
            Err(error) => {
                if saved.outer.is_none() && !matches!(saved.mode, Mode::Insert | Mode::Replace) {
                    self.clear_pending();
                }
                self.register_pending = false;
                return Ok(CommandOutput {
                    mode_changed: true,
                    status: CommandStatus::Error(error.to_string()),
                    ..CommandOutput::complete()
                });
            }
        };
        self.last_expression = Some(expression.clone());
        push_history(
            &mut self.expression_history,
            expression,
            self.vim_options.number("history") as usize,
        );
        let mut output = if let Some(insertion) = saved.insertion {
            self.insert_command_line_register(document, &value, insertion, 0)
        } else if matches!(self.mode, Mode::Insert | Mode::Replace) {
            // Use the same insertion transaction and undo boundary as Ctrl-R.
            self.register_pending = true;
            self.expression_input = Some(value);
            let result = self.handle_edit_mode_key(document, Key::Char('='));
            self.expression_input = None;
            result?
        } else {
            self.expression_input = Some(value);
            self.requested_register = Some('=');
            CommandOutput::pending()
        };
        output.mode_changed = true;
        Ok(output)
    }
}

/// Expressions share the script evaluator and query profile. Text, windows,
/// options, registers and native effects cannot mutate during a register read.
struct ExpressionHost<'a> {
    commands: &'a CommandInterpreter,
    document: &'a Document,
}
impl ScriptHost for ExpressionHost<'_> {
    fn execute(&mut self, _: &mut Runtime, _: &HostCommand) -> EvalResult<()> {
        Err(VimError::new(565, "Not allowed to change text or change window").into())
    }
    fn get_option(&mut self, name: &str, _: OptionScope) -> Result<Value, VimError> {
        self.commands.script_option(self.document, name)
    }
    fn set_option(
        &mut self,
        _: &mut Runtime,
        _: &str,
        _: OptionScope,
        _: &Value,
    ) -> EvalResult<()> {
        Err(VimError::new(
            565,
            "Not allowed to change options while reading a register",
        )
        .into())
    }
    fn option_exists(&self, name: &str, working: bool) -> bool {
        options::option_exists(name, working)
    }
    fn get_register(&mut self, name: char) -> Result<Value, VimError> {
        if name == '=' {
            return Err(VimError::new(
                129,
                "Cannot use the expression register recursively",
            ));
        }
        self.commands.clone().script_register(self.document, name)
    }
    fn set_register(&mut self, _: char, _: &Value, _: bool) -> Result<(), VimError> {
        Err(VimError::new(
            565,
            "Not allowed to change registers while reading a register",
        ))
    }
    fn call_function(
        &mut self,
        _: &mut Runtime,
        name: &str,
        args: &[Value],
    ) -> Option<EvalResult<Value>> {
        if matches!(
            name,
            "setreg" | "histadd" | "histdel" | "setbufvar" | "setwinvar" | "settabvar"
        ) {
            return Some(Err(VimError::new(
                565,
                "Not allowed to change editor state while reading a register",
            )
            .into()));
        }
        if matches!(name, "getreg" | "getregtype")
            && args
                .first()
                .is_some_and(|value| value.to_text().is_ok_and(|text| text.starts_with('=')))
        {
            return Some(Err(VimError::new(
                129,
                "Cannot use the expression register recursively",
            )
            .into()));
        }
        self.commands.clone().script_function(
            self.document,
            self.buffer_name().as_deref(),
            name,
            args,
        )
    }
    fn has_function(&self, name: &str) -> bool {
        scripting::SCRIPT_FUNCTIONS.contains(&name)
    }
    fn feature(&self, name: &str) -> Option<bool> {
        scripting::script_feature(name)
    }
    fn command_exists(&self, name: &str) -> bool {
        scripting::host_command_exists(name)
    }
    fn message(&mut self, _: Message) {}
    fn ignorecase(&self) -> bool {
        self.commands.script_ignorecase()
    }
    fn textlock(&self) -> bool {
        true
    }
    fn buffer_name(&self) -> Option<String> {
        self.document
            .artifact_binding()
            .map(|binding| binding.path())
            .or(self.commands.current_artifact.as_ref())
            .and_then(|path| std::str::from_utf8(path.as_bytes()).ok())
            .map(str::to_owned)
    }
    fn dynamic_variable(&self, name: &[u8]) -> Option<Value> {
        self.commands.script_dynamic_variable(self.document, name)
    }
}
