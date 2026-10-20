//! Vim's option table and Viem's treatment of each option. The `:set`
//! implementation in `ex_execute.rs` owns the options Viem implements; it
//! hands every other name to [`apply_operation`], which:
//!
//! - accepts values matching Viem's fixed behavior silently;
//! - ignores undo, backup and swap options silently (a product decision);
//! - stores mapped and feature options for the code that consumes them;
//! - remembers recognized but unsupported options so scripts read back what
//!   they set, with a notice when the value differs from Vim's default;
//! - rejects unknown names with Vim's E518.
//!
//! The table comes from Vim 9.1's `:help option-list`, with Vim's defaults
//! for a nocompatible session (`vim -u NONE -N`).
use std::collections::BTreeMap;

use super::ex::{OptionAction, OptionOperation, SetScope};
use super::ex_execute::{ExExecuteError, ExFrontendRequest, ExInfoRequest, ExOptionValue, ExOutcome};
use super::*;
use crate::vimscript::value::Value;
use crate::vimscript::{EvalResult, Fail, OptionScope, VimError};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum OptionKind {
    Bool,
    Number,
    Text,
}
use OptionKind::{Bool, Number, Text};

#[derive(Debug)]
pub(crate) struct OptionSpec {
    pub name: &'static str,
    pub abbreviation: &'static str,
    pub kind: OptionKind,
    pub default: &'static str,
    /// Local to a buffer or window in Vim. Viem keeps option state per
    /// buffer, so this only documents Vim's scope.
    #[allow(dead_code)]
    pub local: bool,
}

const fn o(name: &'static str, abbreviation: &'static str, kind: OptionKind, default: &'static str, local: bool) -> OptionSpec {
    OptionSpec { name, abbreviation, kind, default, local }
}

