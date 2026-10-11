"""Scripted reproductions for BUGS.md, keyed by finding ID. See regress.py."""
from regress import case, K, T, P, SEL, W, A, EX

TBL = "| a | b |\n| - | - |\n| 1 | 2 |"


def all_ok_then(pred, msg):
    def check(steps):
        bad = [(i, s) for i, s in enumerate(steps) if not s["ok"] or s["problems"]]
        if bad:
            i, s = bad[0]
            return f"step {i} {s['act']} -> {s['result']} problems={s['problems'][:2]} src={s['source'][:100]!r}"
        if not pred(steps):
            return msg(steps) if callable(msg) else msg
        return True
    return check


def final_src(*options):
    return all_ok_then(lambda st: st[-1]["source"] in options,
                       lambda st: f"source {st[-1]['source']!r} not in {options!r}")


def no_internal(pattern=r"PANIC|InvalidRange|NotGraphemeBoundary|NotCharacterBoundary|BlockIdentityExhausted|TextDoesNotMatchLayout|PositionNotInLayout|WrongSnapshot|InvalidTextOffset|MalformedMeasurement|OutsideMaterializedCoverage"):
    return ("noproblems", pattern)


def cursor_moves(step_index):
    def check(steps):
        bad = [(i, s) for i, s in enumerate(steps) if not s["ok"] or s["problems"]]
        if bad:
            i, s = bad[0]
            return f"step {i} {s['act']} -> {s['result']} problems={s['problems'][:2]}"
        before = steps[step_index - 1]["cursor"]
        after = steps[step_index]["cursor"]
        if before == after:
            return f"cursor stuck at {after} on {steps[step_index]['act']}"
        return True
    return check


FMTS = ["Plain", "Code", "Source", "Wysiwyg"]

# ---------------------------------------------------------------- lock-ups
for f in ["Plain", "Source"]:
    case(f"C0/{f}", f, " 中文", [K("$gg=G"), K("i"), T("x"), K("<Esc>u")])
    case(f"C0b/{f}", f, " 中文\nb", [K("$gg=G"), K("ix<Esc>u")])

for f in FMTS:
    case(f"C78/{f}", f, "hello world", [W(0), A("compose", text="é", commit=True), A("compose", text="ñ", commit=True),
                                        T("x"), A("history", undo=True), T("y")])
case("C78/walk-variant", "Source", "+ plus\n+ list\n\n- minus\n- list",
     [A("select_all"), A("compose", text="中文", commit=True), A("compose", text="ñ", commit=True),
      A("paste", text=TBL), K("<Esc>u"), K("ix")])
for f in FMTS:
    case(f"C81/{f}", f, "First paragraph.\nSecond line.", [A("select_all"), A("compose_open", text="ほん"),
                                                           A("compose_end", commit=True), T("x")])
case("C60/Source", "Source", "é\nb", [K("yyGo<Esc>p"), K("ix")])
for s in ["か\nb", "中文\nb", "👩\nb", "é"]:
    case(f"C60/Source/{s!r}", "Source", s, [K("yyGo<Esc>p"), K("ix")])
case("C1", "Source", "| a | b |\n| - | - |\n| 1 | 2 |\n| a | b |\n| - | - |\n| 1 | 2 |",
     [K("i"), P(32), K("<CR>"), T("x"), T("y")])
case("C45", "Source", "- y\na", [K("GA"), T("é"), T("x")])
case("C45b", "Source", "- y\n👩‍💻a", [K("GA"), T("é"), T("x")])
case("C45c", "Source", "- a\n- b\n- c", [K("i"), SEL(3, 6), A("compose", text="é", commit=True), T("x")])
case("C48", "Wysiwyg", ">> nested", [A("table_insert", columns=2, rows=1), A("resize", width=40, height=900), K("j")])
case("C48b", "Wysiwyg", "| a | b | c |\n| :-: | --- | ---: |\n| 1 | 2 | 3 |", [A("resize", width=120, height=200),
                                                                        A("scroll", top=100000, left=0), K("G")])
case("C61a", "Wysiwyg", "| a |\n| - |\n| 1 |\n| 2 |\n| 3 |\n| 4 |",
     [A("resize", width=300, height=60), A("table_column", after=False), K("<Down>"), T("**"), K("<S-Up><WordRight><Esc>vjJ"), K("ix")],
     check=no_internal())
case("C61b", "Wysiwyg", "| a | b |\n| - | - |\n| **c** | d |", [P(2), K("vjJ"), K("<Esc>ix")], check=no_internal())
for i, s in [(0, "_ab_"), (1, "a _it_ b")]:
    off = 2 if i == 0 else 4
    case(f"C30/{s}", "Source", s, [P(off), A("char_style", style="Code"), T("x")], timeout=15)
for s, offs in [("**a *b* c**", [4, 5, 8]), ("__bold__", [3, 4, 5]), ("**[bold link](u)**", [3, 7, 11])]:
    for o in offs:
        case(f"C30/{s}@{o}", "Source", s, [K("i"), P(o), A("char_style", style="Code"), T("x")], timeout=15)
for a, b in [(3, 4), (3, 5), (4, 5)]:
    case(f"C30/__bold__sel{a}..{b}", "Source", "__bold__", [SEL(a, b), A("char_style", style="")], timeout=15)
for s in ["***both***", "*a **b** c*", "**`code in bold`**"]:
    case(f"C30/sel-all {s}", "Source", s, [A("select_all"), A("char_style", style="")], timeout=15)
case("C51", "Plain", "abcdef\nghij", [K("iq<Esc>"), T("xx"), K("u"), K("ix")], check=no_internal())
case("C51b", "Plain", "---", [K("iq<Esc>iq<Esc>"), T("~~"), A("history", undo=True), K("ix")], check=no_internal())

# ---------------------------------------------------------------- caret movement
for f in FMTS:
    case(f"C2/j/{f}", f, "the file\n\nnext line here" if f == "Wysiwyg" else "the file\nnext line here", [K("wl"), K("j")], check=cursor_moves(2))
    case(f"C2/k/{f}", f, "next line here\n\nthe file" if f == "Wysiwyg" else "next line here\nthe file", [K("Gwl"), K("k")], check=cursor_moves(3))
    case(f"C2/Down/{f}", f, "the file\nnext line here", [K("wl"), K("<Down>")], check=cursor_moves(2))
    case(f"C2/End/{f}", f, "the file\nnext line here", [K("wli"), K("<End>")], check=cursor_moves(3))
case("C2/Home-footnote", "Wysiwyg", "[^1]: lone footnote definition", [P(23), K("i<Home>")], check=no_internal())
case("C3", "Wysiwyg", TBL, [K("i<Right><Right>"), K("<Down>")], width=70, check=cursor_moves(3))
case("C3/normal-j", "Wysiwyg", TBL, [K("ll"), K("j")], width=70, check=cursor_moves(2))
case("C3/list", "Wysiwyg", "- " + TBL.replace("\n", "\n  "), [K("i<Right><Right>"), K("<Down>")], width=70, check=cursor_moves(3))
WIDE = "| one | two | three | four | five | six |\n|---|---|---|---|---|---|\n| 1 | 2 | 3 | 4 | 5 | 6 |\n| alpha | beta | gamma | delta | epsilon | zeta |"
case("C21", "Wysiwyg", WIDE, [K("<Right>")] * 40 + [K("<Down>"), K("<Up>")], width=300)
case("C21/fresh", "Wysiwyg", WIDE, [P(28)] + [K("<Right>")] * 20, width=300)
WIDE2 = "para\n\n| a | b | c | d |\n|---|---|---|---|\n| 1 | 2 | 3 | 4 |"
for k in ["<C-v>I", "<C-v>A", "<C-v>d", "<C-v>jI"]:
    case(f"C21b/{k}", "Wysiwyg", WIDE2, [K(k), T("x") if "I" in k or "A" in k else [], K("<Esc>")], width=120)
case("C22/WordRight", "Wysiwyg", "| | |\n|-|-|\n| | |", [P(1), K("<WordRight>")])
case("C22/WordLeft", "Wysiwyg", "| | |\n|-|-|\n| | |", [P(3), K("<WordLeft>")])
case("C22/WordLeft-ins", "Wysiwyg", "| a | b |\n| - | - |\n|  |  |", [K("i"), P(5), K("<WordLeft>"), K("<WordLeft>")])

