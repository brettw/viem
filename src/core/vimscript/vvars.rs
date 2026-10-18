//! Predefined `v:` variables.
use super::value::{Dict, DictItem, Lock, VStr, Value, VAR_BLOB, VAR_BOOL, VAR_DICT, VAR_FLOAT, VAR_FUNC, VAR_LIST, VAR_NUMBER, VAR_SPECIAL, VAR_STRING};

/// The Vim version Viem's legacy script engine follows.
pub const VERSION: i64 = 902;
pub const VERSION_LONG: i64 = 9_020_000;

/// `v:` variables a script may assign; all others are read-only.
const WRITABLE: &[&str] = &[
    "char", "errmsg", "warningmsg", "statusmsg", "hlsearch", "searchforward", "oldfiles",
    "swapchoice", "swapcommand", "fcs_choice", "scrollstart", "errors", "this_session", "testing",
    "colornames", "lnum",
];

pub fn writable(name: &str) -> bool {
    WRITABLE.contains(&name)
}

pub fn initialize(vars: &Dict) {
    let entries: Vec<(&str, Value)> = vec![
        ("true", Value::Bool(true)),
        ("false", Value::Bool(false)),
        ("none", Value::None),
        ("null", Value::Null),
        ("version", VERSION.into()),
        ("versionlong", VERSION_LONG.into()),
        ("numbersize", 64.into()),
        ("numbermax", i64::MAX.into()),
        ("numbermin", i64::MIN.into()),
        ("maxcol", 2_147_483_647.into()),
        ("sizeofint", 4.into()),
        ("sizeoflong", 8.into()),
        ("sizeofpointer", 8.into()),
        ("t_number", VAR_NUMBER.into()),
        ("t_string", VAR_STRING.into()),
        ("t_func", VAR_FUNC.into()),
        ("t_list", VAR_LIST.into()),
        ("t_dict", VAR_DICT.into()),
        ("t_float", VAR_FLOAT.into()),
        ("t_bool", VAR_BOOL.into()),
        ("t_none", VAR_SPECIAL.into()),
        ("t_job", 8.into()),
        ("t_channel", 9.into()),
        ("t_blob", VAR_BLOB.into()),
        ("t_class", 12.into()),
        ("t_object", 13.into()),
        ("t_typealias", 14.into()),
        ("t_enum", 15.into()),
        ("t_enumvalue", 16.into()),
        ("t_tuple", 17.into()),
        ("count", 0.into()),
        ("count1", 1.into()),
        ("prevcount", 0.into()),
        ("register", "\"".into()),
        ("errmsg", "".into()),
        ("warningmsg", "".into()),
        ("statusmsg", "".into()),
        ("exception", "".into()),
        ("throwpoint", "".into()),
        ("errors", Value::list(Vec::new())),
        ("event", Value::Dict(Dict::new())),
        ("char", "".into()),
        ("lnum", 0.into()),
        ("hlsearch", 0.into()),
        ("searchforward", 1.into()),
        ("insertmode", "i".into()),
        ("operator", "".into()),
        ("progname", "viem".into()),
        ("progpath", "viem".into()),
        ("lang", "C".into()),
        ("lc_time", "C".into()),
        ("ctype", "C".into()),
        ("collate", "C".into()),
        ("servername", "".into()),
        ("this_session", "".into()),
        ("shell_error", 0.into()),
        ("dying", 0.into()),
        ("exiting", Value::Null),
        ("vim_did_enter", 0.into()),
        ("vim_did_init", 0.into()),
        ("testing", 0.into()),
        ("profiling", 0.into()),
        ("oldfiles", Value::list(Vec::new())),
        ("argv", Value::list(Vec::new())),
        ("key", "".into()),
        ("val", "".into()),
        ("cmdarg", "".into()),
        ("cmdbang", 0.into()),
        ("option_new", "".into()),
        ("option_old", "".into()),
        ("option_oldlocal", "".into()),
        ("option_oldglobal", "".into()),
        ("option_type", "".into()),
        ("option_command", "".into()),
        ("completed_item", Value::Dict(Dict::new())),
        ("colornames", Value::Dict(Dict::new())),
        ("echospace", 80.into()),
        ("mouse_win", 0.into()),
        ("mouse_winid", 0.into()),
        ("mouse_lnum", 0.into()),
        ("mouse_col", 0.into()),
        ("scrollstart", "".into()),
        ("swapname", "".into()),
        ("swapchoice", "".into()),
        ("swapcommand", "".into()),
        ("fcs_reason", "".into()),
        ("fcs_choice", "".into()),
        ("folddashes", "".into()),
        ("foldlevel", 0.into()),
        ("foldstart", 0.into()),
        ("foldend", 0.into()),
        ("windowid", 0.into()),
        ("wrap", 0.into()),
        ("stacktrace", Value::list(Vec::new())),
    ];
    let mut data = vars.data();
    for (name, value) in entries {
        let lock = if writable(name) { Lock::Unlocked } else { Lock::Fixed };
        data.items.insert(VStr::from(name), DictItem { value, lock });
    }
    data.lock = Lock::Fixed;
}