static OPTIONS: &[OptionSpec] = &[
    o("aleph", "al", Number, "0", false),
    o("allowrevins", "ari", Bool, "0", false),
    o("altkeymap", "akm", Bool, "0", false),
    o("ambiwidth", "ambw", Text, "single", false),
    o("antialias", "anti", Bool, "0", false),
    o("arabic", "arab", Bool, "0", true),
    o("arabicshape", "arshape", Bool, "0", false),
    o("autochdir", "acd", Bool, "0", false),
    o("autocomplete", "ac", Bool, "0", false),
    o("autocompletedelay", "acl", Number, "0", false),
    o("autocompletetimeout", "act", Number, "80", false),
    o("autoindent", "ai", Bool, "0", true),
    o("autoread", "ar", Bool, "0", true),
    o("autoshelldir", "asd", Bool, "0", false),
    o("autowrite", "aw", Bool, "0", false),
    o("autowriteall", "awa", Bool, "0", false),
    o("background", "bg", Text, "light", false),
    o("backspace", "bs", Text, "indent,eol,start", false),
    o("backup", "bk", Bool, "0", false),
    o("backupcopy", "bkc", Text, "auto", true),
    o("backupdir", "bdir", Text, ".,~/tmp,~/", false),
    o("backupext", "bex", Text, "~", false),
    o("backupskip", "bsk", Text, "/tmp/*", false),
    o("balloondelay", "bdlay", Number, "0", false),
    o("ballooneval", "beval", Bool, "0", false),
    o("balloonevalterm", "bevalterm", Bool, "0", false),
    o("balloonexpr", "bexpr", Text, "", true),
    o("belloff", "bo", Text, "", false),
    o("binary", "bin", Bool, "0", true),
    o("bioskey", "biosk", Bool, "0", false),
    o("bomb", "", Bool, "0", true),
    o("breakat", "brk", Text, "' ", false),
    o("breakindent", "bri", Bool, "0", true),
    o("breakindentopt", "briopt", Text, "", true),
    o("browsedir", "bsdir", Text, "", false),
    o("bufhidden", "bh", Text, "", true),
    o("buflisted", "bl", Bool, "1", true),
    o("buftype", "bt", Text, "", true),
    o("casemap", "cmp", Text, "internal,keepascii", false),
    o("cdhome", "cdh", Bool, "0", false),
    o("cdpath", "cd", Text, ",,", false),
    o("cedit", "", Text, "", false),
    o("charconvert", "ccv", Text, "", false),
    o("chistory", "chi", Number, "10", false),
    o("cindent", "cin", Bool, "0", true),
    o("cinkeys", "cink", Text, "0{,0},0),0],:,0#,!^F,o,O,e", true),
    o("cinoptions", "cino", Text, "", true),
    o("cinscopedecls", "cinsd", Text, "public,protected,private", true),
    o("cinwords", "cinw", Text, "if,else,while,do,for,switch", true),
    o("clipboard", "cb", Text, "", false),
    o("clipmethod", "cpm", Text, "wayland,x11,gui,other", false),
    o("cmdheight", "ch", Number, "1", false),
    o("cmdwinheight", "cwh", Number, "7", false),
    o("colorcolumn", "cc", Text, "", true),
    o("columns", "co", Number, "80", false),
    o("comments", "com", Text, "s1:/*,mb:*,ex:*/,://,b:#,:%,:XCOMM,n:>,fb:-", true),
    o("commentstring", "cms", Text, "/* %s */", true),
    o("compatible", "cp", Bool, "0", false),
    o("complete", "cpt", Text, ".,w,b,u,t,i", true),
    o("completefunc", "cfu", Text, "", true),
    o("completefuzzycollect", "cfc", Text, "", false),
    o("completeitemalign", "cia", Text, "abbr,kind,menu", false),
    o("completeopt", "cot", Text, "menu,preview", true),
    o("completepopup", "cpp", Text, "", false),
    o("completeslash", "csl", Text, "", true),
    o("completetimeout", "cto", Number, "0", false),
    o("concealcursor", "cocu", Text, "", true),
    o("conceallevel", "cole", Number, "0", true),
    o("confirm", "cf", Bool, "0", false),
    o("conskey", "consk", Bool, "0", false),
    o("copyindent", "ci", Bool, "0", true),
    o("cpoptions", "cpo", Text, "aABceFsz", false),
    o("cryptmethod", "cm", Text, "blowfish2", true),
    o("cscopepathcomp", "cspc", Number, "0", false),
    o("cscopeprg", "csprg", Text, "cscope", false),
    o("cscopequickfix", "csqf", Text, "", false),
    o("cscoperelative", "csre", Bool, "0", false),
    o("cscopetag", "cst", Bool, "0", false),
    o("cscopetagorder", "csto", Number, "0", false),
    o("cscopeverbose", "csverb", Bool, "0", false),
    o("cursorbind", "crb", Bool, "0", true),
    o("cursorcolumn", "cuc", Bool, "0", true),
    o("cursorline", "cul", Bool, "0", true),
    o("cursorlineopt", "culopt", Text, "both", true),
    o("debug", "", Text, "", false),
    o("define", "def", Text, "^\\s*#\\s*define", true),
    o("delcombine", "deco", Bool, "0", false),
    o("dictionary", "dict", Text, "", true),
    o("diff", "", Bool, "0", true),
    o("diffanchors", "dia", Text, "", true),
    o("diffexpr", "dex", Text, "", false),
    o("diffopt", "dip", Text, "filler,closeoff,inline:simple", false),
    o("digraph", "dg", Bool, "0", false),
    o("directory", "dir", Text, ".,~/tmp,/var/tmp,/tmp", false),
    o("display", "dy", Text, "", false),
    o("eadirection", "ead", Text, "both", false),
    o("edcompatible", "ed", Bool, "0", false),
    o("emoji", "emo", Bool, "1", false),
    o("encoding", "enc", Text, "latin1", false),
    o("endoffile", "eof", Bool, "0", true),
    o("endofline", "eol", Bool, "1", true),
    o("equalalways", "ea", Bool, "1", false),
    o("equalprg", "ep", Text, "", true),
    o("errorbells", "eb", Bool, "0", false),
    o("errorfile", "ef", Text, "errors.err", false),
    o("errorformat", "efm", Text, "%*[^\"]\"%f\"%*\\D%l: %m,\"%f\"%*\\D%l: %m,%-Gg%\\?make[%*\\d]: *** [%f:%l:%m,%-Gg%\\?make: *** [%f:%l:%m,%-G%f:%l: (Each undeclared identifier is reported only once,%-G%f:%l: for each function it appears in.),%-GIn file included from %f:%l:%c:,%-GIn file included from %f:%l:%c\\,,%-GIn file included from %f:%l:%c,%-GIn file included from %f:%l,%-G%*[ ]from %f:%l:%c,%-G%*[ ]from %f:%l:,%-G%*[ ]from %f:%l\\,,%-G%*[ ]from %f:%l,%f:%l:%c:%m,%f(%l):%m,%f:%l:%m,\"%f\"\\, line %l%*\\D%c%*[^ ] %m,%D%*\\a[%*\\d]: Entering directory %*[`']%f',%X%*\\a[%*\\d]: Leaving directory %*[`']%f',%D%*\\a: Entering directory %*[`']%f',%X%*\\a: Leaving directory %*[`']%f',%DMaking %*\\a in %f,%f|%l| %m", true),
    o("esckeys", "ek", Bool, "1", false),
    o("eventignore", "ei", Text, "", false),
    o("eventignorewin", "eiw", Text, "", false),
    o("expandtab", "et", Bool, "0", true),
    o("exrc", "ex", Bool, "0", false),
    o("fileencoding", "fenc", Text, "", true),
    o("fileencodings", "fencs", Text, "ucs-bom", false),
    o("fileformat", "ff", Text, "unix", true),
    o("fileformats", "ffs", Text, "unix,dos", false),
    o("fileignorecase", "fic", Bool, "1", false),
    o("filetype", "ft", Text, "", true),
    o("fillchars", "fcs", Text, "vert:|,fold:-,eob:~,lastline:@", true),
    o("findfunc", "ffu", Text, "", true),
    o("fixendofline", "fixeol", Bool, "1", true),
    o("fkmap", "fk", Bool, "0", false),
    o("foldclose", "fcl", Text, "", false),
    o("foldcolumn", "fdc", Number, "0", true),
    o("foldenable", "fen", Bool, "1", true),
    o("foldexpr", "fde", Text, "0", true),
    o("foldignore", "fdi", Text, "#", true),
    o("foldlevel", "fdl", Number, "0", true),
    o("foldlevelstart", "fdls", Number, "-1", false),
    o("foldmarker", "fmr", Text, "{{{,}}}", true),
    o("foldmethod", "fdm", Text, "manual", true),
    o("foldminlines", "fml", Number, "1", true),
    o("foldnestmax", "fdn", Number, "20", true),
    o("foldopen", "fdo", Text, "block,hor,mark,percent,quickfix,search,tag,undo", false),
    o("foldtext", "fdt", Text, "foldtext()", true),
    o("formatexpr", "fex", Text, "", true),
    o("formatlistpat", "flp", Text, "^\\s*\\d\\+[\\]:.)}\\t ]\\s*", true),
    o("formatoptions", "fo", Text, "tcq", true),
    o("formatprg", "fp", Text, "", true),
    o("fsync", "fs", Bool, "1", false),
    o("gdefault", "gd", Bool, "0", false),
    o("grepformat", "gfm", Text, "%f:%l:%m,%f:%l%m,%f  %l%m", true),
    o("grepprg", "gp", Text, "grep -n $* /dev/null", true),
    o("guicursor", "gcr", Text, "n-v-c:block,o:hor50,i-ci:hor15,r-cr:hor30,sm:block", false),
    o("guifont", "gfn", Text, "", false),
    o("guifontset", "gfs", Text, "", false),
    o("guifontwide", "gfw", Text, "", false),
    o("guiheadroom", "ghr", Number, "0", false),
    o("guiligatures", "gli", Text, "", false),
    o("guioptions", "go", Text, "", false),
    o("guipty", "", Bool, "0", false),
    o("guitablabel", "gtl", Text, "", false),
    o("guitabtooltip", "gtt", Text, "", false),
    o("helpfile", "hf", Text, "/usr/share/vim/vim91/doc/help.txt", false),
    o("helpheight", "hh", Number, "20", false),
    o("helplang", "hlg", Text, "en", false),
    o("hidden", "hid", Bool, "0", false),
    o("highlight", "hl", Text, "8:SpecialKey,~:EndOfBuffer,@:NonText,d:Directory,e:ErrorMsg,i:IncSearch,l:Search,y:CurSearch,m:MoreMsg,M:ModeMsg,n:LineNr,a:LineNrAbove,b:LineNrBelow,N:CursorLineNr,G:CursorLineSign,O:CursorLineFold,r:Question,s:StatusLine,S:StatusLineNC,c:VertSplit,t:Title,v:Visual,V:VisualNOS,w:WarningMsg,W:WildMenu,f:Folded,F:FoldColumn,A:DiffAdd,C:DiffChange,D:DiffDelete,T:DiffText,E:DiffTextAdd,>:SignColumn,-:Conceal,B:SpellBad,P:SpellCap,R:SpellRare,L:SpellLocal,+:Pmenu,=:PmenuSel,k:PmenuMatch,<:PmenuMatchSel,[:PmenuKind,]:PmenuKindSel,{:PmenuExtra,}:PmenuExtraSel,x:PmenuSbar,X:PmenuThumb,*:TabLine,#:TabLineSel,_:TabLineFill,!:CursorColumn,.:CursorLine,o:ColorColumn,q:QuickFixLine,z:StatusLineTerm,Z:StatusLineTermNC,g:MsgArea,h:ComplMatchIns,%:TabPanel,^:TabPanelSel,&:TabPanelFill,I:PreInsert", false),
    o("history", "hi", Number, "200", false),
    o("hkmap", "hk", Bool, "0", false),
    o("hkmapp", "hkp", Bool, "0", false),
    o("hlsearch", "hls", Bool, "0", false),
    o("icon", "", Bool, "0", false),
    o("iconstring", "", Text, "", false),
    o("ignorecase", "ic", Bool, "0", false),
    o("imactivatefunc", "imaf", Text, "", false),
    o("imactivatekey", "imak", Text, "", false),
    o("imcmdline", "imc", Bool, "0", false),
    o("imdisable", "imd", Bool, "0", false),
    o("iminsert", "imi", Number, "0", true),
    o("imsearch", "ims", Number, "-1", true),
    o("imstatusfunc", "imsf", Text, "", false),
    o("imstyle", "imst", Number, "0", false),
    o("include", "inc", Text, "^\\s*#\\s*include", true),
    o("includeexpr", "inex", Text, "", true),
    o("incsearch", "is", Bool, "0", false),
    o("indentexpr", "inde", Text, "", true),
    o("indentkeys", "indk", Text, "0{,0},0),0],:,0#,!^F,o,O,e", true),
    o("infercase", "inf", Bool, "0", true),
    o("isexpand", "ise", Text, "", true),
    o("insertmode", "im", Bool, "0", false),
    o("isfname", "isf", Text, "@,48-57,/,.,-,_,+,,,#,$,%,~,=", false),
    o("isident", "isi", Text, "@,48-57,_,192-255", false),
    o("iskeyword", "isk", Text, "@,48-57,_,192-255", true),
    o("isprint", "isp", Text, "@,161-255", false),
    o("joinspaces", "js", Bool, "1", false),
    o("jumpoptions", "jop", Text, "", false),
    o("key", "", Text, "", true),
    o("keymap", "kmp", Text, "", true),
    o("keymodel", "km", Text, "", false),
    o("keyprotocol", "kpc", Text, "kitty:kitty,foot:kitty,ghostty:kitty,wezterm:kitty,xterm:mok2", false),
    o("keywordprg", "kp", Text, "man", true),
    o("langmap", "lmap", Text, "", false),
    o("langmenu", "lm", Text, "", false),
    o("langnoremap", "lnr", Bool, "0", false),
    o("langremap", "lrm", Bool, "0", false),
    o("laststatus", "ls", Number, "1", false),
    o("lazyredraw", "lz", Bool, "0", false),
    o("lhistory", "lhi", Number, "10", true),
    o("linebreak", "lbr", Bool, "0", true),
    o("lines", "", Number, "24", false),
    o("linespace", "lsp", Number, "0", false),
    o("lisp", "", Bool, "0", true),
    o("lispoptions", "lop", Text, "", true),
    o("lispwords", "lw", Text, "defun,define,defmacro,set!,lambda,if,case,let,flet,let*,letrec,do,do*,define-syntax,let-syntax,letrec-syntax,destructuring-bind,defpackage,defparameter,defstruct,deftype,defvar,do-all-symbols,do-external-symbols,do-symbols,dolist,dotimes,ecase,etypecase,eval-when,labels,macrolet,multiple-value-bind,multiple-value-call,multiple-value-prog1,multiple-value-setq,prog1,progv,typecase,unless,unwind-protect,when,with-input-from-string,with-open-file,with-open-stream,with-output-to-string,with-package-iterator,define-condition,handler-bind,handler-case,restart-bind,restart-case,with-simple-restart,store-value,use-value,muffle-warning,abort,continue,with-slots,with-slots*,with-accessors,with-accessors*,defclass,defmethod,print-unreadable-object", true),
    o("list", "", Bool, "0", true),
    o("listchars", "lcs", Text, "eol:$", true),
    o("loadplugins", "lpl", Bool, "0", false),
    o("luadll", "", Text, "", false),
    o("macatsui", "", Bool, "0", false),
    o("magic", "", Bool, "1", false),
    o("makeef", "mef", Text, "", false),
    o("makeencoding", "menc", Text, "", true),
    o("makeprg", "mp", Text, "make", true),
    o("matchpairs", "mps", Text, "(:),{:},[:]", true),
    o("matchtime", "mat", Number, "5", false),
    o("maxcombine", "mco", Number, "2", false),
    o("maxfuncdepth", "mfd", Number, "100", false),
    o("maxmapdepth", "mmd", Number, "1000", false),
    o("maxmem", "mm", Number, "64733960", false),
    o("maxmempattern", "mmp", Number, "1000", false),
    o("maxmemtot", "mmt", Number, "64733960", false),
    o("maxsearchcount", "msc", Number, "99", false),
    o("menuitems", "mis", Number, "25", false),
    o("messagesopt", "mopt", Text, "hit-enter,history:500", false),
    o("mkspellmem", "msm", Text, "460000,2000,500", false),
    o("modeline", "ml", Bool, "1", true),
    o("modelineexpr", "mle", Bool, "0", false),
    o("modelines", "mls", Number, "5", false),
    o("modifiable", "ma", Bool, "1", true),
    o("modified", "mod", Bool, "0", true),
    o("more", "", Bool, "1", false),
    o("mouse", "", Text, "", false),
    o("mousefocus", "mousef", Bool, "0", false),
    o("mousehide", "mh", Bool, "0", false),
    o("mousemodel", "mousem", Text, "popup_setpos", false),
    o("mousemoveevent", "mousemev", Bool, "0", false),
    o("mouseshape", "mouses", Text, "", false),
    o("mousetime", "mouset", Number, "500", false),
    o("mzquantum", "mzq", Number, "0", false),
    o("mzschemedll", "", Text, "", false),
    o("mzschemegcdll", "", Text, "", false),
    o("nrformats", "nf", Text, "bin,octal,hex", true),
    o("number", "nu", Bool, "0", true),
    o("numberwidth", "nuw", Number, "4", true),
    o("omnifunc", "ofu", Text, "", true),
    o("opendevice", "odev", Bool, "0", false),
    o("operatorfunc", "opfunc", Text, "", false),
    o("osctimeoutlen", "ost", Number, "1000", false),
    o("osfiletype", "oft", Text, "", true),
    o("packpath", "pp", Text, "~/.vim,/usr/share/vim/vimfiles,/usr/share/vim/vim91,/usr/share/vim/vimfiles/after,~/.vim/after", false),
    o("paragraphs", "para", Text, "IPLPPPQPP TPHPLIPpLpItpplpipbp", false),
    o("paste", "", Bool, "0", false),
    o("pastetoggle", "pt", Text, "", false),
    o("patchexpr", "pex", Text, "", false),
    o("patchmode", "pm", Text, "", false),
    o("path", "pa", Text, ".,/usr/include,,", true),
    o("perldll", "", Text, "", false),
    o("preserveindent", "pi", Bool, "0", true),
    o("previewheight", "pvh", Number, "12", false),
    o("previewpopup", "pvp", Text, "", false),
    o("previewwindow", "pvw", Bool, "0", true),
    o("printdevice", "pdev", Text, "", false),
    o("printencoding", "penc", Text, "", false),
    o("printexpr", "pexpr", Text, "system('lpr' . (&printdevice == '' ? '' : ' -P' . &printdevice) . ' ' . v:fname_in) . delete(v:fname_in) + v:shell_error", false),
    o("printfont", "pfn", Text, "courier", false),
    o("printheader", "pheader", Text, "%<%f%h%m%=Page %N", false),
    o("printmbcharset", "pmbcs", Text, "", false),
    o("printmbfont", "pmbfn", Text, "", false),
    o("printoptions", "popt", Text, "", false),
    o("prompt", "", Bool, "1", false),
    o("pumheight", "ph", Number, "0", false),
    o("pummaxwidth", "pmw", Number, "0", false),
    o("pumwidth", "pw", Number, "15", false),
    o("pythondll", "", Text, "", false),
    o("pythonhome", "", Text, "", false),
    o("pythonthreedll", "", Text, "", false),
    o("pythonthreehome", "", Text, "", false),
    o("pyxversion", "pyx", Number, "0", false),
    o("quickfixtextfunc", "qftf", Text, "", false),
    o("quoteescape", "qe", Text, "\\", true),
    o("readonly", "ro", Bool, "0", true),
    o("redrawtime", "rdt", Number, "2000", false),
    o("regexpengine", "re", Number, "1", false),
    o("relativenumber", "rnu", Bool, "0", true),
    o("remap", "", Bool, "1", false),
    o("renderoptions", "rop", Text, "", false),
    o("report", "", Number, "2", false),
    o("restorescreen", "rs", Bool, "0", false),
    o("revins", "ri", Bool, "0", false),
    o("rightleft", "rl", Bool, "0", true),
    o("rightleftcmd", "rlc", Text, "", true),
    o("rubydll", "", Text, "", false),
    o("ruler", "ru", Bool, "0", false),
    o("rulerformat", "ruf", Text, "", false),
    o("runtimepath", "rtp", Text, "~/.vim,/usr/share/vim/vimfiles,/usr/share/vim/vim91,/usr/share/vim/vimfiles/after,~/.vim/after", false),
    o("scroll", "scr", Number, "11", true),
    o("scrollbind", "scb", Bool, "0", true),
    o("scrollfocus", "scf", Bool, "0", false),
    o("scrolljump", "sj", Number, "1", false),
    o("scrolloff", "so", Number, "0", true),
    o("scrollopt", "sbo", Text, "ver,jump", false),
    o("sections", "sect", Text, "SHNHH HUnhsh", false),
    o("secure", "", Bool, "0", false),
    o("selection", "sel", Text, "inclusive", false),
    o("selectmode", "slm", Text, "", false),
    o("sessionoptions", "ssop", Text, "blank,buffers,curdir,folds,help,options,tabpages,winsize,terminal", false),
    o("shell", "sh", Text, "/bin/zsh", false),
    o("shellcmdflag", "shcf", Text, "-c", false),
    o("shellpipe", "sp", Text, "2>&1| tee", false),
    o("shellquote", "shq", Text, "", false),
    o("shellredir", "srr", Text, ">%s 2>&1", false),
    o("shellslash", "ssl", Bool, "0", false),
    o("shelltemp", "stmp", Bool, "1", false),
    o("shelltype", "st", Number, "0", false),
    o("shellxescape", "sxe", Text, "", false),
    o("shellxquote", "sxq", Text, "", false),
    o("shiftround", "sr", Bool, "0", false),
    o("shiftwidth", "sw", Number, "8", true),
    o("shortmess", "shm", Text, "filnxtToOS", false),
    o("shortname", "sn", Bool, "0", true),
    o("showbreak", "sbr", Text, "", true),
    o("showcmd", "sc", Bool, "1", false),
    o("showcmdloc", "sloc", Text, "last", false),
    o("showfulltag", "sft", Bool, "0", false),
    o("showmatch", "sm", Bool, "0", false),
    o("showmode", "smd", Bool, "1", false),
    o("showtabline", "stal", Number, "1", false),
    o("showtabpanel", "stpl", Number, "0", false),
    o("sidescroll", "ss", Number, "0", false),
    o("sidescrolloff", "siso", Number, "0", true),
    o("signcolumn", "scl", Text, "auto", true),
    o("smartcase", "scs", Bool, "0", false),
    o("smartindent", "si", Bool, "0", true),
    o("smarttab", "sta", Bool, "0", false),
    o("smoothscroll", "sms", Bool, "0", true),
    o("softtabstop", "sts", Number, "0", true),
    o("spell", "", Bool, "0", true),
    o("spellcapcheck", "spc", Text, "'[.?!]\\_[\\])''\"", true),
    o("spellfile", "spf", Text, "", true),
    o("spelllang", "spl", Text, "en", true),
    o("spelloptions", "spo", Text, "", true),
    o("spellsuggest", "sps", Text, "best", false),
    o("splitbelow", "sb", Bool, "0", false),
    o("splitkeep", "spk", Text, "cursor", false),
    o("splitright", "spr", Bool, "0", false),
    o("startofline", "sol", Bool, "1", false),
    o("statusline", "stl", Text, "", true),
    o("suffixes", "su", Text, ".bak,~,.o,.h,.info,.swp,.obj", false),
    o("suffixesadd", "sua", Text, "", true),
    o("swapfile", "swf", Bool, "1", true),
    o("swapsync", "sws", Text, "fsync", false),
    o("switchbuf", "swb", Text, "", false),
    o("synmaxcol", "smc", Number, "3000", true),
    o("syntax", "syn", Text, "", true),
    o("tabclose", "tcl", Text, "", false),
    o("tabline", "tal", Text, "", false),
    o("tabpagemax", "tpm", Number, "10", false),
    o("tabpanel", "tpl", Text, "", false),
    o("tabpanelopt", "tplo", Text, "", false),
    o("tabstop", "ts", Number, "8", true),
    o("tagbsearch", "tbs", Bool, "1", false),
    o("tagcase", "tc", Text, "followic", true),
    o("tagfunc", "tfu", Text, "", true),
    o("taglength", "tl", Number, "0", false),
    o("tagrelative", "tr", Bool, "1", false),
    o("tags", "tag", Text, "./tags,tags", true),
    o("tagstack", "tgst", Bool, "1", false),
    o("tcldll", "", Text, "", false),
    o("term", "", Text, "", false),
    o("termbidi", "tbidi", Bool, "0", false),
    o("termencoding", "tenc", Text, "", false),
    o("termguicolors", "tgc", Bool, "0", false),
    o("termwinkey", "twk", Text, "", true),
    o("termwinscroll", "twsl", Number, "10000", true),
    o("termwinsize", "tws", Text, "", true),
    o("termwintype", "twt", Text, "", false),
    o("terse", "", Bool, "0", false),
    o("textauto", "ta", Bool, "1", false),
    o("textmode", "tx", Bool, "0", true),
    o("textwidth", "tw", Number, "0", true),
    o("thesaurus", "tsr", Text, "", true),
    o("thesaurusfunc", "tsrfu", Text, "", true),
    o("tildeop", "top", Bool, "0", false),
    o("timeout", "to", Bool, "1", false),
    o("ttimeout", "", Bool, "0", false),
    o("timeoutlen", "tm", Number, "1000", false),
    o("ttimeoutlen", "ttm", Number, "-1", false),
    o("title", "", Bool, "0", false),
    o("titlelen", "", Number, "85", false),
    o("titleold", "", Text, "Thanks for flying Vim", false),
    o("titlestring", "", Text, "", false),
    o("toolbar", "tb", Text, "", false),
    o("toolbariconsize", "tbis", Text, "", false),
    o("ttybuiltin", "tbi", Bool, "1", false),
    o("ttyfast", "tf", Bool, "1", false),
    o("ttymouse", "ttym", Text, "", false),
    o("ttyscroll", "tsl", Number, "999", false),
    o("ttytype", "tty", Text, "", false),
    o("undodir", "udir", Text, ".", false),
    o("undofile", "udf", Bool, "0", true),
    o("undolevels", "ul", Number, "1000", true),
    o("undoreload", "ur", Number, "10000", false),
    o("updatecount", "uc", Number, "200", false),
    o("updatetime", "ut", Number, "4000", false),
    o("varsofttabstop", "vsts", Text, "", true),
    o("vartabstop", "vts", Text, "", true),
    o("verbose", "vbs", Number, "0", false),
    o("verbosefile", "vfile", Text, "", false),
    o("viewdir", "vdir", Text, "~/.vim/view", false),
    o("viewoptions", "vop", Text, "folds,options,cursor,curdir", false),
    o("viminfo", "vi", Text, "'100,<50,s10,h", false),
    o("viminfofile", "vif", Text, "NONE", false),
    o("virtualedit", "ve", Text, "", true),
    o("visualbell", "vb", Bool, "0", false),
    o("warn", "", Bool, "1", false),
    o("weirdinvert", "wiv", Bool, "0", false),
    o("whichwrap", "ww", Text, "b,s", false),
    o("wildchar", "wc", Number, "9", false),
    o("wildcharm", "wcm", Number, "0", false),
    o("wildignore", "wig", Text, "", false),
    o("wildignorecase", "wic", Bool, "0", false),
    o("wildmenu", "wmnu", Bool, "1", false),
    o("wildmode", "wim", Text, "full", false),
    o("wildoptions", "wop", Text, "", false),
    o("winaltkeys", "wak", Text, "", false),
    o("wincolor", "wcr", Text, "", true),
    o("window", "wi", Number, "0", false),
    o("winfixbuf", "wfb", Bool, "0", true),
    o("winfixheight", "wfh", Bool, "0", true),
    o("winfixwidth", "wfw", Bool, "0", true),
    o("winheight", "wh", Number, "1", false),
    o("winminheight", "wmh", Number, "1", false),
    o("winminwidth", "wmw", Number, "1", false),
    o("winptydll", "", Text, "", false),
    o("winwidth", "wiw", Number, "20", false),
    o("wlseat", "wse", Text, "", false),
    o("wlsteal", "wst", Bool, "0", false),
    o("wltimeoutlen", "wtm", Number, "0", false),
    o("wrap", "", Bool, "1", true),
    o("wrapmargin", "wm", Number, "0", true),
    o("wrapscan", "ws", Bool, "1", false),
    o("write", "", Bool, "1", false),
    o("writeany", "wa", Bool, "0", false),
    o("writebackup", "wb", Bool, "1", false),
    o("writedelay", "wd", Number, "0", false),
    o("xtermcodes", "", Bool, "1", false),
];