# ---------------------------------------------------------------- WYSIWYG typing
case("C19", "Wysiwyg", "- first item\n- second item", [K("GA<CR><Tab>"), T("x")], check=final_src("- first item\n- second item\n  - x", "- first item\n- second item\n    - x"))
for f in ["Wysiwyg", "Source"]:
    case(f"C19b/{f}", f, "- a\n- \n- c", [K("j"), K("A"), K("<Tab>"), T("x")])
case("C19c/indent-action", "Source", "- a\n- \n- c", [K("jA"), A("indent", unindent=False), T("x")])
case("C5", "Wysiwyg", "a\nb\nc", [K("i"), P(3), K("<ShiftEnter>"), T("x")])
case("C5/up", "Wysiwyg", "a\nb\nc", [K("i"), P(3, down=False), K("<ShiftEnter>"), T("x")])
case("C5b", "Wysiwyg", "a\nb", [K("i"), P(1), K("<ShiftEnter>"), T("x")])
for s in ["#", "##", "-", "---", "***", "___", "\t", " ", "Setext\n===", "[ref]: /url \"title\"", "para\n" + TBL]:
    case(f"C5/{s!r}", "Wysiwyg", s, [K("i"), K("<ShiftEnter>"), T("x")])
for fence in ["```\nx\n```", "```rust\nx\n```", "``` a b\nx\n```"]:
    case(f"C4/{fence!r}", "Wysiwyg", fence, [K("i"), P(1), K("<BS>"), T("y")])
case("C4/cc", "Wysiwyg", "```\ncode\n```", [K("cc"), T("x"), K("<Esc>")], check=final_src("```\nx\n```"))
case("C4/cc2", "Wysiwyg", "text\n```\ncode\n```\ntext", [K("jcc"), T("x"), K("<Esc>")], check=final_src("text\n```\nx\n```\ntext"))
for o in [1, 2, 3]:
    case(f"C6/enter@{o}", "Wysiwyg", "`code`", [K("i"), P(o), K("<CR>"), T("x")])
case("C6/span-nl-enter", "Wysiwyg", "`a\nb`", [K("i"), P(1), K("<CR>")])
for ch in [">", "#", "*", "[", "_", "-", "`", "&", "<", "~", "!"]:
    case(f"C6/span-nl-type{ch}", "Wysiwyg", "`a\nb`", [K("i"), P(2), T(ch)])
for s in ["*it*", "_it_", "*a\nb*"]:
    case(f"C7/{s!r}", "Wysiwyg", s, [K("i"), P(1), K("<CR><BS>")], check=final_src(s))
case("C7/\\na", "Wysiwyg", "\na", [K("i"), P(1), K("<CR><BS>")])
case("C8/a", "Wysiwyg", "- a\n  b", [K("i"), P(3), K("<BS>")])
case("C8/b", "Wysiwyg", "- ab\n  c", [K("i"), P(3), K("<Del>")])
case("C8/c", "Wysiwyg", "- ab\n  c\n- d", [K("i"), P(6), K("<BS>")])
case("C9/bs", "Wysiwyg", "**a\n\nb**", [K("i"), P(4), K("<BS>")])
case("C9/del", "Wysiwyg", "**a\n\nb**", [K("i"), P(3), K("<Del>")])
case("C9/hr", "Wysiwyg", "a\n***\nb", [K("i"), P(3), K("<BS><BS>")])
case("C9/setext-bs", "Wysiwyg", "a\n=\n\nb", [K("i"), P(1), K("<BS>")])
case("C9/setext-del", "Wysiwyg", "a\n=\n\nb", [K("i"), P(0), K("<Del>")])
case("C10/bs", "Wysiwyg", "<https://auto.link>", [K("i"), P(6), K("<BS>")])
case("C10/del", "Wysiwyg", "<https://auto.link>", [K("i"), P(5), K("<Del>")])
case("C10/mailto", "Wysiwyg", "<mailto:a@b.c>", [K("i"), P(6), T("-"), T(" ")])
for ch in ["x", "`", "&", "<", "[", "#", "*", "|", "é"]:
    case(f"C11/{ch}", "Wysiwyg", "> a\n> b", [K("i"), P(2, down=False), T(ch)])
case("C11/nested", "Wysiwyg", "> > nested\n> back", [K("i"), P(7, down=False), T("x")])
for s in [" ", "\t", "   \n  \n"]:
    for op in [T("x"), K("<Tab>"), K("<BackTab>"), A("paste", text="p"), A("compose", text="é", commit=True)]:
        case(f"C12/{s!r}/{op[0].get('key', op[0].get('text', op[0]['op']))}", "Wysiwyg", s, [K("A"), op, K("<Esc>i"), T("y")])
for s in ["abc\n\nend\n  ", "a\n   "]:
    case(f"C12/{s!r}", "Wysiwyg", s, [K("GA"), T("x")])
for op in [T(" "), K("<Tab>"), K("<BackTab>")]:
    case(f"C13/{op[0]}", "Wysiwyg", ">", [K("i"), op, T("x")])
REFS = "[ref]: /url \"title\"\n[other]: <x y>"
for op in [T("x"), K("<Tab>"), K("<ShiftEnter>"), A("paste", text="p"), K("<Del>"), K("<BS>x")]:
    case(f"C14/{op[0]}", "Wysiwyg", REFS, [K("i"), op, T("y")])
case("C15", "Wysiwyg", TBL + "\n\ntext after", [K("i"), P(7), K("<CR>"), T("x")])
case("C15/two-tables", "Wysiwyg", TBL + "\n\n" + TBL, [K("i"), P(7), K("<CR>"), T("x")])
case("C15/list", "Wysiwyg", "1. a\n\n   | x | y |\n   | - | - |\n   | 1 | 2 |\n\n2. b", [K("i"), P(7), K("<CR>"), T("x")])
for s in ["\na", "\nab"]:
    case(f"C16/{s!r}", "Wysiwyg", s, [K("i"), P(1), K("<CR>"), T("x")])
case("C17/image", "Wysiwyg", "![img](local.png)", [K("A"), A("compose", text="é", commit=True)])
case("C17/image-cancel", "Wysiwyg", "![img](local.png)", [K("A"), A("compose", text="é", commit=False)])
case("C17/image-mid", "Wysiwyg", "a ![img](i.png) b", [K("i"), P(5), A("compose", text="é", commit=True)])
for s, off in [("**`code in bold`**", 3), ("<ins>unclosed", 3), ("<mailto:a@b.c>", 4)]:
    case(f"C17/pressandhold/{s}", "Wysiwyg", s, [K("i"), P(off), T("e"), A("compose_replace_prev", text="é")])
for s in ["<!-- comment -->", "<!-- unclosed comment", "<script>alert(1)</script>"]:
    for op in [T(" "), K("<Tab>"), K("<BackTab>")]:
        case(f"C18/{s}/{op[0]}", "Wysiwyg", s, [K("i"), op])
for s in ["</closing-only>", "<unknown-tag attr>"]:
    case(f"C18/{s}/bslash", "Wysiwyg", s, [K("i"), T("\\")])
for ch in ["&", "<"]:
    case(f"C18/script@17{ch}", "Wysiwyg", "<script>alert(1)</script>", [K("i"), P(17), T(ch)])
for s in ["# Heading #", "# Heading ###   "]:
    for seq in ["# ", "- ", "> "]:
        case(f"C20/{s!r}/{seq!r}", "Wysiwyg", s, [K("A"), T(seq[0]), T(seq[1]), T("x")])
case("C20/quoted", "Wysiwyg", "> # quoted heading", [K("A"), T("#"), T(" "), T("x")])
case("C46/~strike~", "Wysiwyg", "~strike~", [K("i"), SEL(0, 1), T("x")])
case("C46/a ~strike~ b", "Wysiwyg", "a ~strike~ b", [K("i"), SEL(2, 3), T("x")])
case("C46/enter-\\na", "Wysiwyg", "\na", [K("i"), SEL(1, 2), K("<CR>")])
case("C46/enter-heading", "Wysiwyg", "# Heading #", [K("i"), SEL(2, 4), K("<CR>")])
case("C46/type-setext", "Wysiwyg", "Setext\n===", [K("i"), SEL(1, 3), T("x")])
case("C46/paste-heading", "Wysiwyg", "# Heading #", [K("i"), SEL(2, 4), A("paste", text="p")])
case("C46/paste-ref", "Wysiwyg", REFS, [K("i"), SEL(1, 3), A("paste", text="p")])
case("C46/paste-ws", "Wysiwyg", "a\n\n   \n  \n", [K("i"), SEL(0, 1), A("paste", text="p")])
for s in ["# abc", "# abc def"]:
    n = len(s) - 2
    for seq in ["*", "-", "#", "> "]:
        case(f"C52/{s}/{seq!r}", "Wysiwyg", s, [K("i"), SEL(0, n), T(seq), T("x")])
