//! Vim color scheme evaluation for theme import. The bounded setup language
//! supplies control flow and expressions; this module owns `:highlight`
//! semantics, Vim's compiled-in default groups and GUI color names. Evaluation
//! models gVim with `:syntax on`, reads no files, and never runs Vim.
use std::collections::BTreeMap;

mod names;
#[cfg(test)]
mod tests;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum VimBackground {
    Light,
    Dark,
}

impl VimBackground {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Light => "light",
            Self::Dark => "dark",
        }
    }
}

/// A GUI color. `Foreground` and `Background` are Vim's `fg`/`bg` references
/// to the Normal group, retained symbolically when Normal leaves them unset.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum VimColor {
    Rgb([u8; 3]),
    Foreground,
    Background,
}

/// `gui=` attribute flags.
pub mod flags {
    pub const BOLD: u16 = 1;
    pub const ITALIC: u16 = 1 << 1;
    pub const UNDERLINE: u16 = 1 << 2;
    pub const UNDERCURL: u16 = 1 << 3;
    pub const UNDERDOUBLE: u16 = 1 << 4;
    pub const UNDERDOTTED: u16 = 1 << 5;
    pub const UNDERDASHED: u16 = 1 << 6;
    pub const STRIKETHROUGH: u16 = 1 << 7;
    pub const REVERSE: u16 = 1 << 8;
    pub const STANDOUT: u16 = 1 << 9;
    pub const NOCOMBINE: u16 = 1 << 10;
    pub const ANY_UNDERLINE: u16 = UNDERLINE | UNDERCURL | UNDERDOUBLE | UNDERDOTTED | UNDERDASHED;
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct VimGuiAttributes {
    pub foreground: Option<VimColor>,
    pub background: Option<VimColor>,
    pub special: Option<VimColor>,
    pub flags: u16,
    /// A `font=` declaration, which theme import does not map.
    pub font: bool,
}

/// The appearance of a group after following its links.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct VimResolvedHighlight {
    /// The group whose attributes apply, after links.
    pub group: String,
    pub gui: VimGuiAttributes,
}

const TERM: u8 = 1;
const CTERM: u8 = 1 << 1;
const CTERM_FG: u8 = 1 << 2;
const CTERM_BG: u8 = 1 << 3;
const CTERM_UL: u8 = 1 << 4;
const CTERM_FONT: u8 = 1 << 5;

const MAX_GROUPS: usize = 8192;
const MAX_GROUP_NAME: usize = 200;
const MAX_LINK_DEPTH: usize = 100;

#[derive(Clone, Debug, Default)]
struct Group {
    name: String,
    gui: VimGuiAttributes,
    /// Nonempty terminal attributes, which still give a group "settings".
    other: u8,
    link: Option<String>,
    default_link: Option<String>,
    /// GUI attributes or a link were set by a non-default `:highlight`. Vim's
    /// compiled-in defaults never replace them.
    user_gui: bool,
    user_link: bool,
    cleared: bool,
    /// The color scheme itself, not a Vim default, last defined this group.
    scheme: bool,
}

impl Group {
    fn has_settings(&self, check_link: bool) -> bool {
        !self.cleared
            && (self.other != 0
                || self.gui != VimGuiAttributes::default()
                || check_link && self.user_link)
    }
    fn clear(&mut self) {
        self.gui = VimGuiAttributes::default();
        self.other = 0;
        self.link = self.default_link.clone();
        self.cleared = true;
    }
}

/// Who issued a `:highlight` command.
#[derive(Clone, Copy)]
enum Origin {
    /// Vim's compiled-in defaults (`init_highlight`).
    Default,
    /// `syntax/syncolor.vim`, which Vim sources whenever defaults are loaded.
    SyntaxColors,
    Scheme,
}

/// The highlight table of a gVim session with `:syntax on`.
#[derive(Clone, Debug)]
pub struct VimHighlights {
    background: VimBackground,
    groups: BTreeMap<String, Group>,
    /// `:highlight` commands the color scheme issued, including failed ones.
    commands: usize,
}