/// Options implemented by `:set` in `ex_execute.rs`.
const CORE: &[&str] = &[
    "wrap", "number", "hlsearch", "incsearch", "ignorecase", "smartcase", "wrapscan", "textwidth", "fileformat",
    "fileformats", "formatoptions", "comments", "formatlistpat", "autoindent", "tabstop", "shiftwidth",
    "softtabstop", "expandtab", "smarttab", "list", "listchars", "keymodel", "selectmode",
];

/// Viem's own options, also implemented by `:set` in `ex_execute.rs`.
const VIEM_OPTIONS: &[&str] = &["autoselect", "continuecommentsonenter", "continuecommentsonopenline"];

/// Undo, backup and swap options are ignored silently.
const IGNORED: &[&str] = &[
    "undofile", "undodir", "undolevels", "undoreload", "backup", "writebackup", "backupdir", "backupcopy",
    "backupext", "backupskip", "swapfile", "directory", "updatecount", "patchmode", "swapsync",
];

/// Accepted silently. `Some(value)` is the value matching Viem's fixed
/// behavior (other values give a notice); `None` means every value is
/// meaningless for Viem (terminal, plugin and display-tuning options).
const MATCHING: &[(&str, Option<&str>)] = &[
    ("compatible", Some("0")),
    ("encoding", Some("utf-8")),
    ("nrformats", Some("bin,hex")),
    ("splitbelow", Some("0")),
    ("splitright", Some("0")),
    ("linebreak", Some("1")),
    ("hidden", Some("1")),
    ("ttyfast", None),
    ("ttimeout", None),
    ("ttimeoutlen", None),
    ("term", None),
    ("termencoding", None),
    ("termguicolors", None),
    ("lazyredraw", None),
    ("conceallevel", None),
    ("concealcursor", None),
    ("synmaxcol", None),
    ("updatetime", None),
    ("signcolumn", None),
    ("mouse", None),
    ("mousemodel", None),
    ("ttymouse", None),
    ("ttybuiltin", None),
    ("ttyscroll", None),
    ("restorescreen", None),
    ("esckeys", None),
    ("weirdinvert", None),
    ("icon", None),
    ("iconstring", None),
    ("title", None),
    ("titlestring", None),
    ("titlelen", None),
    ("titleold", None),
    ("redrawtime", None),
    ("regexpengine", None),
    ("maxmempattern", None),
    ("maxmem", None),
    ("maxmemtot", None),
    ("loadplugins", None),
    ("packpath", None),
    ("runtimepath", None),
    ("helplang", None),
    ("langmenu", None),
    ("modeline", None),
    ("modelines", None),
    ("shortmess", None),
    ("ruler", None),
    ("showcmd", None),
    ("showmode", None),
    ("antialias", None),
    ("renderoptions", None),
    ("fileencodings", None),
    ("viminfo", None),
    ("viminfofile", None),
    ("shell", None),
    ("shellcmdflag", None),
    ("shellquote", None),
    ("shellxquote", None),
    ("shellpipe", None),
    ("shellredir", None),
    ("shellslash", None),
    ("shelltemp", None),
    ("termwinkey", None),
    ("termwinscroll", None),
    ("termwinsize", None),
    ("termwintype", None),
    ("secure", None),
    ("exrc", None),
    ("insertmode", Some("0")),
    ("edcompatible", Some("0")),
    ("magic", Some("1")),
];

