use super::super::evaluate_colorscheme;
use super::*;

fn evaluate(source: &str) -> VimHighlights {
    let (highlights, warnings) = evaluate_colorscheme("test.vim", source.as_bytes()).unwrap();
    assert!(warnings.is_empty(), "{warnings:?}");
    highlights
}

fn gui(highlights: &VimHighlights, name: &str) -> (Option<VimColor>, Option<VimColor>, u16) {
    let resolved = highlights.resolve(name).unwrap();
    (resolved.gui.foreground, resolved.gui.background, resolved.gui.flags)
}

fn rgb(value: u32) -> Option<VimColor> {
    Some(rgb_color(value))
}

#[test]
fn defaults_match_gvim_light_state_and_follow_syncolor_links() {
    let highlights = evaluate("");
    assert_eq!(highlights.background(), VimBackground::Light);
    assert_eq!(gui(&highlights, "Comment"), (rgb(0x0000ff), None, 0));
    assert_eq!(gui(&highlights, "Keyword"), (rgb(0xa52a2a), None, flags::BOLD));
    assert_eq!(highlights.resolve("Float").unwrap().group, "Constant");
    assert_eq!(gui(&highlights, "Search"), (None, rgb(0xffff00), 0));
    assert_eq!(gui(&highlights, "StatusLine"), (None, None, flags::BOLD | flags::REVERSE));
    // Cursor uses Normal's colors, which gVim leaves to the platform.
    assert_eq!(gui(&highlights, "Cursor"), (Some(VimColor::Background), Some(VimColor::Foreground), 0));
    assert!(!highlights.defined_by_scheme("Comment"));
}

#[test]
fn highlight_commands_merge_unlink_clear_and_respect_link_precedence() {
    // Each expectation was observed with Vim 9.1 loading the same commands.
    let highlights = evaluate(
        "hi Search guibg=red\nhi clear\nhi Statement guifg=red\nhi String guifg=#123456\n\
         hi link Comment Search\nhi clear Identifier\nhi! link Type Search\nhi Type guifg=#00ff00\n\
         hi def link Foo Comment\nhi Foo guifg=#111111\nhi clear Foo\nhi Bar NONE\n\
         hi Normal guifg=#eeeeee guibg='dark blue'\nhi Ignore guifg=bg\n",
    );
    assert_eq!(gui(&highlights, "Search"), (None, rgb(0xffff00), 0));
    assert_eq!(gui(&highlights, "Statement"), (rgb(0xff0000), None, flags::BOLD));
    assert_eq!(highlights.resolve("String").unwrap().group, "String");
    assert_eq!(gui(&highlights, "Comment"), (rgb(0x0000ff), None, 0));
    assert_eq!(gui(&highlights, "Identifier"), (None, None, 0));
    assert_eq!(highlights.resolve("Function").unwrap().group, "Identifier");
    assert_eq!(gui(&highlights, "Type"), (rgb(0x00ff00), None, flags::BOLD));
    assert_eq!(highlights.resolve("Foo").unwrap().group, "Comment");
    assert_eq!(highlights.resolve("Bar").unwrap().gui, VimGuiAttributes::default());
    assert_eq!(gui(&highlights, "Ignore"), (rgb(0x00008b), None, 0));
    assert!(highlights.defined_by_scheme("Statement") && !highlights.defined_by_scheme("Constant"));
}

#[test]
fn background_branches_and_reloads_follow_vim_recursion_guard() {
    let dark = evaluate(
        "set background=dark\nhi clear\nlet g:colors_name = 'x'\n\
         if &background == 'dark'\n  hi Normal guifg=#c0c0c0 guibg=#000000\nelse\n  hi Normal guibg=white\nendif\n\
         hi Statement guifg=#123456\nset bg=light\n",
    );
    // Loading a named scheme does not reload defaults when it sets 'background'.
    assert_eq!(dark.background(), VimBackground::Light);
    assert_eq!(gui(&dark, "Normal"), (rgb(0xc0c0c0), rgb(0x000000), 0));
    assert_eq!(gui(&dark, "LineNr"), (rgb(0xffff00), None, 0));
    assert_eq!(gui(&dark, "Statement"), (rgb(0x123456), None, flags::BOLD));
    let unnamed = evaluate("hi Search guibg=red\nhi Statement guifg=red\nset bg=dark\n");
    // Without a name, 'background' restores defaults the scheme left alone and
    // reapplies syncolor.vim, as Vim does.
    assert_eq!(gui(&unnamed, "Search"), (None, rgb(0xff0000), 0));
    assert_eq!(gui(&unnamed, "Statement"), (rgb(0xffff60), None, flags::BOLD));
    assert_eq!(gui(&unnamed, "LineNr"), (rgb(0xffff00), None, 0));
}