impl VimHighlights {
    /// gVim's state before a color scheme loads: 'background' is light.
    pub(crate) fn startup() -> Self {
        let mut highlights = Self { background: VimBackground::Light, groups: BTreeMap::new(), commands: 0 };
        highlights.load_defaults(true);
        highlights
    }

    pub fn background(&self) -> VimBackground {
        self.background
    }

    /// `:highlight clear` restores every group to Vim's defaults.
    fn clear_all(&mut self) {
        self.groups.clear();
        self.load_defaults(true);
    }

    fn load_defaults(&mut self, both: bool) {
        if both {
            self.apply_table(defaults::UI_BOTH, Origin::Default);
        }
        self.apply_table(
            match self.background {
                VimBackground::Light => defaults::UI_LIGHT,
                VimBackground::Dark => defaults::UI_DARK,
            },
            Origin::Default,
        );
        self.syntax_colors(false);
    }

    fn apply_table(&mut self, table: &[(&str, &str)], origin: Origin) {
        for (name, attributes) in table {
            let command = match attributes.strip_prefix("link ") {
                Some(target) => format!("default link {name} {target}"),
                None => format!("{name} {attributes}"),
            };
            let applied = self.apply(&command, false, origin);
            debug_assert!(applied.is_ok(), "{command}: {applied:?}");
        }
    }

    /// `syntax/syncolor.vim`. `:syntax reset` forces its links.
    fn syntax_colors(&mut self, reset: bool) {
        let colors = match self.background {
            VimBackground::Light => defaults::SYNTAX_LIGHT,
            VimBackground::Dark => defaults::SYNTAX_DARK,
        };
        for table in [colors, defaults::SYNTAX_BOTH] {
            for (name, attributes) in table {
                let applied = self.apply(&format!("{name} {attributes}"), false, Origin::SyntaxColors);
                debug_assert!(applied.is_ok(), "{name}: {applied:?}");
            }
        }
        for (name, target) in defaults::SYNTAX_LINKS {
            let applied = self.apply(&format!("link {name} {target}"), reset, Origin::SyntaxColors);
            debug_assert!(applied.is_ok(), "{name}: {applied:?}");
        }
    }

    /// `:syntax reset`.
    pub(crate) fn syntax_reset(&mut self) {
        self.syntax_colors(true);
    }

    /// Setting 'background' reloads defaults for groups the scheme has not
    /// set, unless a named color scheme is loading (Vim's recursion guard).
    pub(crate) fn set_background(&mut self, background: VimBackground, colors_name: bool) {
        self.background = background;
        if !colors_name {
            self.load_defaults(false);
        }
    }

    /// One `:highlight` command from the color scheme. Returns whether it
    /// cleared every group, which also unsets `g:colors_name`.
    pub(crate) fn command(&mut self, arguments: &str, forced: bool) -> Result<bool, String> {
        self.commands = self.commands.saturating_add(1);
        self.apply(arguments, forced, Origin::Scheme)
    }

    /// Whether the source issued any `:highlight` command. Vim's own
    /// `default.vim` only clears, which still loads Vim's default colors.
    pub fn has_highlight_commands(&self) -> bool {
        self.commands > 0
    }