/// Mapped onto Viem settings; see docs/vimscript.md#options-and-native-settings.
const MAPPED: &[&str] = &[
    "background", "guifont", "linespace", "lines", "columns", "laststatus", "guioptions", "timeout",
    "timeoutlen", "scrolloff", "breakindent", "breakindentopt", "readonly", "fileencoding", "bomb", "belloff",
    "visualbell", "errorbells",
];

/// Implemented by Viem's own command code; see docs/vimscript.md#options-and-native-settings.
const FEATURES: &[&str] = &[
    "clipboard", "gdefault", "shiftround", "joinspaces", "whichwrap", "startofline", "history", "iskeyword",
    "filetype", "syntax", "sections", "keywordprg",
];

/// Comma-separated list options, whose `+=`, `^=` and `-=` work on items.
const COMMA_LISTS: &[&str] = &[
    "backspace", "belloff", "breakindentopt", "clipboard", "complete", "completeopt", "cinoptions", "cinkeys",
    "cinwords", "diffopt", "display", "eventignore", "fileencodings", "foldopen", "foldclose", "guicursor",
    "guifont", "guifontwide", "isfname", "isident", "iskeyword", "isprint", "matchpairs", "nrformats", "path",
    "runtimepath", "packpath", "sessionoptions", "spelllang", "suffixes", "suffixesadd", "switchbuf", "tags",
    "viewoptions", "viminfo", "virtualedit", "whichwrap", "wildignore", "wildmode", "wildoptions", "fillchars",
    "mouse", "cdpath", "dictionary", "thesaurus", "spellfile", "spellsuggest", "undodir", "backupdir",
    "directory", "backupskip", "comments", "lispwords", "renderoptions", "tagcase", "colorcolumn",
    "cursorlineopt", "jumpoptions", "wildignore", "keyprotocol", "termwintype",
];

