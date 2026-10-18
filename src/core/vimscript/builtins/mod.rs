//! Builtin functions. Pure functions live here; editor functions such as
//! `getline()` are provided by the host. Functions Vim has but Viem does not
//! implement report "Unknown function ... (not supported by Viem)".
use super::error::{EvalResult, Fail, VimError};
use super::exec::{Runtime, ScriptHost};
use super::value::{VStr, Value};

mod collections;
mod script;
mod system;
mod text;

// Bound individual generated containers before allocating their contents.
pub(crate) const MAX_LIST_ITEMS: usize = 1_000_000;
pub(crate) const MAX_STRING_BYTES: usize = 64 * 1024 * 1024;

/// Every Vim 9.1 builtin function name.
const VIM_BUILTINS: &[&str] = &[
    "abs", "acos", "add", "and", "append", "appendbufline", "argc", "argidx", "arglistid", "argv",
    "asin", "assert_beeps", "assert_equal", "assert_equalfile", "assert_exception", "assert_fails",
    "assert_false", "assert_inrange", "assert_match", "assert_nobeep", "assert_notequal",
    "assert_notmatch", "assert_report", "assert_true", "atan", "atan2", "autocmd_add",
    "autocmd_delete", "autocmd_get", "balloon_gettext", "balloon_show", "balloon_split",
    "base64_decode", "base64_encode", "bindtextdomain", "blob2list", "blob2str", "browse",
    "browsedir", "bufadd", "bufexists", "buflisted", "bufload", "bufloaded", "bufname", "bufnr",
    "bufwinid", "bufwinnr", "byte2line", "byteidx", "byteidxcomp", "call", "ceil", "ch_canread",
    "ch_close", "ch_close_in", "ch_evalexpr", "ch_evalraw", "ch_getbufnr", "ch_getjob", "ch_info",
    "ch_log", "ch_logfile", "ch_open", "ch_read", "ch_readblob", "ch_readraw", "ch_sendexpr",
    "ch_sendraw", "ch_setoptions", "ch_status", "changenr", "char2nr", "charclass", "charcol",
    "charidx", "chdir", "cindent", "clearmatches", "cmdcomplete_info", "col", "complete",
    "complete_add", "complete_check", "complete_info", "complete_match", "confirm", "copy", "cos",
    "cosh", "count", "cscope_connection", "cursor", "debugbreak", "deepcopy", "delete",
    "deletebufline", "did_filetype", "diff", "diff_filler", "digraph_get", "digraph_getlist",
    "digraph_set", "digraph_setlist", "echoraw", "empty", "environ", "err_teapot", "escape",
    "eval", "eventhandler", "executable", "execute", "exepath", "exists", "exists_compiled", "exp",
    "expand", "expandcmd", "extend", "extendnew", "feedkeys", "filecopy", "filereadable",
    "filewritable", "filter", "finddir", "findfile", "flatten", "flattennew", "float2nr", "floor",
    "fmod", "fnameescape", "fnamemodify", "foldclosed", "foldclosedend", "foldlevel", "foldtext",
    "foldtextresult", "foreach", "foreground", "fullcommand", "funcref", "function",
    "garbagecollect", "get", "getbufinfo", "getbufline", "getbufoneline", "getbufvar",
    "getcellpixels", "getcellwidths", "getchangelist", "getchar", "getcharmod", "getcharpos",
    "getcharsearch", "getcharstr", "getcmdcomplpat", "getcmdcompltype", "getcmdline", "getcmdpos",
    "getcmdprompt", "getcmdscreenpos", "getcmdtype", "getcmdwintype", "getcompletion",
    "getcompletiontype", "getcurpos", "getcursorcharpos", "getcwd", "getenv", "getfontname",
    "getfperm", "getfsize", "getftime", "getftype", "getimstatus", "getjumplist", "getline",
    "getloclist", "getmarklist", "getmatches", "getmousepos", "getmouseshape", "getpid", "getpos",
    "getqflist", "getreg", "getreginfo", "getregion", "getregionpos", "getregtype",
    "getscriptinfo", "getstacktrace", "gettabinfo", "gettabvar", "gettabwinvar", "gettagstack",
    "gettext", "getwininfo", "getwinpos", "getwinposx", "getwinposy", "getwinvar", "glob",
    "glob2regpat", "globpath", "has", "has_key", "haslocaldir", "hasmapto", "histadd", "histdel",
    "histget", "histnr", "hlexists", "hlget", "hlset", "hostname", "iconv", "id", "indent",
    "index", "indexof", "input", "inputdialog", "inputlist", "inputrestore", "inputsave",
    "inputsecret", "insert", "instanceof", "interrupt", "invert", "isabsolutepath", "isdirectory",
    "isinf", "islocked", "isnan", "items", "job_getchannel", "job_info", "job_setoptions",
    "job_start", "job_status", "job_stop", "join", "js_decode", "js_encode", "json_decode",
    "json_encode", "keys", "keytrans", "len", "libcall", "libcallnr", "line", "line2byte",
    "lispindent", "list2blob", "list2str", "list2tuple", "listener_add", "listener_flush",
    "listener_remove", "localtime", "log", "log10", "luaeval", "map", "maparg", "mapcheck",
    "maplist", "mapnew", "mapset", "match", "matchadd", "matchaddpos", "matcharg", "matchbufline",
    "matchdelete", "matchend", "matchfuzzy", "matchfuzzypos", "matchlist", "matchstr",
    "matchstrlist", "matchstrpos", "max", "menu_info", "min", "mkdir", "mode", "mzeval",
    "nextnonblank", "ngettext", "nr2char", "or", "pathshorten", "perleval", "popup_atcursor",
    "popup_beval", "popup_clear", "popup_close", "popup_create", "popup_dialog",
    "popup_filter_menu", "popup_filter_yesno", "popup_findecho", "popup_findinfo",
    "popup_findpreview", "popup_getoptions", "popup_getpos", "popup_hide", "popup_list",
    "popup_locate", "popup_menu", "popup_move", "popup_notification", "popup_setbuf",
    "popup_setoptions", "popup_settext", "popup_show", "pow", "prevnonblank", "printf",
    "prompt_getprompt", "prompt_setcallback", "prompt_setinterrupt", "prompt_setprompt",
    "prop_add", "prop_add_list", "prop_clear", "prop_find", "prop_list", "prop_remove",
    "prop_type_add", "prop_type_change", "prop_type_delete", "prop_type_get", "prop_type_list",
    "pum_getpos", "pumvisible", "py3eval", "pyeval", "pyxeval", "rand", "range", "readblob",
    "readdir", "readdirex", "readfile", "reduce", "reg_executing", "reg_recording", "reltime",
    "reltimefloat", "reltimestr", "remote_expr", "remote_foreground", "remote_peek", "remote_read",
    "remote_send", "remote_startserver", "remove", "rename", "repeat", "resolve", "reverse",
    "round", "rubyeval", "screenattr", "screenchar", "screenchars", "screencol", "screenpos",
    "screenrow", "screenstring", "search", "searchcount", "searchdecl", "searchpair",
    "searchpairpos", "searchpos", "server2client", "serverlist", "setbufline", "setbufvar",
    "setcellwidths", "setcharpos", "setcharsearch", "setcmdline", "setcmdpos", "setcursorcharpos",
    "setenv", "setfperm", "setline", "setloclist", "setmatches", "setpos", "setqflist", "setreg",
    "settabvar", "settabwinvar", "settagstack", "setwinvar", "sha256", "shellescape", "shiftwidth",
    "sign_define", "sign_getdefined", "sign_getplaced", "sign_jump", "sign_place",
    "sign_placelist", "sign_undefine", "sign_unplace", "sign_unplacelist", "simplify", "sin",
    "sinh", "slice", "sort", "sound_clear", "sound_playevent", "sound_playfile", "sound_stop",
    "soundfold", "spellbadword", "spellsuggest", "split", "sqrt", "srand", "state", "str2blob",
    "str2float", "str2list", "str2nr", "strcharlen", "strcharpart", "strchars", "strdisplaywidth",
    "strftime", "strgetchar", "stridx", "string", "strlen", "strpart", "strptime", "strridx",
    "strtrans", "strutf16len", "strwidth", "submatch", "substitute", "swapfilelist", "swapinfo",
    "swapname", "synconcealed", "synstack", "system", "systemlist", "tabpagebuflist", "tabpagenr",
    "tabpagewinnr", "tagfiles", "taglist", "tan", "tanh", "tempname", "term_dumpdiff",
    "term_dumpload", "term_dumpwrite", "term_getaltscreen", "term_getansicolors", "term_getattr",
    "term_getcursor", "term_getjob", "term_getline", "term_getscrolled", "term_getsize",
    "term_getstatus", "term_gettitle", "term_gettty", "term_list", "term_scrape", "term_sendkeys",
    "term_setansicolors", "term_setapi", "term_setkill", "term_setrestore", "term_setsize",
    "term_start", "term_wait", "terminalprops", "test_alloc_fail", "test_autochdir",
    "test_feedinput", "test_garbagecollect_now", "test_garbagecollect_soon", "test_getvalue",
    "test_gui_event", "test_ignore_error", "test_mswin_event", "test_null_blob",
    "test_null_channel", "test_null_dict", "test_null_function", "test_null_job", "test_null_list",
    "test_null_partial", "test_null_string", "test_null_tuple", "test_option_not_set",
    "test_override", "test_refcount", "test_setmouse", "test_settime", "test_srand_seed",
    "test_unknown", "test_void", "timer_info", "timer_pause", "timer_start", "timer_stop",
    "timer_stopall", "tolower", "toupper", "tr", "trim", "trunc", "tuple2list", "type", "typename",
    "undofile", "undotree", "uniq", "uri_decode", "uri_encode", "utf16idx", "values", "virtcol",
    "virtcol2col", "visualmode", "wildmenumode", "wildtrigger", "win_execute", "win_findbuf",
    "win_getid", "win_gettype", "win_gotoid", "win_id2tabwin", "win_id2win", "win_move_separator",
    "win_move_statusline", "win_screenpos", "win_splitmove", "winbufnr", "wincol",
    "windowsversion", "winheight", "winlayout", "winline", "winnr", "winrestcmd", "winrestview",
    "winsaveview", "winwidth", "wordcount", "writefile", "xor",];