    fn apply(&mut self, arguments: &str, forced: bool, origin: Origin) -> Result<bool, String> {
        let words = words(arguments)?;
        let mut words = words.iter().map(String::as_str).peekable();
        let default = words.next_if(|word| matches!(*word, "def" | "default")).is_some();
        let Some(first) = words.next() else { return Ok(false) };
        let init = matches!(origin, Origin::Default);
        if first == "clear" {
            let names = words.collect::<Vec<_>>();
            if names.is_empty() {
                self.clear_all();
                return Ok(true);
            }
            for name in names {
                let group = self.group(name)?;
                group.clear();
                group.scheme |= matches!(origin, Origin::Scheme);
            }
            return Ok(false);
        }
        if first == "link" {
            let (Some(from), Some(to), None) = (words.next(), words.next(), words.next()) else {
                return Err("E412: Not enough arguments to :highlight link".into());
            };
            let to = (!to.eq_ignore_ascii_case("NONE"))
                .then(|| self.group(to).map(|group| group.name.to_ascii_lowercase()))
                .transpose()?;
            let group = self.group(from)?;
            if default {
                group.default_link = to.clone();
                if group.has_settings(true) {
                    return Ok(false);
                }
            }
            if init && (group.user_gui || group.user_link) {
                return Ok(false);
            }
            if to.is_some() && !forced && !init && group.has_settings(default) {
                // E414: the group has settings and the link is ignored.
                return Ok(false);
            }
            group.link = to;
            group.cleared = false;
            if !init {
                group.user_link = true;
            }
            group.scheme |= matches!(origin, Origin::Scheme);
            return Ok(false);
        }
        let group = self.group(first)?;
        let attributes = words.collect::<Vec<_>>();
        if attributes.is_empty() {
            // `:highlight Group` lists the group; it changes nothing.
            return Ok(false);
        }
        if default && group.has_settings(true) {
            return Ok(false);
        }
        let mut result = Ok(());
        let mut changed = false;
        for attribute in attributes {
            if attribute.eq_ignore_ascii_case("NONE") {
                if !init || !group.user_gui {
                    group.clear();
                    group.user_gui |= !init;
                    changed = true;
                }
                continue;
            }
            let Some((key, value)) = attribute.split_once('=') else {
                result = Err(format!("E416: Missing equal sign: {attribute}"));
                break;
            };
            let value = value
                .strip_prefix('\'')
                .and_then(|value| value.strip_suffix('\''))
                .unwrap_or(value);
            if value.is_empty() {
                result = Err(format!("E417: Missing argument: {attribute}"));
                break;
            }
            let key = key.to_ascii_lowercase();
            let other = match key.as_str() {
                "term" | "start" | "stop" => Some(TERM),
                "cterm" => Some(CTERM),
                "ctermfg" => Some(CTERM_FG),
                "ctermbg" => Some(CTERM_BG),
                "ctermul" => Some(CTERM_UL),
                "ctermfont" => Some(CTERM_FONT),
                _ => None,
            };
            if let Some(bit) = other {
                if value.eq_ignore_ascii_case("NONE") {
                    group.other &= !bit;
                } else {
                    group.other |= bit;
                }
                changed = true;
                continue;
            }
            if key == "blend" {
                continue;
            }
            if !matches!(key.as_str(), "gui" | "guifg" | "guibg" | "guisp" | "font") {
                result = Err(format!("E423: Illegal argument: {attribute}"));
                break;
            }
            // Compiled-in defaults never replace GUI attributes a script set.
            if init && group.user_gui {
                continue;
            }
            let parsed = match key.as_str() {
                "gui" => gui_flags(value).map(|flags| group.gui.flags = flags),
                "font" => {
                    group.gui.font = !value.eq_ignore_ascii_case("NONE");
                    Ok(())
                }
                color => parse_color(value).map(|value| match color {
                    "guifg" => group.gui.foreground = value,
                    "guibg" => group.gui.background = value,
                    _ => group.gui.special = value,
                }),
            };
            if let Err(error) = parsed {
                result = Err(error);
                break;
            }
            if !init {
                group.user_gui = true;
            }
            changed = true;
        }
        if changed {
            group.cleared = group.gui == VimGuiAttributes::default() && group.other == 0;
            if !init || !group.user_link {
                group.link = None;
            }
            group.scheme |= matches!(origin, Origin::Scheme);
        }
        result.map(|()| false)
    }