case("C52/quote", "Wysiwyg", "> abc", [K("i"), SEL(0, 3), T("-"), T("x")])
for s in ["- # ", "- #", "1. # ", "- ## "]:
    case(f"C54/{s!r}", "Wysiwyg", s, [K("A"), T("x")])
case("C54/cc", "Wysiwyg", "- # h", [K("cc"), T("x"), K("<Esc>")], check=final_src("- # x"))
MC = "<!--\ncomment\n\nspanning\n-->"
for op in [T("```"), T("#"), T("<"), K("<Tab>"), K("<ShiftEnter>x")]:
    case(f"C57/comment/{op[0]}", "Wysiwyg", MC, [K("i"), op, K("<Esc>i"), T("y")])
PRE = "<pre>\npre block\n\n</pre>\n\nafter"
for op in [T("```"), T("#"), T("<"), K("<Tab>"), K("<ShiftEnter>x")]:
    case(f"C57/pre/{op[0]}", "Wysiwyg", PRE, [K("i"), P(11), op, K("<Esc>i"), T("y")])
case("C57/emptyitem-up", "Wysiwyg", "a\n- \nb", [K("i"), P(1, down=False), K("<ShiftEnter>"), T("x"), K("<Esc>i"), T("y")])
case("C66/fence-enter", "Wysiwyg", "- > ```\n  > code\n  > ```", [K("i"), P(0), K("<CR>")])
case("C66/fence-bs", "Wysiwyg", "- > ```\n  > code\n  > ```", [K("i"), P(0), K("<BS>")])
case("C66/source-tab", "Source", "> - > - > a\n> - > - > b", [K("i"), P(12), K("<Tab>"), T("x")])
case("C66/comment-enter", "Wysiwyg", MC, [K("i"), P(4), K("<CR>")])
case("C66/comment-paste", "Wysiwyg", MC, [K("i"), P(4), A("paste", text="x\ny\n\nz")])

# ---------------------------------------------------------------- Vim commands
for f in ["Plain", "Source", "Code"]:
    case(f"C50/cc/{f}", f, "a\nb\n", [K("Gcc"), T("x"), K("<Esc>")])
    case(f"C50/S/{f}", f, "a\nb\n", [K("GS"), T("x"), K("<Esc>")])
    case(f"C50/Vc/{f}", f, "a\nb\n", [K("GVc"), T("x"), K("<Esc>")])
    case(f"C50/Vc-tab/{f}", f, "a\n\tb\n", [K("GVc"), T("x"), K("<Esc>")])
for f, fn in [("Code", "x.rs"), ("Plain", None), ("Source", None)]:
    for k in ["<S-Right>", "<S-Left>", "<S-Up>", "<S-Down>", "<S-Home>", "<S-End>"]:
        case(f"C62/enter/{f}/{k}", f, "fn f() {\n    foo();\n}", [K("jA<CR>"), K(k)], filename=fn)
    case(f"C62/o/{f}", f, "fn f() {\n    foo();\n}", [K("jo"), K("<S-Up>")], filename=fn)
for s in ["Setext\n===", "Setext\n---"]:
    for k in ["dd", "D", "ccx<Esc>", "Sx<Esc>", "Cx<Esc>", "sx<Esc>", "cwx<Esc>", "ciwx<Esc>", "dw", "de", "Vd", "dip", "dap",
              "das", "d}", "dG", "dgg", ":d<CR>", "dd."]:
        case(f"C32/{s!r}/{k}", "Wysiwyg", s, [K(k)])
    case(f"C32/{s!r}/Vjd", "Wysiwyg", s + "\n\nnext", [K("Vjd")])
for k in ["ox<Esc>", "A<CR>x<Esc>", "yyp", "yyP", "r<CR>"]:
    case(f"C32/3line/{k}", "Wysiwyg", "Setext\nline two\n===", [K(k)])
case("C32/dd-following", "Wysiwyg", "Title\n=====\n\nBody text.", [K("dd")],
     check=all_ok_then(lambda st: "#" not in st[-1]["source"] and "Body text." in st[-1]["source"],
                       lambda st: f"dd moved heading style: {st[-1]['source']!r}"))
case("C32/cc-following", "Wysiwyg", "Title\n=====\n\nBody text.", [K("cc"), T("x"), K("<Esc>")],
     check=all_ok_then(lambda st: st[-1]["text"].startswith("x"), lambda st: f"cc result {st[-1]['source']!r}"))
for cmd in ["m+1", "t.", "sort"]:
    case(f"C33/{cmd}", "Code", "fn main() {\n    println!(\"hi\");\n}\n", [A("wait_syntax"), EX(cmd)], filename="x.rs")
    case(f"C33/abc/{cmd}", "Code", "a\nb\nc", [A("wait_syntax"), EX(cmd)], filename="x.rs")
for s in ["\n", "\na", "a<br>b", "Setext\n===\n\nx"]:
    case(f"C34/t./{s!r}", "Wysiwyg", s, [EX("t.")])
for s in ["1) a\n2) b", "a<br>b\n\nc", "a\n=\n\nb"]:
    case(f"C34/m+1/{s!r}", "Wysiwyg", s, [EX("m+1")])
FENCE = "```rust\nfn x() {}\n```\n\npara"
case("C34/fence-t.", "Wysiwyg", FENCE, [K("G"), EX("t.")])
case("C34/fence-m-2", "Wysiwyg", FENCE, [K("G"), EX("m-2")])
case("C34/fence-t.-code", "Wysiwyg", FENCE, [EX("t.")])
case("C34/list-m-2", "Wysiwyg", "# Title\n\nSome **bold** text.\n\n- a\n- b", [K("G"), EX("m-2")])
case("C35", "Source", TBL, [P(21), K("ox<Esc>")])
case("C35/quote", "Source", "> " + TBL.replace("\n", "\n> "), [K("G"), K("ox<Esc>")])
case("C36", "Wysiwyg", "# A\n## B\n### C\n#### D\n##### E\n###### F", [K("jOx<Esc>")])
case("C36/\\na", "Wysiwyg", "\na", [K("GOx<Esc>")])
case("C36/refs", "Wysiwyg", REFS, [K("GOx<Esc>")])
for k in ["J", "gJ", "2dd"]:
    case(f"C37/{k}", "Wysiwyg", TBL, [K(k)], check=no_internal())
case("C38/fence", "Wysiwyg", "```\ncode\n```", [K("ddp")])
case("C38/boldlink", "Wysiwyg", "**[bold link](u)**\n\nx", [K("dd")],
     check=all_ok_then(lambda st: "[](" not in st[-1]["source"], lambda st: f"dd left {st[-1]['source']!r}"))
for k in ["yyP", "yyp", "ddP"]:
    case(f"C38/atx-closing/{k}", "Wysiwyg", "# Heading #", [K(k)])
for s in ["``code with ` tick``", "\na", "`**not bold**`"]:
    case(f"C38/yyp/{s!r}", "Wysiwyg", s, [K("yyp")])
for s in [REFS, "[ref]\n\n[ref]: /url", "[a][ref]\n\n[ref]: https://example.com"]:
    case(f"C40/{s!r}", "Wysiwyg", s, [K(">>")], check=no_internal(r"PANIC|VerificationFailed|InvalidRange"))
for s, want in [("> # h", "> # x"), ("1. > heading", "1. > x"), ("- > # h", "- > # x")]:
    case(f"C55/cc/{s}", "Wysiwyg", s, [K("ccx<Esc>")], check=final_src(want))
case("C55/C", "Wysiwyg", "> - > - > a\n> - > - > b", [K("Cx<Esc>")], check=final_src("> - > - > x\n> - > - > b"))
D55 = "1. > # heading\n   > text\n2. b"
for k in ["ccx<Esc>", "Sx<Esc>", "ciwx<Esc>", "Vjcx<Esc>", "Ox<Esc>"]:
    case(f"C55/{k}", "Wysiwyg", D55, [K(k)])