/// Flag-list options (`+=` appends missing flags, `-=` removes flags).
const FLAG_LISTS: &[&str] = &["shortmess", "cpoptions", "guioptions", "mouse", "formatoptions", "complete", "whichwrap", "concealcursor"];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Treatment {
    Core,
    Matching(Option<&'static str>),
    Ignored,
    Mapped,
    Feature,
    Unsupported,
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct Resolved {
    pub name: &'static str,
    pub kind: OptionKind,
    pub default: &'static str,
    pub treatment: Treatment,
}

/// Resolve a full or abbreviated option name. Terminal codes (`t_xx`)
/// resolve to a meaningless string option of that name.
pub(crate) fn resolve(name: &str) -> Option<Resolved> {
    if let Some(full) = VIEM_OPTIONS.iter().find(|full| **full == name) {
        return Some(Resolved { name: full, kind: Bool, default: "0", treatment: Treatment::Core });
    }
    if is_terminal_code(name) {
        let (kind, default) = if name == "t_Co" { (Number, "256") } else { (Text, "") };
        return Some(Resolved { name: "t_", kind, default, treatment: Treatment::Matching(None) });
    }
    let spec = OPTIONS.iter().find(|spec| spec.name == name || (!spec.abbreviation.is_empty() && spec.abbreviation == name))?;
    let treatment = if CORE.contains(&spec.name) {
        Treatment::Core
    } else if IGNORED.contains(&spec.name) {
        Treatment::Ignored
    } else if let Some((_, value)) = MATCHING.iter().find(|(full, _)| *full == spec.name) {
        Treatment::Matching(*value)
    } else if MAPPED.contains(&spec.name) {
        Treatment::Mapped
    } else if FEATURES.contains(&spec.name) {
        Treatment::Feature
    } else {
        Treatment::Unsupported
    };
    // Options fixed by Viem's behavior report that value by default.
    let default = match treatment {
        Treatment::Matching(Some(value)) => value,
        _ => spec.default,
    };
    Some(Resolved { name: spec.name, kind: spec.kind, default, treatment })
}

fn is_terminal_code(name: &str) -> bool {
    name.len() == 4 && name.starts_with("t_")
}

/// `exists('&name')` (any Vim option) and `exists('+name')` (an option
/// that works in Viem: implemented, mapped, or without meaning in a GUI).
pub(crate) fn option_exists(name: &str, working: bool) -> bool {
    match resolve(name) {
        None => false,
        Some(_) if !working => true,
        Some(resolved) => !matches!(resolved.treatment, Treatment::Unsupported),
    }
}

/// The value of an option outside the core `:set` implementation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum VimOptionValue {
    Bool(bool),
    Number(i64),
    Text(String),
}

impl VimOptionValue {
    fn parse_default(kind: OptionKind, text: &str) -> Self {
        match kind {
            Bool => Self::Bool(text == "1"),
            Number => Self::Number(text.parse().unwrap_or(0)),
            Text => Self::Text(text.to_owned()),
        }
    }

    pub fn number(&self) -> i64 {
        match self {
            Self::Bool(value) => *value as i64,
            Self::Number(value) => *value,
            Self::Text(text) => text.parse().unwrap_or(0),
        }
    }

    pub fn text(&self) -> String {
        match self {
            Self::Bool(value) => (*value as i64).to_string(),
            Self::Number(value) => value.to_string(),
            Self::Text(text) => text.clone(),
        }
    }

    fn display(&self, name: &str) -> String {
        match self {
            Self::Bool(true) => format!("  {name}"),
            Self::Bool(false) => format!("no{name}"),
            Self::Number(value) => format!("  {name}={value}"),
            Self::Text(text) => format!("  {name}={text}"),
        }
    }

    fn script_value(&self) -> Value {
        match self {
            Self::Bool(value) => Value::Number(*value as i64),
            Self::Number(value) => Value::Number(*value),
            Self::Text(text) => Value::String(text.as_str().into()),
        }
    }
}

/// Values set for options outside the core `:set` implementation, keyed by
/// full name (or by the terminal code name).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct VimOptions {
    values: BTreeMap<String, VimOptionValue>,
    keyword: Option<KeywordSet>,
}

impl VimOptions {
    /// The value of a recognized option: the stored value or Vim's default.
    pub fn get(&self, name: &str) -> Option<VimOptionValue> {
        let resolved = resolve(name)?;
        let key = if resolved.name == "t_" { name } else { resolved.name };
        Some(self.values.get(key).cloned().unwrap_or_else(|| VimOptionValue::parse_default(resolved.kind, resolved.default)))
    }

    pub fn number(&self, name: &str) -> i64 {
        self.get(name).map_or(0, |value| value.number())
    }

    pub fn flag(&self, name: &str) -> bool {
        self.number(name) != 0
    }

    pub fn text(&self, name: &str) -> String {
        self.get(name).map(|value| value.text()).unwrap_or_default()
    }

    pub(crate) fn keywords(&self) -> &KeywordSet {
        static DEFAULT: std::sync::LazyLock<KeywordSet> = std::sync::LazyLock::new(|| KeywordSet::parse("@,48-57,_,192-255").unwrap());
        self.keyword.as_ref().unwrap_or(&DEFAULT)
    }