    fn group(&mut self, name: &str) -> Result<&mut Group, String> {
        if name.is_empty()
            || name.len() > MAX_GROUP_NAME
            || !name.bytes().all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'.' | b'@' | b'-'))
        {
            return Err(format!("E669: Invalid highlight group name: {name}"));
        }
        let key = name.to_ascii_lowercase();
        if !self.groups.contains_key(&key) && self.groups.len() >= MAX_GROUPS {
            return Err("E849: Too many highlight and syntax groups".into());
        }
        Ok(self.groups.entry(key).or_insert_with(|| Group { name: name.to_owned(), ..Default::default() }))
    }

    /// Whether the color scheme, rather than a Vim default, last set this
    /// group (as attributes, a link, or a clear).
    pub fn defined_by_scheme(&self, name: &str) -> bool {
        self.groups.get(&name.to_ascii_lowercase()).is_some_and(|group| group.scheme)
    }

    /// Whether the group exists, by its own declarations or a default.
    pub fn contains(&self, name: &str) -> bool {
        self.groups.contains_key(&name.to_ascii_lowercase())
    }

    /// Follow links to the group whose attributes apply. `fg`/`bg` references
    /// resolve to Normal's colors where it has them. A cyclic or overlong link
    /// chain has no appearance, as in Vim.
    pub fn resolve(&self, name: &str) -> Option<VimResolvedHighlight> {
        let mut group = self.groups.get(&name.to_ascii_lowercase())?;
        for _ in 0..MAX_LINK_DEPTH {
            let Some(next) = group.link.as_ref() else {
                let normal = self.groups.get("normal").map(|normal| normal.gui).unwrap_or_default();
                let resolve = |color: Option<VimColor>| match color {
                    Some(VimColor::Foreground) => normal.foreground.filter(|color| matches!(color, VimColor::Rgb(_))).or(color),
                    Some(VimColor::Background) => normal.background.filter(|color| matches!(color, VimColor::Rgb(_))).or(color),
                    color => color,
                };
                let mut gui = group.gui;
                gui.foreground = resolve(gui.foreground);
                gui.background = resolve(gui.background);
                gui.special = resolve(gui.special);
                return Some(VimResolvedHighlight { group: group.name.clone(), gui });
            };
            group = self.groups.get(next)?;
        }
        None
    }
}

/// Split `:highlight` arguments. A quoted value (`font='Monaco 12'`,
/// `guifg='dark blue'`) may contain spaces.
fn words(source: &str) -> Result<Vec<String>, String> {
    let mut words = Vec::new();
    let mut chars = source.trim().chars().peekable();
    while chars.peek().is_some() {
        let mut word = String::new();
        let mut quoted = false;
        while let Some(&ch) = chars.peek() {
            if ch.is_whitespace() && !quoted {
                break;
            }
            if ch == '\'' && word.ends_with('=') {
                quoted = true;
            } else if ch == '\'' && quoted {
                quoted = false;
            }
            word.push(ch);
            chars.next();
        }
        if quoted {
            return Err(format!("E475: Unterminated quoted highlight value: {word}"));
        }
        while chars.next_if(|ch| ch.is_whitespace()).is_some() {}
        words.push(word);
    }
    Ok(words)
}

fn gui_flags(value: &str) -> Result<u16, String> {
    let mut result = 0;
    for name in value.split(',') {
        result |= match name.to_ascii_lowercase().as_str() {
            "none" => 0,
            "bold" => flags::BOLD,
            "italic" => flags::ITALIC,
            "underline" => flags::UNDERLINE,
            "undercurl" => flags::UNDERCURL,
            "underdouble" => flags::UNDERDOUBLE,
            "underdotted" => flags::UNDERDOTTED,
            "underdashed" => flags::UNDERDASHED,
            "strikethrough" => flags::STRIKETHROUGH,
            "reverse" | "inverse" => flags::REVERSE,
            "standout" => flags::STANDOUT,
            "nocombine" => flags::NOCOMBINE,
            _ => return Err(format!("E418: Illegal value: {value}")),
        };
    }
    Ok(result)
}

