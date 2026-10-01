//! Portable `CTRL-W` window command grammar.
//!
//! The frontend owns pane lifetime and focus. Shared layout geometry consumes
//! native intentions; this module never touches buffers when resolving keys.
use super::ex_execute::ExFileRequest;
use super::{CommandInterpreter, CommandOutput, CommandStatus, CountError, Key, Mode, Pending};
use crate::document::Document;

/// A window effect the frontend performs. Panes are addressed by their
/// tree traversal order; an index is one-based, matching Vim's counts.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum WindowRequest {
    /// Move focus down `count` panes, stopping at the bottom.
    FocusDown {
        count: usize,
    },
    /// Move focus up `count` panes, stopping at the top.
    FocusUp {
        count: usize,
    },
    /// Without an index, the next pane, wrapping to the top.
    FocusNext {
        index: Option<usize>,
    },
    /// Without an index, the previous pane, wrapping to the bottom.
    FocusPrevious {
        index: Option<usize>,
    },
    FocusLeft { count: usize },
    FocusRight { count: usize },
    FocusTop,
    FocusBottom,
    /// The pane focused before the current one.
    FocusLastAccessed,
    /// Every pane moves down `count`; the bottom pane becomes the top.
    RotateDown {
        count: usize,
    },
    RotateUp {
        count: usize,
    },
    /// Exchange the focused pane with the next one, with the previous one when
    /// it is last, or with the pane at `index`. Focus follows the pane.
    Exchange {
        index: Option<usize>,
    },
    MoveToTop,
    MoveToBottom,
    MoveToLeft,
    MoveToRight,
    /// Close every pane except the focused one.
    CloseOthers,
    /// Grow the focused pane by `rows` default paragraph lines, taking the space from its
    /// neighbours.
    Grow {
        rows: usize,
    },
    Shrink {
        rows: usize,
    },
    /// Set the focused pane to `rows` rows, or as tall as the window allows.
    SetHeight {
        rows: Option<usize>,
    },
    Resize { index: Option<usize>, width: bool, change: i8, size: Option<usize> },
    EqualizeHeights,
    EqualizeHeightOnly,
    EqualizeWidthOnly,
    GrowWidth { columns: usize },
    ShrinkWidth { columns: usize },
    SetWidth { columns: Option<usize> },
}

/// What `CTRL-W` followed by one key means.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum WindowCommand {
    /// A window effect for the frontend.
    Request(WindowRequest),
    /// A pane lifecycle request handled by the document host.
    File(ExFileRequest),
    /// Valid grammar with nothing to do.
    Accepted,
    /// Open the Ex prompt, as with `:` in the current selection mode.
    Prompt,
    /// A real Vim command this product deliberately does not provide, such as
    /// one that needs tab pages.
    Unsupported,
}

/// Resolve a window command after combining counts on either side of `CTRL-W`.
pub fn window_command(key: Key, count: Option<usize>) -> WindowCommand {
    let key = normalized_window_key(key);
    let repeat = count.unwrap_or(1).max(1);
    let request = |request| WindowCommand::Request(request);
    match key {
        Key::Char('j') | Key::Down | Key::Ctrl('j') => {
            request(WindowRequest::FocusDown { count: repeat })
        }
        Key::Char('k') | Key::Up | Key::Ctrl('k') => {
            request(WindowRequest::FocusUp { count: repeat })
        }
        Key::Char('w') | Key::Ctrl('w') => request(WindowRequest::FocusNext { index: count }),
        Key::Char('W') => request(WindowRequest::FocusPrevious { index: count }),
        Key::Char('t') | Key::Ctrl('t') => request(WindowRequest::FocusTop),
        Key::Char('b') | Key::Ctrl('b') => request(WindowRequest::FocusBottom),
        Key::Char('p') | Key::Ctrl('p') => request(WindowRequest::FocusLastAccessed),
        Key::Char('h') | Key::Ctrl('h') | Key::Backspace | Key::Left => request(WindowRequest::FocusLeft { count: repeat }),
        Key::Char('l') | Key::Ctrl('l') | Key::Right => request(WindowRequest::FocusRight { count: repeat }),
        Key::Char('r') | Key::Ctrl('r') => request(WindowRequest::RotateDown { count: repeat }),
        Key::Char('R') => request(WindowRequest::RotateUp { count: repeat }),
        Key::Char('x') | Key::Ctrl('x') => request(WindowRequest::Exchange { index: count }),
        Key::Char('K') => request(WindowRequest::MoveToTop),
        Key::Char('J') => request(WindowRequest::MoveToBottom),
        Key::Char('H') => request(WindowRequest::MoveToLeft),
        Key::Char('L') => request(WindowRequest::MoveToRight),
        Key::Char('o') | Key::Ctrl('o') => request(WindowRequest::CloseOthers),
        Key::Char('s' | 'S' | 'v') | Key::Ctrl('s' | 'v') => {
            WindowCommand::File(ExFileRequest::Split {
                path: None,
                height: count,
                vertical: matches!(key, Key::Char('v') | Key::Ctrl('v')),
            })
        }
        Key::Char('n') | Key::Ctrl('n') => {
            WindowCommand::File(ExFileRequest::NewPane { height: count, vertical: false })
        }
        Key::Char('q' | 'c') | Key::Ctrl('q') => {
            WindowCommand::File(ExFileRequest::Quit { force: false })
        }
        Key::Char('+') => request(WindowRequest::Grow { rows: repeat }),
        Key::Char('-') => request(WindowRequest::Shrink { rows: repeat }),
        Key::Char('_') | Key::Ctrl('_') => request(WindowRequest::SetHeight { rows: count }),
        Key::Char('>') => request(WindowRequest::GrowWidth { columns: repeat }),
        Key::Char('<') => request(WindowRequest::ShrinkWidth { columns: repeat }),
        Key::Char('|') => request(WindowRequest::SetWidth { columns: count }),
        Key::Char('=') => request(WindowRequest::EqualizeHeights),
        Key::Char(':') => WindowCommand::Prompt,
        Key::Ctrl('c') => WindowCommand::Accepted,
        // Vim tab pages remain outside the product scope.
        Key::Char('T') => WindowCommand::Unsupported,
        _ => WindowCommand::Unsupported,
    }
}