pub fn is_vim_builtin(name: &str) -> bool {
    VIM_BUILTINS.contains(&name)
}

/// Base argument position for `base->name()` method calls (0-based).
pub fn method_base_position(name: &[u8]) -> usize {
    match name {
        b"printf" | b"setline" | b"append" | b"setreg" | b"win_execute" | b"timer_start" | b"setqflist"
        | b"assert_equal" | b"assert_notequal" | b"assert_match" | b"assert_notmatch" => 1,
        b"appendbufline" | b"setbufline" | b"setbufvar" | b"settabvar" | b"setwinvar" | b"libcall" | b"libcallnr"
        | b"assert_inrange" => 2,
        b"settabwinvar" => 3,
        _ => 0,
    }
}

pub(crate) fn arity(name: &str, args: &[Value], min: usize, max: usize) -> EvalResult<()> {
    if args.len() < min {
        return Err(Fail::Error(VimError::argument(119, name)));
    }
    if args.len() > max {
        return Err(Fail::Error(VimError::argument(118, name)));
    }
    Ok(())
}

pub(crate) fn number(args: &[Value], index: usize) -> EvalResult<i64> {
    match args.get(index) {
        Some(value) => Ok(value.to_number()?),
        None => Ok(0),
    }
}

pub(crate) fn opt_number(args: &[Value], index: usize, default: i64) -> EvalResult<i64> {
    match args.get(index) {
        Some(value) => Ok(value.to_number()?),
        None => Ok(default),
    }
}