for s, want in [("- # heading", "- # x"), ("1. > # heading", "1. > # x"), ("1. > heading", "1. > x")]:
    case(f"C55/word/{s}", "Wysiwyg", s, [K("i"), W(1), T("x")], check=final_src(want))
case("C55/parse", "Wysiwyg", "> # > x", [], check=lambda st: True)
case("C55/parse2", "Wysiwyg", "> # > x", [K("l")], check=all_ok_then(lambda st: st[-1]["text"] == "> x", lambda st: f"text {st[-1]['text']!r}, GitHub shows '> x'"))
BIG = "alpha line\nbeta line\n" * 2000
for f in ["Plain", "Code", "Source"]:
    case(f"C59/{f}", f, BIG, [EX("g/al/d")], timeout=60,
         check=all_ok_then(lambda st: st[-1]["secs"] < 2.0, lambda st: f"took {st[-1]['secs']:.1f}s"))
case("C59/subst", "Plain", BIG, [EX("g/al/s/a/b/")], timeout=60,
     check=all_ok_then(lambda st: st[-1]["secs"] < 2.0, lambda st: f"took {st[-1]['secs']:.1f}s"))
case("C59/wysiwyg", "Wysiwyg", "alpha line\n\nbeta line\n\n" * 1000, [EX("g/al/d")], timeout=60,
     check=all_ok_then(lambda st: st[-1]["secs"] < 2.0 and "alpha" not in st[-1]["source"],
                       lambda st: f"took {st[-1]['secs']:.1f}s, alpha left={('alpha' in st[-1]['source'])}"))
for f in ["Plain", "Code", "Source"]:
    case(f"C70/dd/{f}", f, "a\nb\nc\n", [K("Gkdd")] if False else [K("jjdd")], check=final_src("a\nb\n"))
    case(f"C70/dj/{f}", f, "a\nb\nc\n", [K("jdj")], check=final_src("a\n"))
    case(f"C70/ddp/{f}", f, "a\nb\nc\n", [K("jddp")], check=final_src("a\nc\nb\n"))
    case(f"C70/yyp/{f}", f, "a\nb\nc\n", [K("jjyyp")], check=final_src("a\nb\nc\nc\n"))
    case(f"C70/cc/{f}", f, "a\nb\nc\n", [K("jjccx<Esc>")], check=final_src("a\nb\nx\n"))
case("C70/2cc/Code", "Code", "if a {\n    b();\n}\n", [K("j2ccx<Esc>")], filename="x.rs", check=final_src("if a {\n    x\n"))
case("C70/cc-indent/Plain", "Plain", "class A:\n    pass\n", [K("jccx<Esc>")], check=final_src("class A:\n    x\n"))
for f in ["Plain", "Code"]:
    case(f"C71/diw-indent/{f}", f, "x\n   b\n", [K("jdiw")], check=final_src("x\nb\n"))
    case(f"C71/daw-indent/{f}", f, "x\n   b\n", [K("jdaw")], check=final_src("x\n\n"))
    case(f"C71/diw-trail/{f}", f, "x  \n   b\n", [K("$diw")], check=final_src("x\n   b\n"))
    case(f"C71/diw-empty/{f}", f, "x\n\ny\n", [K("jdiw")], check=final_src("x\n\ny\n"))
    case(f"C71/viwd-empty/{f}", f, "x\n\ny\n", [K("jviwd")], check=final_src("x\ny\n"))
    case(f"C71/ciw-indent/{f}", f, "fn main() {\n    println!(\"hi\");\n}\n", [K("jciwx<Esc>")], check=final_src("fn main() {\nxprintln!(\"hi\");\n}\n"))
    case(f"C71/yiwP/{f}", f, "fn main() {\n    println!(\"hi\");\n}\n", [K("jyiwP")], check=final_src("fn main() {\n        println!(\"hi\");\n}\n"))
    case(f"C71/ciw-blank/{f}", f, "a\n\n\nb\n", [K("jjciwx<Esc>")], check=final_src("a\n\nx\nb\n"))
    case(f"C71/cw-blank/{f}", f, "a\n\n\nb\n", [K("jjcwx<Esc>")], check=final_src("a\n\nx\nb\n"))
case("C71/source", "Source", "x\n\n   y", [K("Gdiw")], check=all_ok_then(lambda st: "\n\n" in st[-1]["source"], lambda st: f"merged: {st[-1]['source']!r}"))
case("C72/Code", "Code", "if a {\n    b();\n}\n", [K("jSx<Esc>")], filename="x.rs", check=final_src("if a {\n    x\n}\n"))
case("C72/Plain", "Plain", "  a\n  b\n", [K("jccx<Esc>")], check=final_src("  a\n  x\n"))
case("C73/gUU", "Source", "a\n\nb", [K("gUU")], check=final_src("A\n\nb"))
case("C73/VU", "Source", "a\n\nb", [K("VU")], check=final_src("A\n\nb"))
case("C73/guu-heading", "Source", "# **Bold** heading\n\ntext", [K("guu")], check=final_src("# **bold** heading\n\ntext"))
case("C73/wysiwyg-soft", "Wysiwyg", "x\ny\n\nb", [K("gUU")], check=final_src("X\nY\n\nb"))
case("C73/listed-code", "Wysiwyg", "- a\n  ```\n  code\n  ```\n- b", [P(2), K("gUU")], check=final_src("- a\n  ```\n  CODE\n  ```\n- b"))
case("C73/listed-code-g~~", "Wysiwyg", "- a\n  ```\n  code\n  ```\n- b", [P(2), K("g~~")], check=final_src("- a\n  ```\n  CODE\n  ```\n- b"))
case("C74/>>", "Plain", "a", [K(">>")], check=final_src("  a"))
case("C74/V>.", "Plain", "a", [K("V>.")], check=final_src("    a"))
case("C74/>>tab", "Plain", "\tb", [K(">>")], check=final_src("    b"))
case("C74/<<tabs", "Plain", "\t\tb", [K("<<")], check=final_src("  b"))
case("C74/==", "Plain", "  a\n      b", [K("j==")], check=final_src("  a\n  b"))
LONG = "The quick brown fox jumps over the lazy dog and keeps running far away."
case("C74/wrapped-row", "Plain", LONG + "\nnext", [K("gj>>")], width=300,
     check=all_ok_then(lambda st: "     " not in st[-1]["source"].split("\n")[0].lstrip(), lambda st: f"mid-line indent: {st[-1]['source']!r}"),
     note="Modified items keep displayed-row policy")
for k, want in [("e", 9), ("E", 9), ("2e", 15)]:
    case(f"C75/{k}", "Plain", "alpha beta gamma", [K("4l"), K(k)], check=all_ok_then(lambda st, w=want: st[-1]["cursor"] == w, lambda st, w=want: f"cursor {st[-1]['cursor']} != {w}"))