#[test]
fn helper_functions_dictionaries_and_execute_generate_highlights() {
    let highlights = evaluate(concat!(
        "hi clear\nif exists('syntax_on')\n  syntax reset\nendif\n",
        "let s:palette = {'bg': ['#1d2021', 234], 'fg': ['#ebdbb2', 223], 'red': ['#fb4934', 167]}\n",
        "let s:bold = has('gui_running') ? 'bold,' : ''\n",
        "function! s:HL(group, fg, ...)\n",
        "  let bg = a:0 >= 1 ? a:1 : ['NONE', 'NONE']\n",
        "  let emphasis = a:0 >= 2 ? a:2 : 'NONE,'\n",
        "  let parts = ['hi', a:group, 'guifg=' . a:fg[0], 'guibg=' . bg[0], 'gui=' . emphasis[:-2]]\n",
        "  execute join(parts, ' ')\n",
        "endfunction\n",
        "function! s:Literal()\n  hi Todo guifg=#abcdef\nendfunction\n",
        "call s:HL('Normal', s:palette.fg, s:palette.bg)\n",
        "call s:HL('Comment', s:palette.red, ['NONE', 'NONE'], s:bold)\n",
        "call s:Literal()\n",
        "exe 'hi Visual guibg=' . s:palette['red'][0]\n",
        "for group in ['Keyword', 'Conditional']\n  exe 'hi! link' group 'Comment'\nendfor\n",
        "if (has('termguicolors') && &termguicolors) || &t_Co >= 256\n  hi Title guifg=#010203\nendif\n",
        "if has('nvim')\n  lua require('x').setup()\nendif\n",
    ));
    assert_eq!(gui(&highlights, "Normal"), (rgb(0xebdbb2), rgb(0x1d2021), 0));
    assert_eq!(gui(&highlights, "Comment"), (rgb(0xfb4934), None, flags::BOLD));
    assert_eq!(gui(&highlights, "Todo").0, rgb(0xabcdef));
    assert_eq!(gui(&highlights, "Visual").1, rgb(0xfb4934));
    assert_eq!(highlights.resolve("Conditional").unwrap().group, "Comment");
    assert_eq!(gui(&highlights, "Title").0, rgb(0x010203));
}

#[test]
fn unsupported_statements_are_reported_without_rejecting_the_scheme() {
    let (highlights, warnings) = evaluate_colorscheme(
        "odd.vim",
        b"hi Normal guibg=#202020\naugroup X\nhi Comment guifg=notacolor\nhi Constant guifg=#ff0000 gui=sparkly\n\
          call Missing()\ncolorscheme default\nhi String guifg=#00ff00\n",
    )
    .unwrap();
    assert_eq!(gui(&highlights, "Normal").1, rgb(0x202020));
    assert_eq!(gui(&highlights, "String").0, rgb(0x00ff00));
    // Like Vim, keys before an invalid one are applied.
    assert_eq!(gui(&highlights, "Constant").0, rgb(0xff0000));
    assert_eq!(gui(&highlights, "Comment").0, rgb(0x0000ff));
    let lines = warnings.iter().map(|warning| warning.line).collect::<Vec<_>>();
    assert_eq!(lines, [3, 4, 5, 6], "{warnings:?}");
    // Budgets still end evaluation. `range()` charges its length up front.
    let looping = b"while 1\n  let s:items = range(999999)\nendwhile\n";
    assert!(evaluate_colorscheme("loop.vim", looping).is_err());
}

#[test]
fn color_names_follow_vim_tables() {
    assert_eq!(parse_color("SeaGreen").unwrap(), rgb(0x2e8b57));
    assert_eq!(parse_color("dark blue").unwrap(), rgb(0x00008b));
    assert_eq!(parse_color("DarkYellow").unwrap(), rgb(0x8b8b00));
    assert_eq!(parse_color("Grey40").unwrap(), rgb(0x666666));
    assert_eq!(parse_color("#A0b0C0").unwrap(), rgb(0xa0b0c0));
    assert_eq!(parse_color("NONE").unwrap(), None);
    assert_eq!(parse_color("bg").unwrap(), Some(VimColor::Background));
    assert!(parse_color("#abc").is_err() && parse_color("chartreuse9").is_err());
    assert!(names::NAMES.windows(2).all(|pair| pair[0].0 < pair[1].0));
}