pub(crate) fn vstr(args: &[Value], index: usize) -> EvalResult<VStr> {
    match args.get(index) {
        Some(value) => Ok(value.to_vstr()?),
        None => Ok(VStr::empty()),
    }
}

pub(crate) fn text(args: &[Value], index: usize) -> EvalResult<String> {
    Ok(vstr(args, index)?.to_str_lossy().into_owned())
}

pub(crate) fn opt_bool(args: &[Value], index: usize, default: bool) -> EvalResult<bool> {
    match args.get(index) {
        Some(value) => Ok(value.to_bool()?),
        None => Ok(default),
    }
}

/// Call a callback given as a Funcref or a function name.
pub(crate) fn call_callback(rt: &mut Runtime, host: &mut dyn ScriptHost, callback: &Value, args: Vec<Value>) -> EvalResult<Value> {
    match callback {
        Value::Funcref(funcref) => rt.call_funcref(host, funcref, args, None),
        Value::String(name) => {
            let funcref = rt.resolve_function_name(host, name.as_bytes())?;
            rt.call_funcref(host, &funcref, args, None)
        }
        _ => Err(Fail::Error(VimError::new(921, "Invalid callback argument"))),
    }
}

/// Like Vim, an error in a builtin's callback is reported at once and the
/// builtin stops early but still returns its result. Inside `:try` the
/// error propagates to become an exception.
pub(crate) fn callback_failed(rt: &mut Runtime, host: &mut dyn ScriptHost, fail: Fail) -> EvalResult<()> {
    match fail {
        Fail::Error(error) if rt.trylevel == 0 && rt.emsg_silent == 0 => {
            rt.report(host, error);
            Ok(())
        }
        Fail::Error(error) if rt.emsg_silent > 0 => {
            rt.state.set_vvar("errmsg", error.text().into());
            rt.did_emsg = true;
            Ok(())
        }
        other => Err(other),
    }
}

/// Dispatch an engine builtin. None means `name` is not one.
pub(crate) fn call(rt: &mut Runtime, host: &mut dyn ScriptHost, name: &str, args: Vec<Value>) -> Option<EvalResult<Value>> {
    if let Some(result) = text::call(rt, host, name, &args) {
        return Some(result);
    }
    if let Some(result) = collections::call(rt, host, name, &args) {
        return Some(result);
    }
    if let Some(result) = script::call(rt, host, name, &args) {
        return Some(result);
    }
    if let Some(result) = system::call(rt, host, name, &args) {
        return Some(result);
    }
    None
}

/// Whether the engine implements `name` (for `exists('*name')`).
pub fn is_implemented(name: &str) -> bool {
    text::NAMES.contains(&name) || collections::NAMES.contains(&name) || script::NAMES.contains(&name) || system::NAMES.contains(&name)
}