    pub(crate) fn is_keyword(&self, character: char) -> bool { self.keywords().contains(character) }

    /// Whether a comma-separated list option contains `item`.
    pub fn has_item(&self, name: &str, item: &str) -> bool {
        self.text(name).split(',').any(|part| part == item)
    }

    /// Whether the option was set (by any script or command).
    pub fn is_set(&self, name: &str) -> bool {
        resolve(name).is_some_and(|resolved| self.values.contains_key(if resolved.name == "t_" { name } else { resolved.name }))
    }

    pub(crate) fn sync_document_policy(&mut self, document: &Document) {
        self.values.insert("readonly".into(), VimOptionValue::Bool(document.is_read_only()));
        self.values.insert("bomb".into(), VimOptionValue::Bool(document.has_bom()));
        self.values.insert("fileencoding".into(), VimOptionValue::Text(match document.encoding() {
            crate::document::Encoding::Utf8 => "utf-8",
            crate::document::Encoding::Latin1 => "latin1",
            crate::document::Encoding::Utf16Le => "utf-16le",
            crate::document::Encoding::Utf16Be => "utf-16",
        }.into()));
    }

    pub(crate) fn apply(&mut self, effect: &VimOptionEffect) {
        if effect.name == "iskeyword" { self.keyword = KeywordSet::parse(&effect.new_value.text()).ok(); }
        self.values.insert(effect.name.clone(), effect.new_value.clone());
    }

    /// `:set` listing lines for values that differ from Vim's defaults.
    pub(crate) fn changed_listing(&self) -> Option<String> {
        let lines: Vec<String> = self
            .values
            .iter()
            .filter(|(name, value)| {
                resolve(name).is_some_and(|resolved| **value != VimOptionValue::parse_default(resolved.kind, resolved.default))
            })
            .map(|(name, value)| value.display(name))
            .collect();
        (!lines.is_empty()).then(|| lines.join("\n"))
    }
}

/// Reuse the validated keyword environment used by bundled Vim syntax.
/// Preserve the editor's established Unicode words until an explicit Vim
/// keyword option supplies a different character set.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(crate) struct KeywordSet(Option<crate::document::syntax::vim::VimKeyword>);

impl KeywordSet {
    pub(crate) fn parse(value: &str) -> Result<Self, String> {
        crate::document::syntax::vim::VimKeyword::parse(value).map(|set| Self(Some(set)))
    }

    pub(crate) fn contains(&self, character: char) -> bool {
        self.0.as_ref().map_or_else(|| character == '_' || character.is_alphanumeric(), |set| set.contains(character))
    }
}

/// Snapshot-local columns avoid whole-document compatibility materialization.
pub(super) fn logical_column(lines: &HardLineSnapshot, at: usize) -> usize {
    let start = line_start(lines, at);
    lines.slice_utf8(start..at).expect("valid caret prefix").graphemes(true).count()
}
pub(super) fn at_logical_column(lines: &HardLineSnapshot, target: usize, column: usize) -> usize {
    let range = lines.line_at_offset(target).expect("valid target line").content_range();
    let mut at = range.start;
    for _ in 0..column {
        let Some(next) = lines.next_grapheme_boundary(at).filter(|next| *next < range.end) else { break; };
        at = next;
    }
    at
}

/// Column arithmetic shared by Normal, Visual and Ex shifts. A left shift
/// first removes a partial level; a right shift starts at the lower level.
pub(super) fn shifted_columns(old: usize, width: usize, amount: usize, right: bool, round: bool) -> Option<usize> {
    let shift = width.checked_mul(amount)?;
    if !round { return if right { old.checked_add(shift) } else { Some(old.saturating_sub(shift)) }; }
    let base = old / width * width;
    if right { base.checked_add(shift) }
    else { Some(base.saturating_sub(shift.saturating_sub(if old != base { width } else { 0 }))) }
}

pub(super) fn join_separator(last: Option<char>, next: Option<char>, joinspaces: bool) -> &'static str {
    if last.is_none_or(|c| matches!(c, ' ' | '\t')) || next.is_none_or(|c| c == ')') { "" }
    else if joinspaces && last.is_some_and(|c| matches!(c, '.' | '?' | '!')) { "  " }
    else { " " }
}

/// A change to an option outside the core `:set` implementation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VimOptionEffect {
    pub name: String,
    pub old_value: VimOptionValue,
    pub new_value: VimOptionValue,
}

fn info(outcome: &mut ExOutcome, text: String) {
    outcome.frontend_requests.push(ExFrontendRequest::Info(ExInfoRequest::Message(text)));
}

fn invalid(argument: String, value: String) -> ExExecuteError {
    ExExecuteError::InvalidOptionValue { option: argument, value }
}

/// Parse a Number option value as Vim does (decimal, `0x` hex, `0` octal).
fn parse_number(text: &str) -> Option<i64> {
    let (value, used) = crate::vimscript::value::str2nr(text.as_bytes(), true);
    (used > 0 && used == text.len()).then_some(value)
}

fn edit_list(old: &str, value: &str, action: &OptionAction, comma: bool, flags: bool) -> String {
    if flags && !comma {
        return match action {
            OptionAction::Append(_) => {
                let mut result = old.to_owned();
                for flag in value.chars().filter(|flag| !old.contains(*flag)) {
                    result.push(flag);
                }
                result
            }
            OptionAction::Prepend(_) => {
                let mut result: String = value.chars().filter(|flag| !old.contains(*flag)).collect();
                result.push_str(old);
                result
            }
            _ => old.chars().filter(|flag| !value.contains(*flag)).collect(),
        };
    }
    if !comma {
        return match action {
            OptionAction::Append(_) => format!("{old}{value}"),
            OptionAction::Prepend(_) => format!("{value}{old}"),
            _ => old.replacen(value, "", 1),
        };
    }
    let present = old.split(',').any(|part| part == value);
    match action {
        OptionAction::Append(_) if present || value.is_empty() => old.to_owned(),
        OptionAction::Prepend(_) if present || value.is_empty() => old.to_owned(),
        OptionAction::Append(_) if old.is_empty() => value.to_owned(),
        OptionAction::Prepend(_) if old.is_empty() => value.to_owned(),
        OptionAction::Append(_) => format!("{old},{value}"),
        OptionAction::Prepend(_) => format!("{value},{old}"),
        _ => old.split(',').filter(|part| *part != value).collect::<Vec<_>>().join(","),
    }
}

/// Validate values for options whose consumers need a well-formed value.
fn validate(name: &str, value: &VimOptionValue) -> Result<(), String> {
    let text = value.text();
    let items = || text.split(',').filter(|item| !item.is_empty());
    let ok = match name {
        "background" => matches!(text.as_str(), "light" | "dark"),
        "breakindentopt" => text.is_empty() || text.strip_prefix("shift:")
            .and_then(|value| value.parse::<u16>().ok()).is_some_and(|shift| shift <= 1024),
        "guifont" => text.is_empty() || text.split(',').all(|font| {
            let mut fields = font.split(':');
            fields.next().is_some_and(|family| !family.is_empty()) && fields.all(|field| {
                matches!(field, "b" | "i") || field.strip_prefix('h')
                    .and_then(|height| height.parse::<f64>().ok())
                    .is_some_and(|height| height.is_finite() && (1.0..=1000.0).contains(&height))
            })
        }),
        "clipboard" => items().all(|item| {
            matches!(item, "unnamed" | "unnamedplus" | "autoselect" | "autoselectplus" | "autoselectml" | "html")
                || item.starts_with("exclude:")
        }),
        "whichwrap" => items().all(|item| matches!(item, "b" | "s" | "h" | "l" | "<" | ">" | "~" | "[" | "]")),
        "iskeyword" => KeywordSet::parse(&text).is_ok(),
        "filetype" | "syntax" => text.bytes().all(|b| b.is_ascii_alphanumeric() || b"_+.#-".contains(&b)),
        "history" => (0..=10000).contains(&value.number()),
        "timeoutlen" => (0..=i32::MAX as i64).contains(&value.number()),
        "scrolloff" => (0..=1_000_000).contains(&value.number()),
        "linespace" => (0..=1000).contains(&value.number()),
        "laststatus" => (0..=3).contains(&value.number()),
        "lines" => (2..=10000).contains(&value.number()),
        "columns" => (12..=10000).contains(&value.number()),
        _ => true,
    };
    if ok { Ok(()) } else { Err(text) }
}