fn normalized_window_key(key: Key) -> Key {
    match key {
        Key::Ctrl(character) => Key::Ctrl(character.to_ascii_lowercase()),
        key => key,
    }
}

impl CommandInterpreter {
    /// Resolve this prefix before ordinary mode shortcuts: its CTRL-V means
    /// split, and its CTRL-C cancels just the pending command.
    pub(super) fn try_handle_window_key(
        &mut self,
        document: &Document,
        key: Key,
    ) -> Option<CommandOutput> {
        let key = normalized_window_key(key);
        if let Pending::Window {
            count: prefix_count,
        } = self.pending
        {
            if matches!(key, Key::Escape | Key::Ctrl('c')) {
                self.clear_pending();
                return Some(CommandOutput {
                    status: CommandStatus::Cancelled,
                    ..CommandOutput::complete()
                });
            }
            if let Key::Char(digit @ '0'..='9') = key {
                if digit != '0' || self.count.is_some() {
                    return Some(self.push_count(digit));
                }
            }
            if key == Key::Delete && self.count.is_some() {
                self.count = self
                    .count
                    .and_then(|count| (count >= 10).then_some(count / 10));
                return Some(CommandOutput::pending());
            }
            let suffix_count = self.count.take();
            self.pending = Pending::None;
            let count = match (prefix_count, suffix_count) {
                (Some(before), Some(after)) => match before.checked_mul(after) {
                    Some(count) => Some(count),
                    None => return Some(CommandOutput::count_error(CountError::Overflow)),
                },
                (before, after) => before.or(after),
            };
            return Some(self.execute_window_command(document, key, count));
        }
        if key == Key::Ctrl('w')
            && self.pending == Pending::None
            && !self.register_pending
            && matches!(
                self.mode,
                Mode::Normal | Mode::VisualCharacter | Mode::VisualLine | Mode::VisualBlock
            )
        {
            if let Some(output) = self.finish_overflowed_count() {
                return Some(output);
            }
            self.pending = Pending::Window {
                count: self.count.take(),
            };
            self.requested_register = None;
            return Some(CommandOutput::pending());
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn control_letter_case_does_not_change_window_commands() {
        for character in 'A'..='Z' {
            for count in [None, Some(3)] {
                assert_eq!(
                    window_command(Key::Ctrl(character), count),
                    window_command(Key::Ctrl(character.to_ascii_lowercase()), count)
                );
            }
        }
        assert_eq!(
            window_command(Key::Char('W'), None),
            WindowCommand::Request(WindowRequest::FocusPrevious { index: None })
        );
        assert_eq!(
            window_command(Key::Ctrl('W'), None),
            WindowCommand::Request(WindowRequest::FocusNext { index: None })
        );
    }

    #[test]
    fn focus_commands_accept_their_vim_spellings_and_counts() {
        for key in [Key::Char('j'), Key::Down, Key::Ctrl('j')] {
            assert_eq!(
                window_command(key, None),
                WindowCommand::Request(WindowRequest::FocusDown { count: 1 }),
                "{key:?}"
            );
            assert_eq!(
                window_command(key, Some(3)),
                WindowCommand::Request(WindowRequest::FocusDown { count: 3 }),
                "{key:?}"
            );
        }
        for key in [Key::Char('k'), Key::Up, Key::Ctrl('k')] {
            assert_eq!(
                window_command(key, None),
                WindowCommand::Request(WindowRequest::FocusUp { count: 1 }),
                "{key:?}"
            );
        }
        for key in [Key::Char('w'), Key::Ctrl('w')] {
            assert_eq!(
                window_command(key, None),
                WindowCommand::Request(WindowRequest::FocusNext { index: None }),
                "{key:?}"
            );
            assert_eq!(
                window_command(key, Some(2)),
                WindowCommand::Request(WindowRequest::FocusNext { index: Some(2) }),
                "{key:?}"
            );
        }
        assert_eq!(
            window_command(Key::Char('W'), Some(2)),
            WindowCommand::Request(WindowRequest::FocusPrevious { index: Some(2) })
        );
        for (key, expected) in [
            (Key::Char('t'), WindowRequest::FocusTop),
            (Key::Ctrl('t'), WindowRequest::FocusTop),
            (Key::Char('b'), WindowRequest::FocusBottom),
            (Key::Ctrl('b'), WindowRequest::FocusBottom),
            (Key::Char('p'), WindowRequest::FocusLastAccessed),
            (Key::Ctrl('p'), WindowRequest::FocusLastAccessed),
        ] {
            assert_eq!(window_command(key, None), WindowCommand::Request(expected));
        }
    }

    #[test]
    fn horizontal_focus_carries_direction_and_counts() {
        for (key,left) in [(Key::Char('h'),true),(Key::Ctrl('h'),true),(Key::Backspace,true),(Key::Left,true),(Key::Char('l'),false),(Key::Ctrl('l'),false),(Key::Right,false)] {
            for count in [None,Some(4)] {
                let count_value=count.unwrap_or(1);
                assert_eq!(window_command(key,count),WindowCommand::Request(if left {WindowRequest::FocusLeft {count:count_value}} else {WindowRequest::FocusRight {count:count_value}}));
            }
        }
    }

    #[test]
    fn order_commands_carry_their_counts_and_indexes() {
        assert_eq!(
            window_command(Key::Char('r'), None),
            WindowCommand::Request(WindowRequest::RotateDown { count: 1 })
        );
        assert_eq!(
            window_command(Key::Ctrl('r'), Some(2)),
            WindowCommand::Request(WindowRequest::RotateDown { count: 2 })
        );
        assert_eq!(
            window_command(Key::Char('R'), Some(2)),
            WindowCommand::Request(WindowRequest::RotateUp { count: 2 })
        );
        assert_eq!(
            window_command(Key::Char('x'), None),
            WindowCommand::Request(WindowRequest::Exchange { index: None })
        );
        assert_eq!(
            window_command(Key::Ctrl('x'), Some(3)),
            WindowCommand::Request(WindowRequest::Exchange { index: Some(3) })
        );
        assert_eq!(
            window_command(Key::Char('K'), None),
            WindowCommand::Request(WindowRequest::MoveToTop)
        );
        assert_eq!(
            window_command(Key::Char('J'), None),
            WindowCommand::Request(WindowRequest::MoveToBottom)
        );
    }

    #[test]
    fn lifecycle_commands_reuse_their_ex_requests() {
        for key in [
            Key::Char('s'),
            Key::Char('S'),
            Key::Ctrl('s'),
            Key::Char('v'),
            Key::Ctrl('v'),
        ] {
            assert_eq!(
                window_command(key, None),
                WindowCommand::File(ExFileRequest::Split {
                    path: None,
                    height: None, vertical: matches!(key, Key::Char('v') | Key::Ctrl('v')) }),
                "{key:?}"
            );
        }
        for key in [Key::Char('n'), Key::Ctrl('n')] {
            assert_eq!(
                window_command(key, None),
                WindowCommand::File(ExFileRequest::NewPane { height: None, vertical: false }),
                "{key:?}"
            );
        }
        for key in [Key::Char('q'), Key::Ctrl('q'), Key::Char('c')] {
            assert_eq!(
                window_command(key, None),
                WindowCommand::File(ExFileRequest::Quit { force: false }),
                "{key:?}"
            );
        }
        for key in [Key::Char('o'), Key::Ctrl('o')] {
            assert_eq!(
                window_command(key, None),
                WindowCommand::Request(WindowRequest::CloseOthers),
                "{key:?}"
            );
        }
    }

    #[test]
    fn size_commands_default_to_one_row_and_an_open_height() {
        assert_eq!(
            window_command(Key::Char('+'), None),
            WindowCommand::Request(WindowRequest::Grow { rows: 1 })
        );
        assert_eq!(
            window_command(Key::Char('+'), Some(5)),
            WindowCommand::Request(WindowRequest::Grow { rows: 5 })
        );
        assert_eq!(
            window_command(Key::Char('-'), Some(5)),
            WindowCommand::Request(WindowRequest::Shrink { rows: 5 })
        );
        assert_eq!(
            window_command(Key::Char('_'), None),
            WindowCommand::Request(WindowRequest::SetHeight { rows: None })
        );
        assert_eq!(
            window_command(Key::Char('_'), Some(12)),
            WindowCommand::Request(WindowRequest::SetHeight { rows: Some(12) })
        );
        assert_eq!(
            window_command(Key::Char('='), None),
            WindowCommand::Request(WindowRequest::EqualizeHeights)
        );
    }

    #[test]
    fn layouts_this_product_lacks_are_reported_not_ignored() {
        for key in [
            Key::Char('T'),
            Key::Char('z'),
            Key::Char('5'),
            Key::Enter,
            Key::Tab,
        ] {
            assert_eq!(
                window_command(key, None),
                WindowCommand::Unsupported,
                "{key:?}"
            );
        }
    }
}