/// Differential check against an installed Vim. Set `VIEM_VIM_COLORS` to a
/// directory of color schemes (for example `/usr/share/vim/vim91/colors`).
#[test]
fn installed_vim_resolves_the_same_gui_colors() {
    let Ok(directory) = std::env::var("VIEM_VIM_COLORS") else { return };
    let vim = std::env::var("VIEM_VIM").unwrap_or_else(|_| "vim".into());
    let groups = [
        "Normal", "Comment", "Constant", "String", "Character", "Number", "Boolean", "Float", "Identifier",
        "Function", "Statement", "Conditional", "Repeat", "Label", "Operator", "Keyword", "Exception",
        "PreProc", "Include", "Define", "Macro", "PreCondit", "Type", "StorageClass", "Structure", "Typedef",
        "Special", "SpecialChar", "Tag", "Delimiter", "SpecialComment", "Debug", "Underlined", "Ignore",
        "Error", "Todo", "Added", "Changed", "Removed", "Title", "Visual", "Search", "IncSearch", "CurSearch",
        "StatusLine", "StatusLineNC", "LineNr", "CursorLineNr", "Cursor", "NonText", "SpecialKey",
        "DiffAdd", "DiffChange", "DiffDelete", "DiffText", "Directory", "MatchParen", "Pmenu",
    ];
    let scratch = std::env::temp_dir().join(format!("viem-colors-{}", std::process::id()));
    std::fs::create_dir_all(scratch.join("colors")).unwrap();
    let mut entries = std::fs::read_dir(&directory).unwrap().filter_map(Result::ok)
        .map(|entry| entry.path()).filter(|path| path.extension().is_some_and(|ext| ext == "vim")).collect::<Vec<_>>();
    entries.sort();
    let mut failures = Vec::new();
    for path in entries {
        let name = path.file_stem().unwrap().to_string_lossy().into_owned();
        let source = std::fs::read(&path).unwrap();
        std::fs::write(scratch.join("colors").join(path.file_name().unwrap()), &source).unwrap();
        let output = scratch.join("output.txt");
        let list = groups.iter().map(|group| format!("'{group}'")).collect::<Vec<_>>().join(",");
        let probe = format!(
            "redir! > {} | for g in [{list}] | let i = synIDtrans(hlID(g)) | echo g synIDattr(i, 'fg#', 'gui') \
             synIDattr(i, 'bg#', 'gui') synIDattr(i, 'bold', 'gui') synIDattr(i, 'italic', 'gui') \
             synIDattr(i, 'reverse', 'gui') synIDattr(i, 'underline', 'gui') | endfor | echo 'background' &bg | redir END",
            output.display()
        );
        let status = std::process::Command::new(&vim)
            .args(["-Nu", "NONE", "-i", "NONE", "-es", "--cmd", &format!("set rtp^={}", scratch.display())])
            .args(["-c", "syntax on", "-c", "set termguicolors", "-c", &format!("colorscheme {name}"), "-c", &probe, "-c", "qa!"])
            .env("TERM", "xterm-256color")
            .stdin(std::process::Stdio::null())
            .status()
            .unwrap();
        assert!(status.success());
        let expected = std::fs::read_to_string(&output).unwrap();
        let (highlights, _) = evaluate_colorscheme(&name, &source).unwrap();
        // Without Normal colors, `fg`/`bg` are the platform's black and white.
        let hex = |color: Option<VimColor>| match color {
            Some(VimColor::Rgb([r, g, b])) => format!("#{r:02x}{g:02x}{b:02x}"),
            Some(VimColor::Foreground) => "#000000".into(),
            Some(VimColor::Background) => "#ffffff".into(),
            None => String::new(),
        };
        for line in expected.lines().filter(|line| !line.is_empty()) {
            let fields = line.split(' ').collect::<Vec<_>>();
            if fields[0] == "background" {
                if fields[1] != highlights.background().as_str() {
                    failures.push(format!("{name}: background {} != {}", highlights.background().as_str(), fields[1]));
                }
                continue;
            }
            // Terminal Vim lacks gVim's compiled-in `Cursor guibg=fg guifg=bg`.
            if fields[0] == "Cursor" && fields[1..].iter().all(|field| field.is_empty()) {
                continue;
            }
            let resolved = highlights.resolve(fields[0]).map(|resolved| resolved.gui).unwrap_or_default();
            let flag = |bit: u16| if resolved.flags & bit != 0 { "1" } else { "" };
            let actual = [fields[0].to_owned(), hex(resolved.foreground), hex(resolved.background),
                flag(flags::BOLD).into(), flag(flags::ITALIC).into(), flag(flags::REVERSE).into(), flag(flags::UNDERLINE).into()].join(" ");
            if actual.to_ascii_lowercase() != line.to_ascii_lowercase() {
                failures.push(format!("{name}: viem `{actual}` vim `{line}`"));
            }
        }
    }
    let _ = std::fs::remove_dir_all(&scratch);
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}