/// A `guifg`/`guibg`/`guisp` value; `NONE` is no color.
pub(crate) fn parse_color(value: &str) -> Result<Option<VimColor>, String> {
    let lower = value.to_ascii_lowercase();
    match lower.as_str() {
        "none" => return Ok(None),
        "fg" | "foreground" => return Ok(Some(VimColor::Foreground)),
        "bg" | "background" => return Ok(Some(VimColor::Background)),
        _ => {}
    }
    if let Some(hex) = lower.strip_prefix('#') {
        if hex.len() == 6 && hex.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            let rgb = u32::from_str_radix(hex, 16).map_err(|error| error.to_string())?;
            return Ok(Some(rgb_color(rgb)));
        }
    }
    let lookup = |name: &str| names::NAMES.binary_search_by(|(key, _)| (*key).cmp(name)).ok().map(|index| names::NAMES[index].1);
    lookup(&lower)
        .or_else(|| lookup(&lower.replace(' ', "")))
        .map(|rgb| Some(rgb_color(rgb)))
        .ok_or_else(|| format!("E254: Cannot allocate color {value}"))
}

fn rgb_color(rgb: u32) -> VimColor {
    VimColor::Rgb([(rgb >> 16) as u8, (rgb >> 8) as u8, rgb as u8])
}

