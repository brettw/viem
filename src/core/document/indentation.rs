//! Portable indentation columns and buffer-owned editing options. Layout gives
//! these logical columns physical widths; this module never measures a font.
use serde::{Deserialize, Serialize};
use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct IndentationOptions {
    pub autoindent: bool,
    pub tabstop: u32,
    pub shiftwidth: u32,
    pub softtabstop: i32,
    pub expandtab: bool,
    pub smarttab: bool,
    pub continue_comments_on_enter: bool,
    pub continue_comments_on_open_line: bool,
}
impl Default for IndentationOptions {
    fn default() -> Self {
        Self {
            autoindent: true,
            tabstop: 2,
            shiftwidth: 2,
            softtabstop: 2,
            expandtab: true,
            smarttab: true,
            continue_comments_on_enter: true,
            continue_comments_on_open_line: true,
        }
    }
}
impl IndentationOptions {
    pub fn validate(&self) -> Result<(), &'static str> {
        if !(1..=1024).contains(&self.tabstop) {
            return Err("tabstop must be between 1 and 1024");
        }
        if self.shiftwidth > 1024 {
            return Err("shiftwidth must be between 0 and 1024");
        }
        if !(-1..=1024).contains(&self.softtabstop) {
            return Err("softtabstop must be between -1 and 1024");
        }
        Ok(())
    }
    pub fn shift_width(self) -> usize {
        (if self.shiftwidth == 0 {
            self.tabstop
        } else {
            self.shiftwidth
        }) as usize
    }
    pub fn soft_tab_width(self) -> usize {
        if self.softtabstop < 0 {
            self.shift_width()
        } else {
            self.softtabstop as usize
        }
    }
    pub fn columns(self, text: &str) -> usize {
        end_column(0, text, self.tabstop as usize)
    }
    /// Minimal whitespace advancing from one logical column to another.
    pub fn whitespace(self, start: usize, end: usize) -> String {
        self.try_whitespace(start, end)
            .expect("indentation allocation")
    }

    pub fn try_whitespace(
        self,
        start: usize,
        end: usize,
    ) -> Result<String, std::collections::TryReserveError> {
        let mut result = String::new();
        let mut column = start;
        let tabstop = self.tabstop as usize;
        let first_tab = start.checked_add(tabstop - start % tabstop);
        let tab_count = if !self.expandtab {
            first_tab
                .filter(|first| *first <= end)
                .map_or(0, |first| (end - first) / tabstop + 1)
        } else {
            0
        };
        let spaces = if tab_count > 0 {
            end % tabstop
        } else {
            end.saturating_sub(start)
        };
        result.try_reserve_exact(tab_count.saturating_add(spaces))?;
        while column < end {
            let next = column.checked_add(tabstop - column % tabstop);
            if !self.expandtab && next.is_some_and(|next| next <= end && next - column > 1) {
                result.push('\t');
                column = next.unwrap();
            } else {
                result.push(' ');
                column += 1;
            }
        }
        Ok(result)
    }
}

/// Per-field overrides keep unrelated settings inherited after `:setlocal`.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct IndentationOverrides {
    pub autoindent: Option<bool>,
    pub tabstop: Option<u32>,
    pub shiftwidth: Option<u32>,
    pub softtabstop: Option<i32>,
    pub expandtab: Option<bool>,
    pub smarttab: Option<bool>,
    pub continue_comments_on_enter: Option<bool>,
    pub continue_comments_on_open_line: Option<bool>,
}
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct IndentationSetting {
    default: IndentationOptions,
    pub local: IndentationOverrides,
}
impl IndentationSetting {
    pub fn effective(self) -> IndentationOptions {
        let d = self.default;
        let l = self.local;
        IndentationOptions {
            autoindent: l.autoindent.unwrap_or(d.autoindent),
            tabstop: l.tabstop.unwrap_or(d.tabstop),
            shiftwidth: l.shiftwidth.unwrap_or(d.shiftwidth),
            softtabstop: l.softtabstop.unwrap_or(d.softtabstop),
            expandtab: l.expandtab.unwrap_or(d.expandtab),
            smarttab: l.smarttab.unwrap_or(d.smarttab),
            continue_comments_on_enter: l
                .continue_comments_on_enter
                .unwrap_or(d.continue_comments_on_enter),
            continue_comments_on_open_line: l
                .continue_comments_on_open_line
                .unwrap_or(d.continue_comments_on_open_line),
        }
    }
    pub fn defaults(self) -> IndentationOptions {
        self.default
    }
    pub fn set_default(&mut self, options: IndentationOptions) {
        self.default = options;
    }
}

pub fn leading_whitespace_len(text: &str) -> usize {
    text.graphemes(true)
        .take_while(|g| matches!(*g, " " | "\t"))
        .map(str::len)
        .sum()
}
pub fn end_column(start: usize, text: &str, tabstop: usize) -> usize {
    text.graphemes(true).fold(start, |column, grapheme| {
        if grapheme == "\t" {
            column + tabstop - column % tabstop
        } else {
            column + UnicodeWidthStr::width(grapheme)
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn defaults_and_independent_overrides() {
        let mut setting = IndentationSetting::default();
        setting.local.tabstop = Some(8);
        setting.set_default(IndentationOptions {
            shiftwidth: 3,
            ..Default::default()
        });
        assert_eq!(setting.effective().tabstop, 8);
        assert_eq!(setting.effective().shift_width(), 3);
        setting.local.tabstop = None;
        assert_eq!(setting.effective().tabstop, 2);
    }
    #[test]
    fn columns_and_minimal_mixed_whitespace() {
        let o = IndentationOptions {
            tabstop: 8,
            expandtab: false,
            ..Default::default()
        };
        assert_eq!(o.columns(" \t "), 9);
        assert_eq!(o.whitespace(3, 10), "\t  ");
        assert_eq!(leading_whitespace_len(" \u{301}x"), 0);
        assert!(IndentationOptions { tabstop: 0, ..o }.validate().is_err());
    }
}