case("C75/de", "Plain", "alpha beta gamma", [K("4lde")], check=final_src("alph gamma"))
case("C76/x", "Wysiwyg", "| a |\n| --- |", [K("x")], check=all_ok_then(lambda st: st[-1]["source"].count("|") >= 4, lambda st: f"table gone: {st[-1]['source']!r}"))
case("C76/A-BS", "Wysiwyg", "| a |\n| --- |", [K("A<BS>")], check=all_ok_then(lambda st: st[-1]["source"].count("|") >= 4, lambda st: f"table gone: {st[-1]['source']!r}"))
case("C76/ciw", "Wysiwyg", "| abc |\n| --- |", [K("ciwz<Esc>")], check=final_src("| z |\n| --- |"))
case("C76/text", "Wysiwyg", "|a|\n|-|\n\ntext", [K("x")], check=all_ok_then(lambda st: st[-1]["source"].count("|") >= 4, lambda st: f"table gone: {st[-1]['source']!r}"))
case("C76/~", "Wysiwyg", "| a |\n| --- |", [K("~")], check=final_src("| A |\n| --- |"))
case("C76/rz", "Wysiwyg", "| a |\n| --- |", [K("rz")], check=final_src("| z |\n| --- |"))
for f in ["Plain", "Code"]:
    case(f"C77/dw-eol/{f}", f, "one two\nthree\n", [K("wdw")], check=final_src("one \nthree\n"))
    case(f"C77/ywP/{f}", f, "one two\nthree\n", [K("wywP")], check=final_src("one twotwo\nthree\n"))
    case(f"C77/2dw/{f}", f, "one two\nthree\n", [K("w2dw")], check=final_src("one \n"))
    case(f"C77/db/{f}", f, "one two\n  three\n", [K("jdb")], check=final_src("one \n  three\n"))
    case(f"C77/dl/{f}", f, "alpha\n", [K("$dl")], check=final_src("alph\n"))
    case(f"C77/ylp/{f}", f, "alpha\n", [K("$ylp")], check=final_src("alphaa\n"))
    case(f"C77/cl/{f}", f, "alpha\n", [K("$clx<Esc>")], check=final_src("alphx\n"))
    case(f"C77/r<CR>/{f}", f, "alpha beta\n", [K("4lr<CR>")], check=final_src("alph\nbeta\n"))
    case(f"C77/v$d/{f}", f, "one.\ntwo\n", [K("v$d")], check=final_src("two\n"))
    case(f"C77/dip/{f}", f, "p1 a.\np1 b.\n\np2\n", [K("dip")], check=final_src("\np2\n"))
    case(f"C77/p-empty/{f}", f, "ab\n\ncd\n", [K("yljp")], check=final_src("ab\na\ncd\n"))
    case(f"C77/xp/{f}", f, "{\nab\n", [K("xp")], check=final_src("{\nab\n"))
    case(f"C77/I-blank/{f}", f, "a\n   \n", [K("jIx<Esc>")], check=final_src("a\n   x\n"))
    case(f"C77/I-BS/{f}", f, "a\n   \n", [K("jI<BS><BS><Esc>")], check=final_src("a\n\n"))
    case(f"C77/~x/{f}", f, "ab\ncd\n", [K("l~x")], check=final_src("a\ncd\n"))
    case(f"C77/~./{f}", f, "ab\ncd\n", [K("l~.")], check=final_src("ab\ncd\n"))
    case(f"C77/w-empty/{f}", f, "ab\n\ncd\n", [K("w")], check=all_ok_then(lambda st: st[-1]["cursor"] == 3, lambda st: f"w landed at {st[-1]['cursor']}"))
P79 = "First paragraph.\nSecond line."
case("C79/cc-last-row", "Plain", P79, [K("gjccx<Esc>")], width=140, check=all_ok_then(lambda st: "\nSecond line." in st[-1]["source"], lambda st: f"merged: {st[-1]['source']!r}"))
case("C79/dd-last-row", "Plain", P79, [K("gjdd")], width=140, check=all_ok_then(lambda st: "\nSecond line." in st[-1]["source"] or st[-1]["source"].startswith("Second"), lambda st: f"merged: {st[-1]['source']!r}"))
case("C79/yyP-last-row", "Plain", P79, [K("gjyyP")], width=140, check=all_ok_then(lambda st: "paraparagraph" not in st[-1]["source"], lambda st: f"split word: {st[-1]['source']!r}"))
case("C79/yyp-interior", "Plain", LONG + "\nsecond line", [K("gjyyp")], width=300, check=all_ok_then(lambda st: "oover" not in st[-1]["source"], lambda st: f"split word: {st[-1]['source']!r}"))
for f in ["Plain", "Code"]:
    case(f"C80/g^$d/{f}", f, "a\n\n\nb\n", [EX("g/^$/d")], check=final_src("a\nb\n"))
    case(f"C80/g^$d4/{f}", f, "a\n\n\n\nb\n", [EX("g/^$/d")], check=final_src("a\nb\n"))
    case(f"C80/v/{f}", f, "a\n\nb\n\n\nc\n", [EX("v/a/d")], check=final_src("a\n"))
for k in ["VjJ", "VJ", "vJ", "vlJ"]:
    case(f"C82/{k}", "Code", "one\ntwo\nthree\n", [K(k)], filename="x.rs", check=final_src("one two\nthree\n"))

# ---------------------------------------------------------------- formatting
for s in ["one\n\ntwo", "# Title\n\nBody text here."]:
    for st in ["strong", "emphasis"]:
        case(f"C53/all/{st}/{s!r}", "Wysiwyg", s, [A("select_all"), A("semantic", style=st, enabled=True)],
             check=all_ok_then(lambda st_: st_[-1]["changed"], "Bold on Select All did nothing"))
for s, a, b in [("one two", 0, 4), ("one two", 3, 7), ("one two three", 4, 8), ("# A\n## B", 0, 3), ("- a\n- b", 0, 3),
                ("Line one\nLine two", 1, 14), ("&amp; &lt; x", 0, 2)]:
    case(f"C53/{s!r}/{a}..{b}", "Wysiwyg", s, [K("i"), SEL(a, b), A("semantic", style="strong", enabled=True)],
         check=all_ok_then(lambda st_: st_[-1]["changed"], "not applied (presented unavailable?)"))
case("C53/source-enter-it", "Source", "*it*", [K("i"), SEL(1, 4), K("<CR>")], check=no_internal())
case("C53/source-enter-bold", "Source", "**bold**", [K("i"), SEL(1, 8), K("<CR>")], check=no_internal())
case("C53/source-pipe-del", "Source", "a | b\n- | -\n1 | 2", [K("i"), SEL(0, 14), K("<BS>")])
case("C53/source-pipe-type", "Source", "a | b\n- | -\n1 | 2", [K("i"), SEL(0, 14), T("x")])
case("C53/ins-default", "Wysiwyg", "<ins>under</ins> text", [K("i"), SEL(0, 3), A("char_style", style="")])
for s in ["trailing spaces   ", "a\\\nb", "line  \n  \nafter double space", "> a\n> b", "> > nested\n> back", "- a\n  b\n  c",
          "- a\n\n  continued\n- b", "- > quote in list", "text\n    not code (lazy)", "`a\nb`", "Setext\n===", " ", "\t",
          "a<br>b", "```\na\n\nb\n```"]:
    case(f"C41/{s!r}", "Wysiwyg", s, [A("paragraph_style", style="Heading1")])
for s, off in [("`code`", 0), ("# `code` heading", 0), ("[`code` link](u)", 0), ("``code with ` tick``", 0), ("`**not bold**`", 0),
               ("<https://auto.link>", 0), ("<mailto:a@b.c>", 0), ("trailing backslash\\", 19)]:
    for st in ["strong", "emphasis"]:
        case(f"C42/{st}/{s!r}@{off}", "Wysiwyg", s, [K("i"), P(off), A("semantic", style=st, enabled=True), T("x")])
case("C42/table-pipe", "Wysiwyg", "| `a\\|b` | c |\n| - | - |\n| 1 | 2 |", [K("i"), P(3), A("semantic", style="strong", enabled=True), T("x")])
for s in ["a  \nb", "a\\\nb", "a <br/> b <br> c", "> a\n> b", "> # h\n> text", "> > nested\n> back", "# Heading #",
          "# Heading ###   ", "Setext\n===", "- a\n  b\n  c", "text\n    lazy", " ", "` `", "```js\nx\n```", "```\nunclosed",
          "para\n" + TBL]:
    case(f"C43/{s!r}", "Wysiwyg", s, [A("list", style="bullet")])
for s in ["Setext\n===", "- a\n  - b\n    - c\n      - d", "- a\n\n  ```\n  c\n  ```", "para\n```\nc\n```"]:
    case(f"C43/quote/{s!r}", "Wysiwyg", s, [K("G"), A("quote", enabled=True)])
for s, off in [("`unclosed code", 0), ("https://bare.link.example", 8), ("<p align=\"center\">x</p>", 0),
               ("<ins>under</ins> text", 0), ("<details>\n\nx\n\n</details>", 0), ("- a\n  - b\n    - c\n      - d", 7),
               ("| `a\\|b` | c |\n| - | - |\n| 1 | 2 |", 3), ("**`code in bold`**", 3)]:
    case(f"C44/{s!r}@{off}", "Wysiwyg", s, [K("i"), P(off), A("char_style", style="Code"), T("x")])
for st in ["strong", "emphasis", "code"]:
    for off in range(0, 11):
        case(f"C44/source-strike/{st}@{off}", "Source", "~~strike~~", [K("i"), P(off), A("char_style", style="Code") if st == "code" else A("semantic", style=st, enabled=True), T("x")])
case("C31/1", "Source", "**bold**", [K("i"), P(1), A("char_style", style="Code"), T("x")], check=final_src("*`x`*bold**"))
case("C31/7", "Source", "**bold**", [K("i"), P(7), A("char_style", style="Code"), T("x")],
     check=all_ok_then(lambda st: st[-1]["source"].count("*") == 4, lambda st: f"asterisks lost {st[-1]['source']!r}"))