/// GUI-relevant parts of Vim 9.1's compiled-in groups and `syncolor.vim`.
/// Terminal attributes are kept only where they give a group settings.
mod defaults {
    pub(super) const UI_BOTH: &[(&str, &str)] = &[
        ("EndOfBuffer", "link NonText"),
        ("NonText", "term=bold gui=bold guifg=Blue"),
        ("ErrorMsg", "term=standout guifg=White guibg=Red"),
        ("IncSearch", "term=reverse cterm=reverse gui=reverse"),
        ("CurSearch", "link Search"),
        ("MoreMsg", "term=bold gui=bold guifg=SeaGreen"),
        ("ModeMsg", "term=bold cterm=bold gui=bold"),
        ("CursorLineSign", "link SignColumn"),
        ("CursorLineFold", "link FoldColumn"),
        ("StatusLine", "term=bold,reverse cterm=bold,reverse gui=bold,reverse"),
        ("StatusLineNC", "term=reverse cterm=reverse gui=reverse"),
        ("VertSplit", "term=reverse cterm=reverse gui=reverse"),
        ("Title", "term=bold gui=bold guifg=Magenta"),
        ("VisualNOS", "term=bold,underline cterm=bold,underline gui=bold,underline"),
        ("WarningMsg", "term=standout guifg=Red"),
        ("WildMenu", "term=standout guifg=Black guibg=Yellow"),
        ("DiffText", "term=reverse cterm=bold gui=bold guibg=Red"),
        ("DiffTextAdd", "link DiffText"),
        ("Conceal", "guifg=LightGrey guibg=DarkGrey"),
        ("SpellBad", "term=reverse gui=undercurl guisp=Red"),
        ("SpellCap", "term=reverse gui=undercurl guisp=Blue"),
        ("SpellRare", "term=reverse gui=undercurl guisp=Magenta"),
        ("PmenuMatch", "link Pmenu"),
        ("PmenuMatchSel", "link PmenuSel"),
        ("PmenuKind", "link Pmenu"),
        ("PmenuKindSel", "link PmenuSel"),
        ("PmenuExtra", "link Pmenu"),
        ("PmenuExtraSel", "link PmenuSel"),
        ("PmenuSbar", "guibg=Grey"),
        ("TabLineSel", "term=bold cterm=bold gui=bold"),
        ("TabLineFill", "term=reverse cterm=reverse gui=reverse"),
        ("QuickFixLine", "link Search"),
        ("TabPanel", "link TabLine"),
        ("TabPanelSel", "link TabLineSel"),
        ("TabPanelFill", "link TabLineFill"),
        ("PreInsert", "link Added"),
        ("PopupSelected", "link PmenuSel"),
        ("MessageWindow", "link WarningMsg"),
        ("PopupNotification", "link WarningMsg"),
        ("Cursor", "guibg=fg guifg=bg"),
        ("lCursor", "guibg=fg guifg=bg"),
    ];
    pub(super) const UI_LIGHT: &[(&str, &str)] = &[
        ("SpecialKey", "term=bold guifg=Blue"),
        ("Directory", "term=bold guifg=Blue"),
        ("Search", "term=reverse guibg=Yellow"),
        ("LineNr", "term=underline guifg=Brown"),
        ("CursorLineNr", "term=bold cterm=underline gui=bold guifg=Brown"),
        ("Question", "term=standout gui=bold guifg=SeaGreen"),
        ("Visual", "term=reverse cterm=reverse guifg=Black guibg=LightGrey"),
        ("Folded", "term=standout guifg=DarkBlue guibg=LightGrey"),
        ("FoldColumn", "term=standout guifg=DarkBlue guibg=Grey"),
        ("DiffAdd", "term=bold guibg=LightBlue"),
        ("DiffChange", "term=bold guibg=LightMagenta"),
        ("DiffDelete", "term=bold gui=bold guifg=Blue guibg=LightCyan"),
        ("SignColumn", "term=standout guifg=DarkBlue guibg=Grey"),
        ("SpellLocal", "term=underline gui=undercurl guisp=DarkCyan"),
        ("Pmenu", "guibg=LightMagenta"),
        ("PmenuSel", "guibg=Grey"),
        ("PmenuThumb", "guibg=Black"),
        ("TabLine", "term=underline cterm=underline gui=underline guibg=LightGrey"),
        ("CursorColumn", "term=reverse guibg=Grey90"),
        ("CursorLine", "term=underline cterm=underline guibg=Grey90"),
        ("ColorColumn", "term=reverse guibg=LightRed"),
        ("StatusLineTerm", "term=bold,reverse cterm=bold gui=bold guifg=bg guibg=DarkGreen"),
        ("StatusLineTermNC", "term=reverse guifg=bg guibg=DarkGreen"),
        ("MatchParen", "term=reverse guibg=Cyan"),
        ("ToolbarLine", "term=underline guibg=LightGrey"),
        ("ToolbarButton", "cterm=bold gui=bold guifg=White guibg=Grey40"),
    ];
    pub(super) const UI_DARK: &[(&str, &str)] = &[
        ("SpecialKey", "term=bold guifg=Cyan"),
        ("Directory", "term=bold guifg=Cyan"),
        ("Search", "term=reverse guifg=Black guibg=Yellow"),
        ("LineNr", "term=underline guifg=Yellow"),
        ("CursorLineNr", "term=bold cterm=underline gui=bold guifg=Yellow"),
        ("Question", "term=standout gui=bold guifg=Green"),
        ("Visual", "term=reverse cterm=reverse guifg=LightGrey guibg=#575757"),
        ("Folded", "term=standout guifg=Cyan guibg=DarkGrey"),
        ("FoldColumn", "term=standout guifg=Cyan guibg=Grey"),
        ("DiffAdd", "term=bold guibg=DarkBlue"),
        ("DiffChange", "term=bold guibg=DarkMagenta"),
        ("DiffDelete", "term=bold gui=bold guifg=Blue guibg=DarkCyan"),
        ("SignColumn", "term=standout guifg=Cyan guibg=Grey"),
        ("SpellLocal", "term=underline gui=undercurl guisp=Cyan"),
        ("Pmenu", "guibg=Magenta"),
        ("PmenuSel", "guibg=DarkGrey"),
        ("PmenuThumb", "guibg=White"),
        ("TabLine", "term=underline cterm=underline gui=underline guibg=DarkGrey"),
        ("CursorColumn", "term=reverse guibg=Grey40"),
        ("CursorLine", "term=underline cterm=underline guibg=Grey40"),
        ("ColorColumn", "term=reverse guibg=DarkRed"),
        ("StatusLineTerm", "term=bold,reverse cterm=bold gui=bold guifg=bg guibg=LightGreen"),
        ("StatusLineTermNC", "term=reverse guifg=bg guibg=LightGreen"),
        ("MatchParen", "term=reverse guibg=DarkCyan"),
        ("ToolbarLine", "term=underline guibg=Grey50"),
        ("ToolbarButton", "cterm=bold gui=bold guifg=Black guibg=LightGrey"),
    ];
    // `syncolor.vim` assigns complete GUI attributes, replacing earlier ones.
    pub(super) const SYNTAX_LIGHT: &[(&str, &str)] = &[
        ("Comment", "term=bold gui=NONE guifg=Blue guibg=NONE"),
        ("Constant", "term=underline gui=NONE guifg=Magenta guibg=NONE"),
        ("Special", "term=bold gui=NONE guifg=#6a5acd guibg=NONE"),
        ("Identifier", "term=underline gui=NONE guifg=DarkCyan guibg=NONE"),
        ("Statement", "term=bold gui=bold guifg=Brown guibg=NONE"),
        ("PreProc", "term=underline gui=NONE guifg=#6a0dad guibg=NONE"),
        ("Type", "term=underline gui=bold guifg=SeaGreen guibg=NONE"),
        ("Underlined", "term=underline cterm=underline gui=underline guifg=SlateBlue"),
        ("Ignore", "ctermfg=white gui=NONE guifg=bg guibg=NONE"),
        ("Added", "ctermfg=DarkGreen gui=NONE guifg=SeaGreen guibg=NONE"),
        ("Changed", "ctermfg=Blue gui=NONE guifg=DodgerBlue guibg=NONE"),
        ("Removed", "ctermfg=Red gui=NONE guifg=Red guibg=NONE"),
    ];
    pub(super) const SYNTAX_DARK: &[(&str, &str)] = &[
        ("Comment", "term=bold gui=NONE guifg=#80a0ff guibg=NONE"),
        ("Constant", "term=underline gui=NONE guifg=#ffa0a0 guibg=NONE"),
        ("Special", "term=bold gui=NONE guifg=Orange guibg=NONE"),
        ("Identifier", "term=underline cterm=bold gui=NONE guifg=#40ffff guibg=NONE"),
        ("Statement", "term=bold gui=bold guifg=#ffff60 guibg=NONE"),
        ("PreProc", "term=underline gui=NONE guifg=#ff80ff guibg=NONE"),
        ("Type", "term=underline gui=bold guifg=#60ff60 guibg=NONE"),
        ("Underlined", "term=underline cterm=underline gui=underline guifg=#80a0ff"),
        ("Ignore", "ctermfg=black gui=NONE guifg=bg guibg=NONE"),
        ("Added", "ctermfg=Green gui=NONE guifg=LimeGreen guibg=NONE"),
        ("Changed", "ctermfg=Blue gui=NONE guifg=DodgerBlue guibg=NONE"),
        ("Removed", "ctermfg=Red gui=NONE guifg=Red guibg=NONE"),
    ];
    pub(super) const SYNTAX_BOTH: &[(&str, &str)] = &[
        ("Error", "term=reverse gui=NONE guifg=White guibg=Red"),
        ("Todo", "term=standout gui=NONE guifg=Blue guibg=Yellow"),
        ("Bold", "term=bold cterm=bold gui=bold guifg=NONE guibg=NONE"),
        ("Italic", "term=italic cterm=italic gui=italic guifg=NONE guibg=NONE"),
        ("BoldItalic", "term=bold,italic cterm=bold,italic gui=bold,italic guifg=NONE guibg=NONE"),
    ];
    pub(super) const SYNTAX_LINKS: &[(&str, &str)] = &[
        ("String", "Constant"),
        ("Character", "Constant"),
        ("Number", "Constant"),
        ("Boolean", "Constant"),
        ("Float", "Number"),
        ("Function", "Identifier"),
        ("Conditional", "Statement"),
        ("Repeat", "Statement"),
        ("Label", "Statement"),
        ("Operator", "Statement"),
        ("Keyword", "Statement"),
        ("Exception", "Statement"),
        ("Include", "PreProc"),
        ("Define", "PreProc"),
        ("Macro", "PreProc"),
        ("PreCondit", "PreProc"),
        ("StorageClass", "Type"),
        ("Structure", "Type"),
        ("Typedef", "Type"),
        ("Tag", "Special"),
        ("SpecialChar", "Special"),
        ("Delimiter", "Special"),
        ("SpecialComment", "Special"),
        ("Debug", "Special"),
    ];
}