/// Apply one `:set` operation for an option outside the core
/// implementation.
pub(crate) fn apply_operation(
    scope: SetScope,
    operation: &OptionOperation,
    values: &mut VimOptions,
    outcome: &mut ExOutcome,
) -> Result<(), ExExecuteError> {
    let _ = scope;
    let given = operation.name.as_str();
    let resolved = resolve(given).ok_or_else(|| ExExecuteError::UnsupportedOption(given.to_owned()))?;
    if resolved.treatment == Treatment::Core {
        return Err(ExExecuteError::UnsupportedOptionOperation(given.to_owned()));
    }
    let name = if resolved.name == "t_" { given } else { resolved.name };
    let current = values.get(given).expect("resolved options have values");
    let default = VimOptionValue::parse_default(resolved.kind, resolved.default);
    let new = match (&operation.action, resolved.kind) {
        (OptionAction::Query, _) | (OptionAction::Enable, Number | Text) => {
            info(outcome, current.display(name));
            return Ok(());
        }
        (OptionAction::Enable, Bool) => VimOptionValue::Bool(true),
        (OptionAction::Disable, Bool) => VimOptionValue::Bool(false),
        (OptionAction::Toggle, Bool) => VimOptionValue::Bool(current.number() == 0),
        (OptionAction::Reset | OptionAction::Inherit, _) => default.clone(),
        (OptionAction::Disable | OptionAction::Toggle, _) => {
            return Err(invalid(given.to_owned(), "a Boolean action on a value option".into()));
        }
        (OptionAction::Assign(_) | OptionAction::Append(_) | OptionAction::Prepend(_) | OptionAction::Remove(_), Bool) => {
            return Err(invalid(given.to_owned(), "a value for a Boolean option".into()));
        }
        (OptionAction::Assign(text) | OptionAction::Append(text) | OptionAction::Prepend(text) | OptionAction::Remove(text), Number) => {
            let amount = parse_number(text).ok_or_else(|| invalid(given.to_owned(), text.clone()))?;
            let old = current.number();
            VimOptionValue::Number(match &operation.action {
                OptionAction::Assign(_) => amount,
                OptionAction::Append(_) => old.wrapping_add(amount),
                OptionAction::Prepend(_) => old.wrapping_mul(amount),
                _ => old.wrapping_sub(amount),
            })
        }
        (OptionAction::Assign(text), Text) => VimOptionValue::Text(text.clone()),
        (action, Text) => {
            let (OptionAction::Append(text) | OptionAction::Prepend(text) | OptionAction::Remove(text)) = action else {
                unreachable!("other actions are handled above")
            };
            VimOptionValue::Text(edit_list(&current.text(), text, action, COMMA_LISTS.contains(&name), FLAG_LISTS.contains(&name)))
        }
    };
    validate(name, &new).map_err(|value| invalid(name.to_owned(), value))?;
    match resolved.treatment {
        Treatment::Matching(Some(expected)) if new.text() != expected => {
            info(outcome, format!("Viem does not support changing '{name}'"));
        }
        Treatment::Unsupported if new != default && new != current => {
            info(outcome, format!("Viem does not support the '{name}' option"));
        }
        _ => {}
    }
    if new != current || !values.values.contains_key(name) {
        if resolved.treatment == Treatment::Mapped && !matches!(name, "readonly" | "fileencoding" | "bomb") {
            outcome.frontend_requests.push(ExFrontendRequest::Application(ex_execute::ApplicationRequest::Option {
                name: resolved.name, value: new.text(),
            }));
        }
        outcome.vim_option_effects.push(VimOptionEffect { name: name.to_owned(), old_value: current, new_value: new.clone() });
    }
    if name == "iskeyword" { values.keyword = KeywordSet::parse(&new.text()).ok(); }
    values.values.insert(name.to_owned(), new);
    Ok(())
}

fn ex_value(value: &ExOptionValue) -> Value {
    match value {
        ExOptionValue::Boolean(value) => Value::Number(*value as i64),
        ExOptionValue::String(text) => Value::String(text.as_str().into()),
        ExOptionValue::Integer(value) => Value::Number(*value as i64),
        ExOptionValue::Number(value) => Value::Number(*value as i64),
        ExOptionValue::OptionalNumber(value) => Value::Number(value.unwrap_or(0) as i64),
        ExOptionValue::FileFormat(format) => Value::str(file_format_name(*format)),
        ExOptionValue::FileFormats(formats) => {
            Value::String(formats.iter().map(|format| file_format_name(*format)).collect::<Vec<_>>().join(",").into())
        }
        ExOptionValue::Indentation(_) | ExOptionValue::Reflow(_) | ExOptionValue::VisibleWhitespace(_) => Value::str(""),
    }
}

fn file_format_name(format: FileFormat) -> &'static str {
    match format {
        FileFormat::Unix => "unix",
        FileFormat::Dos => "dos",
        FileFormat::Mac => "mac",
    }
}

/// The effective value of an option for a script's `&name`.
pub(crate) fn read_option(commands: &CommandInterpreter, document: &Document, name: &str) -> Option<Value> {
    let resolved = resolve(name)?;
    if resolved.treatment == Treatment::Core {
        let context = commands.option_context();
        return ex_execute::current_option_values(document, &context)
            .into_iter()
            .find(|display| display.name.vim_name() == resolved.name)
            .map(|display| ex_value(&display.value));
    }
    let value = match resolved.name {
        "readonly" => Value::Number(document.is_read_only() as i64),
        "modified" => Value::Number(document.is_dirty() as i64),
        "fileencoding" => Value::str(match document.encoding() {
            crate::document::Encoding::Utf8 => "utf-8",
            crate::document::Encoding::Latin1 => "latin1",
            crate::document::Encoding::Utf16Le => "utf-16le",
            crate::document::Encoding::Utf16Be => "utf-16",
        }),
        "bomb" => Value::Number(document.has_bom() as i64),
        "filetype" | "syntax" if !commands.vim_options.is_set(resolved.name) => {
            Value::String(commands.reflow_options.language().unwrap_or_default().into())
        }
        _ => commands.vim_options.get(name)?.script_value(),
    };
    Some(value)
}

/// The `:set` command equivalent to `let &name = value`.
pub(crate) fn set_command_for(
    commands: &CommandInterpreter,
    document: &Document,
    name: &str,
    scope: OptionScope,
    value: &Value,
) -> EvalResult<String> {
    let _ = (commands, document);
    let resolved = resolve(name).ok_or_else(|| Fail::Error(VimError::new(355, format!("Unknown option: {name}"))))?;
    let command = match scope {
        OptionScope::Both => "set",
        OptionScope::Local => "setlocal",
        OptionScope::Global => "setglobal",
    };
    let target = if resolved.name == "t_" { name } else { resolved.name };
    Ok(match resolved.kind {
        Bool => format!("{command} {}{target}", if value.to_number()? != 0 { "" } else { "no" }),
        Number => format!("{command} {target}={}", value.to_number()?),
        Text => format!("{command} {target}={}", super::scripting::set_argument_value(&value.to_text()?)),
    })
}

/// Vim commands the host implements here rather than in `ex.rs`.
pub(crate) const HOST_COMMANDS: &[&str] = &[
    "colorscheme", "syntax", "filetype", "highlight", "setfiletype", "scriptnames", "winpos", "winsize", "simalt",
    "digraphs", "match", "2match", "3match", "sign", "compiler", "helptags", "menu", "amenu", "nmenu", "vmenu",
    "xmenu", "smenu", "omenu", "imenu", "cmenu", "tmenu", "noremenu", "anoremenu", "nnoremenu", "vnoremenu",
    "xnoremenu", "snoremenu", "onoremenu", "inoremenu", "cnoremenu", "unmenu", "aunmenu", "nunmenu", "vunmenu",
    "xunmenu", "sunmenu", "ounmenu", "iunmenu", "cunmenu", "tunmenu", "macmenu", "popup", "tearoff", "set",
    "setlocal", "setglobal",
];

/// Theme names the frontend offers, for `:colorscheme`. The catalog is
/// process-wide application configuration; the frontend replaces it
/// whenever its themes change.
static THEME_NAMES: std::sync::Mutex<Vec<String>> = std::sync::Mutex::new(Vec::new());