case("C29/list", "Source", "text before\n" + TBL, [A("list", style=None)], check=no_internal())
case("C29/para", "Source", "text before\n" + TBL, [A("paragraph_style", style="Paragraph")], check=no_internal())
for s, a, b in [("one **two** three", 2, 6), ("one *two* three", 0, 6), ("a*b*c", 0, 2)]:
    case(f"C64/{s}", "Wysiwyg", s, [K("i"), SEL(a, b), A("link_insert", text="t", destination="https://x.y")],
         check=all_ok_then(lambda st: "](https://x.y)" in st[-1]["source"] or "](<https://x.y>)" in st[-1]["source"], lambda st: f"no link: {st[-1]['source']!r}"))


def copy_paste(s, a, b, at, fmt="Wysiwyg", **kw):
    return [K("i"), SEL(a, b), K("<Copy>"), P(at), A("paste_stored")]


for s, a, b, at in [("**bold**", 0, 1, 1), ("a**b**c", 0, 1, 1), ("a\n***\nb", 0, 1, 2), ("a\nb", 0, 2, 1), ("\na", 0, 1, 0),
                    ("*it*", 0, 1, 1), ("a*b*c", 1, 2, 3), ("`a\nb`", 0, 1, 2)]:
    case(f"C65/{s!r}/{a}..{b}@{at}", "Wysiwyg", s, copy_paste(s, a, b, at))
case("C65/bold-all-end", "Wysiwyg", "**bold**", copy_paste("**bold**", 0, 4, 4),
     check=all_ok_then(lambda st: "<strong>" not in st[-1]["source"], lambda st: f"respelled {st[-1]['source']!r}"))
for s in ["- [ ] task", "- > quote in list"]:
    case(f"C65/leading-space/{s}", "Wysiwyg", s, [K("i"), A("paste", text=" x")])

# ---------------------------------------------------------------- Source view
case("C24", "Source", "| a |\n| - |\n| 1 |", [K("G"), A("table_delete_row")])
case("C24/shift-enter", "Source", TBL, [K("GA<ShiftEnter>")])
case("C25", "Source", "```\ncode\n```", [K("i"), P(1), K("<CR><BS>")], check=final_src("```\ncode\n```"))
for s in ["~~~\ncode\n~~~", "<div>\nblock\n</div>", "text\n```\ncode\n```\ntext"]:
    case(f"C25/{s!r}", "Source", s, [K("i"), P(1), K("<CR><BS>")], check=final_src(s))
case("C25/blank", "Source", "```\n\n\n```", [K("2li"), T("xy"), K("<BS><BS>")])
L26 = "1. a\n\n   | x | y |\n   | - | - |\n   | 1 | 2 |\n\n2. b"
case("C26", "Source", L26, [K("i"), P(8), K("<CR>")])
case("C27", "Source", "    a\n\n    b", [K("GA<CR><BS>")])
L39 = "- item\n\n  | a | b | c | d |\n  |---|---|---|---|\n  | 1 | 2 | 3 | 4 |\n\n- next"
for k in ["x", "X", "3x", "dw", "de", "cwz<Esc>", "sz<Esc>", "vd", "<<", "==", "i<BS>", "a<Del>"]:
    case(f"C39/{k}", "Source", L39, [P(14), K(k)])
    case(f"C39b/{k}", "Source", L26, [P(14), K(k)])
case("C67/cc", "Source", "first line\nsecond line\nthird line", [K("jccnew<Esc>")], check=final_src("first line\nnew\nthird line"))
case("C67/xxx", "Source", "a\nbb\nc", [K("jxxx")], check=final_src("a\n\nc"))
case("C67/last-of-para", "Source", "a\nbb\n\nc", [K("jccz<Esc>")], check=final_src("a\nz\n\nc"))
case("C67/A-BS", "Source", "a\nbb\n\nc", [K("jA<BS><BS><Esc>")], check=final_src("a\n\n\nc"))
case("C67/quote", "Source", "> a\n> bb\n> c", [K("jccz<Esc>")], check=final_src("> a\n> z\n> c", "> a\nz\n> c"))
case("C67/list", "Source", "- a\n  bb\n  c", [K("jccz<Esc>")], check=final_src("- a\n  z\n  c"))
case("C67/list-xxx", "Source", "- a\n  bb\n  c", [K("j^xxxiz<Esc>")], check=final_src("- a\n  z\n  c"))
case("C67/last-line", "Source", "a\nb\nc\n", [K("jjccx<Esc>")], check=final_src("a\nb\nx\n"))
case("C67/list-last", "Source", "- a\n  bb\n", [K("jccx<Esc>")], check=final_src("- a\n  x\n"))
case("C68/fence", "Source", "```\n\n\n```", [K("2lRxy<BS><BS><Esc>")], check=final_src("```\n\n\n```"))
case("C68/tab", "Source", "a\nb\n\nc", [K("jR<Tab><BS><Esc>")], check=final_src("a\nb\n\nc"))
case("C68/list-tab", "Source", "- a\n  b\n  c", [K("j^R<Tab><BS><Esc>")], check=final_src("- a\n  b\n  c"))
case("C68/quote-tab", "Source", "> a\n>\n> b", [K("jR<Tab><BS><Esc>")], check=final_src("> a\n>\n> b"))
case("C68/indented", "Source", "    a\n\n    b", [K("Gl"), K("Rxy<BS><BS><Esc>")], check=final_src("    a\n\n    b"))
case("C68/nl", "Source", "\n", [K("R<CR><BS><Esc>")], check=final_src("\n"))
case("C68/ws", "Source", "   \n  \n", [K("R<CR><BS><Esc>")], check=final_src("   \n  \n"))
case("C68/wys-it", "Wysiwyg", "*it*", [K("R<CR><BS><Esc>")], check=final_src("*it*"))
for s in ["&amp;x", "\\*x", "\\[x", "a\nb", "a  \nb"]:
    case(f"C68/respell/{s!r}", "Wysiwyg", s, [K("Rxyz<BS><BS><BS><Esc>")], check=final_src(s))
case("C69/BS", "Plain", "First paragraph.", [K("9|Rxyz<BS><BS><BS><BS><BS><Esc>")], check=final_src("First paragraph."))
case("C69/C-w", "Plain", "First paragraph.", [K("9|Rxyz<C-w><Esc>")], check=final_src("First paragraph."))
case("C69/source-br", "Source", "a<br>b", [K("3|Rxyz<C-w><Esc>")], check=final_src("a<br>b"))
case("L6/insert-C-w", "Plain", "paragraph", [K("5|ixyz<C-w><Esc>")], check=final_src("paragraph"))
case("C47", "Source", "a  \nb", [K("i"), SEL(2, 3), A("indent", unindent=False)], check=no_internal())
case("C47b", "Source", "a<br>b", [K("i"), SEL(2, 3), A("indent", unindent=False)], check=no_internal())
case("C58/a", "Source", "a\n- \nb", [K("i"), SEL(4, 6), K("<CR>")], check=no_internal())
case("C58/b", "Source", "*\ta\n*\tb", [K("i"), SEL(1, 7), K("<CR>")], check=no_internal())
case("C58/c", "Source", "x<br/>\ny", [K("i"), SEL(2, 3), A("indent", unindent=False)], check=no_internal())
case("C58/d", "Source", "![a](u \"t\")![b](v)", [K("i"), SEL(1, 3), A("indent", unindent=False)], check=no_internal())
E58 = "- a\n\n- 👩‍💻\n  > q\n\n  ```\n  c\n  ```\n\n- b"
case("C58/emoji-tab", "Source", E58, [K("jjA"), K("<S-Down>"), K("<Tab>")], check=no_internal(r"PANIC|NotGraphemeBoundary|InvalidRange|VerificationFailed"))

# ---------------------------------------------------------------- layout
T3 = "| a | b | c |\n| :-: | --- | ---: |\n| 1 | 2 | 3 |"
case("C49/zoom", "Wysiwyg", T3, [A("resize", width=120, height=200), A("select_all"), A("scale", scale=0.5)])
case("C49/zoom2", "Wysiwyg", "| a | b |\n| -- | - |\n| é👩‍💻 | العربية |", [K("i<S-Down>"), A("scale", scale=2.0), K("<S-Down>"), A("scale", scale=0.5)])
case("C49/resize", "Wysiwyg", TBL + "\n\npara words", [K("G"), W(5), A("paste", text="x\ny\nz"), A("resize", width=300, height=900)])
case("C23", "Wysiwyg", "- a", [K("i"), T("-"), T(" ")])
for s in ["0. zero", "1) a\n2) b", "- a\n1. b", "-\ta\n-\tb"]:
    for seq in ["- ", "# ", "> ", "1. "]:
        case(f"C23/{s!r}/{seq!r}", "Wysiwyg", s, [K("i"), T(seq[0]), T(seq[1:])])
for s in ["*\ta\n*\tb", "-\tfoo\n\n\tbar"]:
    for seq in ["- ", "# ", "> "]:
        case(f"C56/tab/{s!r}/{seq!r}", "Wysiwyg", s, [K("i"), T(seq[0]), T(seq[1:])])
case("C56/tab-cc", "Wysiwyg", "-\tfoo\n\n\tbar", [K("Gccx<Esc>")])
for seq in ["- ", "# ", "> "]:
    case(f"C56/1.a/{seq!r}", "Wysiwyg", "1. a\n\n\n2. b", [K("i"), T(seq[0]), T(seq[1:])])
case("C56/code-in-list-o", "Wysiwyg", "- a\n\n      indented code in list\n- b", [K("jox<Esc>")])
case("C56/code-in-list-O", "Wysiwyg", "- a\n\n      indented code in list\n- b", [K("jOx<Esc>")])
for k in ["dip", "das", "vipd"]:
    case(f"C56/unclosed-fence/{k}", "Wysiwyg", "> ```\n> unclosed fence in quote\n\nafter", [K(k)])
case("C56/1.one", "Wysiwyg", "1. one\n1. one\n1. one\n\n   para in third", [K("Vjcx<Esc>")])
case("C56/footnote", "Wysiwyg", "foot[^1] note\n\n[^1]: The note.", [K("G$x")])
case("C56/ref-nextline", "Wysiwyg", "[ref]:\n/url\n\n[ref]", [K("Cx<Esc>")])
case("C56/unclosed-comment-o", "Wysiwyg", "<!-- unclosed comment", [K("o"), T("---")])
case("C56/ref-A", "Wysiwyg", "[ref]\n\n[ref]: /url", [K("A"), T("]"), T("é")])
case("C56/empty-item-dw", "Wysiwyg", "a\n- \nb", [K("jdw")])
for s in ["- a\n  b\n  c", "1) a\n2) b", "- عربي"]:
    case(f"C56/1./{s!r}", "Wysiwyg", s, [K("i"), T("1"), T("."), T(" ")])

# ---------------------------------------------------------------- lower-priority traces
case("L1", "Source", "<ins>unclosed", [A("paste", text="- x\n- y\n"), K("O"), T("العربية"), P(1), A("semantic", style="emphasis", enabled=True), K("<CR>")], check=no_internal())
case("L2", "Wysiwyg", "&bogus; &#xZZ; &", [A("resize", width=700, height=1200), K("i"), T("q"), A("resize", width=120, height=30), W(1, extend=True),
                                             K("a"), T("---"), T("q"), K("<Esc>dd.r<CR>O<S-Left>"), T("é")], check=no_internal())
case("L3", "Wysiwyg", TBL + "\n\ntext after", [A("resize", width=700, height=2000), K("a<CR>"), T("العربية"), K("<Esc><C-v>d")], check=no_internal())
case("L4", "Wysiwyg", "- | a | b |\n  | - | - |\n  | 1 | 2 |", [A("resize", width=40, height=30), A("table_delete_row"), A("scroll", top=50, left=0),
                                                              A("resize", width=300, height=200)], check=no_internal(r"PANIC|LayoutInstall|InconsistentLayoutSnapshot"))
case("L4b", "Wysiwyg", "**`code in bold`**", [A("resize", width=600, height=30), K("i<Esc>")])
case("L5/r", "Wysiwyg", "`a b c`", [EX("s/ b /\\r/")])
case("L5/n", "Wysiwyg", "`a b c`", [EX("s/ b /\\n/")], check=no_internal(r"PANIC|candidate source did not"))
case("L7", "Wysiwyg", "<ins>unclosed", [K("i"), SEL(0, 8), A("compose", text="é", commit=True)], check=no_internal())

# ---------------------------------------------------------------- demo document
import os
DEMO = open(os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "..", "..", "docs", "markdown_demo.md")).read()
case("C63/dd-fence", "Wysiwyg", "```\n  This fence is indented.\n    Two additional spaces.\n  ```\n\n    Four spaces code.", [K("dd")])
case("C63/dd-fence2", "Wysiwyg", "  ```\n  This fence is indented.\n    Two additional spaces.\n  ```\n\n    Four spaces code.", [K("dd")])
case("C63/demo-g", "Wysiwyg", DEMO, [EX("g/a/d")], timeout=120, check=no_internal(r"PANIC|VerificationFailed"))
case("C63/demo-t.", "Wysiwyg", DEMO, [EX("t.")])
case("C63/demo-m+1", "Wysiwyg", DEMO, [EX("m+1")])
case("C63/demo-dip", "Wysiwyg", DEMO, [K("dip")])

# ---------------------------------------------------------------- October 11 audit: regressions (R)
# IDs match the "Fix-verification audit, October 11, 2026" section of BUGS.md.
for f in ["Plain", "Code", "Source", "Wysiwyg"]:
    case(f"R1/one-letter/{f}", f, "I saw a cat.\n", [K("wwcwx<Esc>")], check=final_src("I saw x cat.\n"))
    case(f"R1/word-end/{f}", f, "ab\ncd\nef\n", [K("lcwx<Esc>")], check=final_src("ax\ncd\nef\n", "ax\ncd\nef"))
for f in ["Plain", "Code", "Source"]:
    case(f"R1/cW/{f}", f, "ab\ncd\nef\n", [K("lcWx<Esc>")], check=final_src("ax\ncd\nef\n"))
    case(f"R1/c2w/{f}", f, "ab\ncd\nef\n", [K("lc2wx<Esc>")], check=final_src("ax\nef\n"))
    case(f"R1/final-newline/{f}", f, "a\n", [K("cwx<Esc>")], check=final_src("x\n"))
for f in ["Plain", "Source", "Wysiwyg"]:
    for cmd in ["d", ".,$d", "t0", "m-2", "m0"]:
        case(f"R2/{f}/{cmd}", f, "a\n\nbé", [K("G"), EX(cmd)], check=no_internal(r"AddressOverflow"))
for f in ["Plain", "Code"]:
    case(f"R3/{f}", f, "a\nb\na", [EX("g/a/d")], check=final_src("b"))
case("R4/last-paragraph", "Plain", "First paragraph.\nSecond line.\n\nLast line.", [K("Gdap")], check=final_src("First paragraph.\nSecond line."))
case("R4/final-newline", "Plain", "a\nb\n\nc\n", [K("Gkdap")], check=final_src("a\nb\n"))
case("R5/de", "Plain", "ab\n", [K("$de")], check=final_src("a\n"))
case("R5/yeP", "Plain", "ab\n", [K("$yeP")], check=final_src("abb\n"))
case("R5/v$d", "Plain", "ab\ncd\n", [K("jv$d")], check=final_src("ab\n\n"))
case("R5/d2w", "Code", "s = '''\nmulti\nline\n'''\n", [K("jd2w")], filename="x.py", check=final_src("s = '''\n'''\n"))
case("R5/db", "Code", "s = '''\nmulti\nline\n'''\n", [K("jjdb")], filename="x.py", check=final_src("s = '''\nline\n'''\n"))
case("R5/dd.", "Code", "plain\n\ttext\n", [K("jdd.")], filename="x.rs", check=final_src(""))
case("R5/dw-empty-line", "Plain", "P.\nS.\n\nL.\n", [K("2jdw")], check=final_src("P.\nS.\nL.\n"))
case("R5/r-enter-autoindent", "Plain", " * star\n", [K("lr<CR>")], check=final_src(" \n star\n"))
for f in ["Wysiwyg", "Source"]:
    for s_ in ["1. \n2. ", "- \n- "]:
        case(f"R6/{f}/{s_!r}", f, s_, [K("G$a<Tab>")])