pub(crate) fn set_theme_names(names: Vec<String>) {
    *THEME_NAMES.lock().unwrap_or_else(|poisoned| poisoned.into_inner()) = names;
}

fn find_theme(name: &str) -> Option<String> {
    THEME_NAMES
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .iter()
        .find(|theme| theme.eq_ignore_ascii_case(name))
        .cloned()
}

impl CommandInterpreter {
    pub(super) fn horizontal_key_motion(&self, key: Key) -> Motion {
        let (flag, direction) = match key {
            Key::Backspace => ("b", -1), Key::Char(' ') => ("s", 1),
            Key::Char('h') => ("h", -1), _ => ("l", 1),
        };
        if self.vim_options.has_item("whichwrap", flag) { Motion::ArrowHorizontal(direction) }
        else { Motion::Horizontal(direction) }
    }

    /// `:colorscheme {name}` selects the theme whose name matches, ignoring
    /// case; without a name it reports the current scheme like Vim.
    pub(crate) fn colorscheme_command(&mut self, argument: &str) -> CommandOutput {
        let globals = self.script.globals();
        if argument.is_empty() {
            let name = globals.get_str("colors_name").map(|name| name.echo_string()).unwrap_or_else(|| "default".into());
            return message_output(name);
        }
        if self.native_requests_blocked { return super::scripting::native_request_error(); }
        let Some(theme) = find_theme(argument) else {
            return error_output(format!("E185: Cannot find color scheme '{argument}'"));
        };
        globals.insert("colors_name", Value::String(theme.as_str().into()));
        let mut outcome = ExOutcome::default();
        outcome.frontend_requests.push(ExFrontendRequest::Application(ex_execute::ApplicationRequest::Theme(theme)));
        CommandOutput { ex_outcome: Some(outcome), ..CommandOutput::complete() }
    }
}

fn message_output(text: impl Into<String>) -> CommandOutput {
    let mut outcome = ExOutcome::default();
    info(&mut outcome, text.into());
    CommandOutput { ex_outcome: Some(outcome), ..CommandOutput::complete() }
}

fn error_output(text: impl Into<String>) -> CommandOutput {
    CommandOutput { status: CommandStatus::Error(text.into()), ..CommandOutput::complete() }
}

/// Execute a script command that the Ex parser does not implement.
pub(crate) fn execute_script_only_command(
    commands: &mut CommandInterpreter,
    document: &mut Document,
    name: Option<&str>,
    argument: &str,
) -> Option<CommandOutput> {
    let name = name?;
    let argument = argument.trim();
    Some(match name {
        "syntax" => match argument.split_whitespace().next().unwrap_or("") {
            "" | "on" | "enable" | "manual" | "reset" | "clear" | "sync" | "spell" | "iskeyword" | "case" | "foldlevel"
            | "conceal" | "keyword" | "match" | "region" | "cluster" | "include" | "list" => CommandOutput::complete(),
            "off" => message_output("Viem keeps syntax highlighting on"),
            other => error_output(format!("E410: Invalid :syntax subcommand: {other}")),
        },
        "filetype" => {
            let words: Vec<&str> = argument.split_whitespace().collect();
            if words.is_empty() {
                message_output("filetype detection:ON  plugin:OFF  indent:OFF")
            } else if words.iter().all(|word| matches!(*word, "on" | "off" | "plugin" | "indent" | "detect")) {
                CommandOutput::complete()
            } else {
                error_output(format!("E475: Invalid argument: {argument}"))
            }
        }
        "setfiletype" => {
            if read_option(commands, document, "filetype").is_some_and(|value| !value.echo_string().is_empty()) {
                CommandOutput::complete()
            } else {
                let text = format!("setlocal filetype={}", super::scripting::set_argument_value(argument));
                commands.execute_ex_command(document, &text)
            }
        }
        "scriptnames" => {
            let lines: Vec<String> = commands
                .script
                .scripts()
                .into_iter()
                .enumerate()
                .map(|(index, script)| format!("{:3}: {}", index + 1, script.name))
                .collect();
            if lines.is_empty() { CommandOutput::complete() } else { message_output(lines.join("\n")) }
        }
        "colorscheme" => commands.colorscheme_command(argument),
        "highlight" | "match" | "2match" | "3match" | "sign" | "digraphs" | "compiler" | "helptags" | "winpos"
        | "winsize" | "simalt" | "popup" | "tearoff" => CommandOutput::complete(),
        menu if menu.ends_with("menu") => CommandOutput::complete(),
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolves_abbreviations_and_categories() {
        assert_eq!(resolve("ts").unwrap().name, "tabstop");
        assert_eq!(resolve("sc").unwrap().name, "showcmd");
        assert_eq!(resolve("scs").unwrap().name, "smartcase");
        assert_eq!(resolve("udf").unwrap().treatment, Treatment::Ignored);
        assert_eq!(resolve("gcr").unwrap().treatment, Treatment::Unsupported);
        assert_eq!(resolve("cb").unwrap().treatment, Treatment::Feature);
        assert_eq!(resolve("t_vb").unwrap().treatment, Treatment::Matching(None));
        assert!(resolve("nosuchoption").is_none());
        assert!(option_exists("cursorline", false));
        assert!(!option_exists("cursorline", true));
        assert!(!option_exists("guifontwide", true));
        assert!(option_exists("ttyfast", true));
    }

    #[test]
    fn every_table_entry_is_unique() {
        let mut names = std::collections::HashSet::new();
        for spec in OPTIONS {
            assert!(names.insert(spec.name), "duplicate {}", spec.name);
            assert!(spec.abbreviation.is_empty() || names.insert(spec.abbreviation) || spec.abbreviation == spec.name, "duplicate {}", spec.abbreviation);
        }
        for name in CORE.iter().chain(IGNORED).chain(MAPPED).chain(FEATURES).chain(MATCHING.iter().map(|(name, _)| name)) {
            assert!(OPTIONS.iter().any(|spec| spec.name == *name), "{name} is not a Vim option");
        }
    }

    fn set(values: &mut VimOptions, name: &str, action: OptionAction) -> Result<ExOutcome, ExExecuteError> {
        let mut outcome = ExOutcome::default();
        apply_operation(SetScope::GlobalAndLocal, &OptionOperation { name: name.into(), action }, values, &mut outcome)?;
        Ok(outcome)
    }

    #[test]
    fn list_options_edit_items() {
        let mut values = VimOptions::default();
        set(&mut values, "clipboard", OptionAction::Append("unnamed".into())).unwrap();
        set(&mut values, "cb", OptionAction::Append("unnamedplus".into())).unwrap();
        set(&mut values, "cb", OptionAction::Append("unnamed".into())).unwrap();
        assert_eq!(values.text("clipboard"), "unnamed,unnamedplus");
        set(&mut values, "clipboard", OptionAction::Remove("unnamed".into())).unwrap();
        assert_eq!(values.text("clipboard"), "unnamedplus");
        assert!(set(&mut values, "clipboard", OptionAction::Assign("bogus".into())).is_err());
        set(&mut values, "shortmess", OptionAction::Append("cI".into())).unwrap();
        assert_eq!(values.text("shortmess"), "filnxtToOScI");
        set(&mut values, "history", OptionAction::Assign("0x10".into())).unwrap();
        assert_eq!(values.number("history"), 16);
    }

    #[test]
    fn notices_only_for_differing_unsupported_values() {
        let mut values = VimOptions::default();
        assert!(set(&mut values, "cursorline", OptionAction::Disable).unwrap().frontend_requests.is_empty());
        assert_eq!(set(&mut values, "cursorline", OptionAction::Enable).unwrap().frontend_requests.len(), 1);
        assert!(set(&mut values, "undofile", OptionAction::Enable).unwrap().frontend_requests.is_empty());
        assert!(set(&mut values, "nocompatible", OptionAction::Enable).is_err());
        assert!(set(&mut values, "compatible", OptionAction::Disable).unwrap().frontend_requests.is_empty());
        let query = set(&mut values, "cursorline", OptionAction::Query).unwrap();
        assert_eq!(query.frontend_requests, vec![ExFrontendRequest::Info(ExInfoRequest::Message("  cursorline".into()))]);
        assert!(matches!(set(&mut values, "bogus", OptionAction::Enable), Err(ExExecuteError::UnsupportedOption(_))));
    }
}