for s_, want in [("- a\n- b", "- a\n- b\n  - x"), ("1. a\n2. b", "1. a\n2. b\n   1. x")]:
    case(f"R7/{s_!r}", "Wysiwyg", s_, [K("GA<CR><Tab>"), T("x")], check=final_src(want))
case("R8", "Wysiwyg", "| a |\n| --- |\n\nx", [EX("t."), K("i"), T("q")], check=no_internal(r"PANIC|InputPanic"))
case("R9", "Wysiwyg", "| *a* | b |\n| - | - |", [K("i"), SEL(0, 2), A("semantic", style="emphasis", enabled=True)], check=no_internal(r"PANIC|InputPanic"))
case("R10/yyp", "Wysiwyg", "Setext\n===", [K("yyp")])
case("R10/ddp", "Wysiwyg", "Setext\n===\n\nx", [K("ddp")])
case("R11", "Wysiwyg", "a `b` c `d` e", [K("xp")], check=final_src("&#32;a`b` c `d` e", " a`b` c `d` e"))

# ---------------------------------------------------------------- October 11 audit: partly fixed earlier IDs
case("C0/global-jump", "Plain", "xyz", [EX("v/a/d"), K("A"), T("éé"), K("<Esc>"), EX("s/^/#/")], check=no_internal(r"PANIC|InputPanic"))
for s_, a, b, at in [("# A\n## B", 0, 1, 2), ("https://bare.link.example", 0, 1, 2), ("<ins>under</ins> <sup>sup</sup>", 0, 1, 5)]:
    case(f"C65/more/{s_!r}", "Wysiwyg", s_, [K("i"), SEL(a, b), K("<Copy>"), P(at), A("paste_stored")])
case("C68/code-replace-enter", "Code", "fn main() {\n    println!(\"hi\");\n}\n", [K("j0lR<CR><BS><Esc>")], filename="x.rs", check=final_src("fn main() {\n    println!(\"hi\");\n}\n"))
case("C44/deep-list-code-style", "Wysiwyg", "- a\n  - b\n    - c\n      - d", [K("i"), P(7), A("char_style", style="Code"), T("x")])
case("C23/1-dot-in-item", "Wysiwyg", "- a\n1. b", [K("i"), T("1"), T(". ")])
for s_, k in [("[ref]\n\n[ref]: /url", "G:t.<CR>"), ("1. a\n   - b\n2. c", ":m+1<CR>"), ("- a\n\n  continued\n- b", "j:m0<CR>"), ("    a\n\n    b", ":t.<CR>")]:
    case(f"C34/more/{s_!r}", "Wysiwyg", s_, [K(k)])

# ---------------------------------------------------------------- October 11 audit: new findings (N)
DOC_N1 = "# Title\n\nFirst para line one\nline two.\n\n- item a\n- item b\n\nLast para."
for f in ["Wysiwyg", "Source"]:
    case(f"N1/dap/{f}", f, DOC_N1, [P(8), K("dap")], check=all_ok_then(lambda st: "Last para." in st[-1]["source"] and "item a" in st[-1]["source"], lambda st: f"deleted too much: {st[-1]['source']!r}"))
    case(f"N1/brace/{f}", f, DOC_N1, [P(8), K("}")], check=all_ok_then(lambda st: st[-1]["cursor"] < len(st[-1]["text"]) - 12, lambda st: f"}} went to {st[-1]['cursor']} of {len(st[-1]['text'])}"))
case("N2/autolink-space", "Wysiwyg", "<https://auto.link>", [K("A"), T(" "), T("x")])
case("N2/mailto-space", "Wysiwyg", "<mailto:a@b.c>", [K("A"), T(" "), T("x")])
case("N3/quoted-table-empty-cell", "Wysiwyg", "> |  |  |\n> | - | - |\n> |  |  |", [K("i"), P(1), T("x")])
case("N3/quoted-table-insert", "Wysiwyg", "> nested", [A("table_insert", columns=2, rows=1), T("x")])
case("N4/2dd-into-code", "Wysiwyg", "p\n\n```\nc1\nc2\n```\n\nz", [K("2dd")], check=final_src("```\nc2\n```\n\nz"))
case("N5/autolink-cc", "Wysiwyg", "<https://auto.link>", [K("ccx<Esc>")], check=final_src("x"))
case("N5/autolink-dd-last", "Wysiwyg", "x\n\n<https://auto.link>", [K("Gdd")])
case("N6/setext-next-cc", "Wysiwyg", "a\n=\n\nb", [K("Gccx<Esc>")], check=final_src("a\n=\n\nx"))
case("N7/enter-before-image", "Wysiwyg", "![](local.png)", [K("i<CR>")])
case("N7/quote-off-in-list", "Wysiwyg", "- > quote in list", [A("quote", enabled=False)], check=final_src("- quote in list"))
case("N7/atx-closing-to-paragraph", "Wysiwyg", "# Heading #", [A("paragraph_style", style="Paragraph")], check=final_src("Heading"))
case("N8/backslash-heading", "Wysiwyg", "# \\\\a", [K("i"), T("q")])
case("N8/list-closing-tag", "Wysiwyg", "- </ins>", [K("i"), T("q")])
case("N9/r-dash", "Wysiwyg", "- a\n- b", [K("r-")])
case("N9/empty-item-hr", "Wysiwyg", "- ", [K("A"), T("---")])
case("N9/source-table-cw", "Source", "| a |\n| - |\n| 1 |", [P(12), K("cwx<Esc>")])
case("N10/source-r-enter-emoji", "Source", "a\né👩‍💻", [K("G0r<CR>")])
case("N10/empty-link-C", "Wysiwyg", "[text]()", [K("Cx<Esc>")], check=all_ok_then(lambda st: "[]()" not in st[-1]["source"], lambda st: f"left {st[-1]['source']!r}"))
case("N12/row-select-heading-delete", "Wysiwyg", "# T\n\nend", [A("row_select", y=2, extend=False), K("<BS>")], check=final_src("end"))
case("N12/row-select-copy-paste", "Wysiwyg", "- a\n- b\n- c", [A("row_select", y=2, extend=False), K("<Copy>"), A("paste_stored")], check=final_src("- a\n- b\n- c"))
case("N12/row-select-quote-paste", "Wysiwyg", "> q\n\nz", [A("row_select", y=2, extend=False), K("<Copy>"), A("paste_stored")], check=final_src("> q\n\nz"))
case("N12/row-select-table-delete", "Wysiwyg", "| a | b |\n| - | - |\n| 1 | 2 |\n\nend", [A("row_select", y=30, extend=False), K("<BS>")])
case("N13/line-number-overlap", "Wysiwyg", "| a |\n| --- |\n", [A("number", enabled=True), K("i"), P(1), K("<CR>")])
case("R12/shift-enter", "Source", "| a |\n| - |\n| 1 |", [K("i"), P(10), K("<ShiftEnter>"), T("x")])
case("R12/shift-enter-last", "Source", "| a | b |\n| - | - |\n| 1 | 2 |", [K("i"), P(28), K("<ShiftEnter>"), T("x")])
case("R13", "Wysiwyg", "- \n- \n- ", [K("i"), P(1), K("<ShiftEnter>"), T("x")], check=final_src("- \n- <br>x\n- "))
case("R14", "Wysiwyg", "<!-- unclosed comment", [K("A"), T("\\")])
case("R1/table-cell", "Wysiwyg", "| a | b |\n| - | - |\n| 1 | 2 |", [K("cwx<Esc>")], check=final_src("| x | b |\n| - | - |\n| 1 | 2 |"))
case("R15", "Wysiwyg", "| a | b |\n| - | - |\n| 1 | 2 |", [K("R<CR><BS><Esc>")], check=final_src("| a | b |\n| - | - |\n| 1 | 2 |"))
case("R16", "Source", "```rust\nfn main() {}\n```", [EX("v/a/d")], check=final_src("fn main() {}"))
case("R17", "Wysiwyg", "- a\n- b\n- c", [K("j"), EX("s/^/#/")])
case("R18/page-down", "Plain", "a\nb\nc", [K("<PageDown>")], check=all_ok_then(lambda st: st[-1]["cursor"] > 0, lambda st: f"cursor stayed at {st[-1]['cursor']}"))
case("R18/page-up", "Plain", "a\nb\nc", [K("G<PageUp>")], check=all_ok_then(lambda st: st[-1]["cursor"] < 4, lambda st: f"cursor stayed at {st[-1]['cursor']}"))
