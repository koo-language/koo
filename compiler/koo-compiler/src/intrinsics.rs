//! The functions of the support library that the prelude calls by name.
//!
//! The prelude (`prelude.koo`) is written in Koo. Where it needs something that Koo cannot
//! express yet, such as looking at the bytes of a string, it calls one of these as
//! `__name(args)`. Only the prelude may: the names are not part of the language.

use crate::hir::Ty;

/// The types an intrinsic may take and return.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum I {
    Unit,
    Int,
    Float,
    Bool,
    Str,
    Strs,
    Bytes,
    /// A value of the program's that the support library keeps and hands back without
    /// knowing its type: the state of an actor, or what waits to be run. The checker
    /// gives it the type that is expected of it.
    Cell,
    /// Like `Cell`, and the support library keeps it: the program's variable is left
    /// empty, and does not free it.
    Given,
}

impl I {
    pub fn ty(self) -> Ty {
        match self {
            I::Unit => Ty::Unit,
            I::Int => Ty::Int,
            I::Float => Ty::Float,
            I::Bool => Ty::Bool,
            I::Str => Ty::Str,
            I::Strs => Ty::list(Ty::Str),
            I::Bytes => Ty::list(Ty::Byte),
            I::Cell | I::Given => Ty::Unknown,
        }
    }

    fn llvm(self) -> &'static str {
        match self {
            I::Unit => "void",
            I::Int => "i64",
            I::Float => "double",
            I::Bool => "i1",
            I::Str | I::Strs | I::Bytes | I::Cell | I::Given => "ptr",
        }
    }
}

pub struct Intrinsic {
    /// The name in the prelude, without the leading `__`. The runtime function is `koo_` + name.
    pub name: &'static str,
    pub params: &'static [I],
    pub ret: I,
}

pub const INTRINSICS: &[Intrinsic] = &[
    Intrinsic {
        name: "str_find",
        params: &[I::Str, I::Str, I::Int],
        ret: I::Int,
    },
    Intrinsic {
        name: "str_starts_with",
        params: &[I::Str, I::Str],
        ret: I::Bool,
    },
    Intrinsic {
        name: "str_ends_with",
        params: &[I::Str, I::Str],
        ret: I::Bool,
    },
    Intrinsic {
        name: "str_slice",
        params: &[I::Str, I::Int, I::Int],
        ret: I::Str,
    },
    Intrinsic {
        name: "str_trim",
        params: &[I::Str],
        ret: I::Str,
    },
    Intrinsic {
        name: "str_upper",
        params: &[I::Str],
        ret: I::Str,
    },
    Intrinsic {
        name: "str_lower",
        params: &[I::Str],
        ret: I::Str,
    },
    Intrinsic {
        name: "str_replace",
        params: &[I::Str, I::Str, I::Str],
        ret: I::Str,
    },
    Intrinsic {
        name: "str_split",
        params: &[I::Str, I::Str],
        ret: I::Strs,
    },
    Intrinsic {
        name: "str_repeat",
        params: &[I::Str, I::Int],
        ret: I::Str,
    },
    Intrinsic {
        name: "str_count",
        params: &[I::Str],
        ret: I::Int,
    },
    Intrinsic {
        name: "str_bytes",
        params: &[I::Str],
        ret: I::Bytes,
    },
    Intrinsic {
        name: "bytes_valid_utf8",
        params: &[I::Bytes],
        ret: I::Bool,
    },
    Intrinsic {
        name: "bytes_to_str",
        params: &[I::Bytes],
        ret: I::Str,
    },
    Intrinsic {
        name: "args",
        params: &[],
        ret: I::Strs,
    },
    Intrinsic {
        name: "env",
        params: &[I::Str],
        ret: I::Str,
    },
    Intrinsic {
        name: "env_has",
        params: &[I::Str],
        ret: I::Bool,
    },
    Intrinsic {
        name: "read_line",
        params: &[],
        ret: I::Str,
    },
    Intrinsic {
        name: "at_end",
        params: &[],
        ret: I::Bool,
    },
    Intrinsic {
        name: "read_file",
        params: &[I::Str],
        ret: I::Str,
    },
    Intrinsic {
        name: "write_file",
        params: &[I::Str, I::Str],
        ret: I::Bool,
    },
    Intrinsic {
        name: "last_error",
        params: &[],
        ret: I::Str,
    },
    Intrinsic {
        name: "failed",
        params: &[],
        ret: I::Bool,
    },
    Intrinsic {
        name: "eprint",
        params: &[I::Str],
        ret: I::Unit,
    },
    Intrinsic {
        name: "write",
        params: &[I::Str],
        ret: I::Unit,
    },
    Intrinsic {
        name: "exit_now",
        params: &[I::Int],
        ret: I::Unit,
    },
    Intrinsic {
        name: "now_nanos",
        params: &[],
        ret: I::Int,
    },
    // Digests, bytes as text, and randomness (`koo-rt/src/codec.rs`).
    Intrinsic {
        name: "sha256",
        params: &[I::Bytes],
        ret: I::Bytes,
    },
    Intrinsic {
        name: "hmac_sha256",
        params: &[I::Bytes, I::Bytes],
        ret: I::Bytes,
    },
    Intrinsic {
        name: "pbkdf2_sha256",
        params: &[I::Bytes, I::Bytes, I::Int],
        ret: I::Bytes,
    },
    Intrinsic {
        name: "bytes_same",
        params: &[I::Bytes, I::Bytes],
        ret: I::Bool,
    },
    Intrinsic {
        name: "hex",
        params: &[I::Bytes],
        ret: I::Str,
    },
    Intrinsic {
        name: "hex_valid",
        params: &[I::Str],
        ret: I::Bool,
    },
    Intrinsic {
        name: "hex_decode",
        params: &[I::Str],
        ret: I::Bytes,
    },
    Intrinsic {
        name: "base64",
        params: &[I::Bytes, I::Bool],
        ret: I::Str,
    },
    Intrinsic {
        name: "base64_valid",
        params: &[I::Str, I::Bool],
        ret: I::Bool,
    },
    Intrinsic {
        name: "base64_decode",
        params: &[I::Str, I::Bool],
        ret: I::Bytes,
    },
    Intrinsic {
        name: "url_encode",
        params: &[I::Str],
        ret: I::Str,
    },
    Intrinsic {
        name: "url_valid",
        params: &[I::Str],
        ret: I::Bool,
    },
    Intrinsic {
        name: "url_decode",
        params: &[I::Str],
        ret: I::Str,
    },
    Intrinsic {
        name: "random_bytes",
        params: &[I::Int],
        ret: I::Bytes,
    },
    Intrinsic {
        name: "random_below",
        params: &[I::Int],
        ret: I::Int,
    },
    Intrinsic {
        name: "random_float",
        params: &[],
        ret: I::Float,
    },
    // Databases (`std/sql`, `koo-rt/src/pg.rs`).
    Intrinsic {
        name: "sql_open",
        params: &[I::Str, I::Int],
        ret: I::Int,
    },
    Intrinsic {
        name: "sql_run",
        params: &[I::Int, I::Int, I::Str, I::Strs, I::Str],
        ret: I::Bool,
    },
    Intrinsic {
        name: "sql_begin",
        params: &[I::Int, I::Int],
        ret: I::Int,
    },
    Intrinsic {
        name: "sql_end",
        params: &[I::Int, I::Int, I::Bool],
        ret: I::Bool,
    },
    Intrinsic {
        name: "sql_rows",
        params: &[],
        ret: I::Int,
    },
    Intrinsic {
        name: "sql_cols",
        params: &[],
        ret: I::Int,
    },
    Intrinsic {
        name: "sql_affected",
        params: &[],
        ret: I::Int,
    },
    Intrinsic {
        name: "sql_name",
        params: &[I::Int],
        ret: I::Str,
    },
    Intrinsic {
        name: "sql_value",
        params: &[I::Int, I::Int],
        ret: I::Str,
    },
    Intrinsic {
        name: "sql_null",
        params: &[I::Int, I::Int],
        ret: I::Bool,
    },
    Intrinsic {
        name: "sql_error",
        params: &[],
        ret: I::Str,
    },
    Intrinsic {
        name: "sql_code",
        params: &[],
        ret: I::Str,
    },
    Intrinsic {
        name: "sql_kind",
        params: &[],
        ret: I::Int,
    },
    // The clock (`std/time`).
    Intrinsic {
        name: "time_now",
        params: &[],
        ret: I::Int,
    },
    Intrinsic {
        name: "uptime",
        params: &[],
        ret: I::Int,
    },
    Intrinsic {
        name: "sleep",
        params: &[I::Int],
        ret: I::Unit,
    },
    // Reading and writing JSON.
    Intrinsic {
        name: "json_ws",
        params: &[I::Str, I::Int],
        ret: I::Int,
    },
    Intrinsic {
        name: "str_byte",
        params: &[I::Str, I::Int],
        ret: I::Int,
    },
    Intrinsic {
        name: "json_string",
        params: &[I::Str, I::Int],
        ret: I::Int,
    },
    Intrinsic {
        name: "json_text",
        params: &[],
        ret: I::Str,
    },
    Intrinsic {
        name: "json_number",
        params: &[I::Str, I::Int],
        ret: I::Int,
    },
    Intrinsic {
        name: "json_is_int",
        params: &[],
        ret: I::Bool,
    },
    Intrinsic {
        name: "json_int",
        params: &[],
        ret: I::Int,
    },
    Intrinsic {
        name: "json_float",
        params: &[],
        ret: I::Float,
    },
    Intrinsic {
        name: "json_word",
        params: &[I::Str, I::Int, I::Str],
        ret: I::Int,
    },
    Intrinsic {
        name: "json_skip",
        params: &[I::Str, I::Int],
        ret: I::Int,
    },
    Intrinsic {
        name: "json_tag",
        params: &[I::Str, I::Int, I::Str],
        ret: I::Bool,
    },
    Intrinsic {
        name: "json_quote",
        params: &[I::Str],
        ret: I::Str,
    },
    Intrinsic {
        name: "json_begin",
        params: &[],
        ret: I::Unit,
    },
    Intrinsic {
        name: "json_enter",
        params: &[],
        ret: I::Bool,
    },
    Intrinsic {
        name: "json_leave",
        params: &[],
        ret: I::Unit,
    },
    // Actors (`koo-rt/src/actor.rs`).
    Intrinsic {
        name: "actor_new",
        params: &[I::Str, I::Cell, I::Int],
        ret: I::Int,
    },
    Intrinsic {
        name: "actor_take",
        params: &[I::Int, I::Str],
        ret: I::Cell,
    },
    Intrinsic {
        name: "actor_put",
        params: &[I::Int, I::Given],
        ret: I::Unit,
    },
    Intrinsic {
        name: "actor_tell",
        params: &[I::Int, I::Given, I::Str],
        ret: I::Unit,
    },
    Intrinsic {
        name: "actor_try_tell",
        params: &[I::Int, I::Given],
        ret: I::Bool,
    },
    Intrinsic {
        name: "actor_letter",
        params: &[I::Int, I::Int],
        ret: I::Unit,
    },
    Intrinsic {
        name: "actor_alive",
        params: &[I::Int],
        ret: I::Bool,
    },
    Intrinsic {
        name: "actor_end",
        params: &[I::Int, I::Int],
        ret: I::Cell,
    },
    Intrinsic {
        name: "actor_scan",
        params: &[],
        ret: I::Unit,
    },
    Intrinsic {
        name: "actor_any",
        params: &[I::Str],
        ret: I::Int,
    },
    Intrinsic {
        name: "freeze",
        params: &[I::Cell],
        ret: I::Unit,
    },
    Intrinsic {
        name: "guard",
        params: &[],
        ret: I::Bool,
    },
    Intrinsic {
        name: "guard_done",
        params: &[],
        ret: I::Unit,
    },
    Intrinsic {
        name: "actor_recover",
        params: &[],
        ret: I::Unit,
    },
    Intrinsic {
        name: "actor_home",
        params: &[],
        ret: I::Str,
    },
    Intrinsic {
        name: "actor_local",
        params: &[I::Str],
        ret: I::Bool,
    },
    Intrinsic {
        name: "actor_message",
        params: &[],
        ret: I::Str,
    },
    Intrinsic {
        name: "actor_body",
        params: &[],
        ret: I::Str,
    },
    Intrinsic {
        name: "actor_reply",
        params: &[I::Str, I::Str],
        ret: I::Unit,
    },
    Intrinsic {
        name: "actor_hang_up",
        params: &[],
        ret: I::Unit,
    },
    Intrinsic {
        name: "actor_ask",
        params: &[I::Str, I::Str, I::Str],
        ret: I::Str,
    },
    Intrinsic {
        name: "actor_kind",
        params: &[],
        ret: I::Str,
    },
    Intrinsic {
        name: "actor_send",
        params: &[I::Str, I::Str, I::Str],
        ret: I::Unit,
    },
    // The web server (`koo-rt/src/http.rs`).
    Intrinsic {
        name: "http_reset",
        params: &[],
        ret: I::Unit,
    },
    Intrinsic {
        name: "http_route",
        params: &[I::Str, I::Str],
        ret: I::Unit,
    },
    Intrinsic {
        name: "http_listen",
        params: &[I::Int, I::Int, I::Int, I::Int],
        ret: I::Int,
    },
    Intrinsic {
        name: "http_next",
        params: &[],
        ret: I::Int,
    },
    Intrinsic {
        name: "http_id",
        params: &[],
        ret: I::Int,
    },
    Intrinsic {
        name: "http_method",
        params: &[I::Int],
        ret: I::Str,
    },
    Intrinsic {
        name: "http_path",
        params: &[I::Int],
        ret: I::Str,
    },
    Intrinsic {
        name: "http_rest",
        params: &[I::Int],
        ret: I::Str,
    },
    Intrinsic {
        name: "http_param",
        params: &[I::Int, I::Str],
        ret: I::Str,
    },
    Intrinsic {
        name: "http_query_has",
        params: &[I::Int, I::Str],
        ret: I::Bool,
    },
    Intrinsic {
        name: "http_query",
        params: &[I::Int, I::Str],
        ret: I::Str,
    },
    Intrinsic {
        name: "http_form_has",
        params: &[I::Int, I::Str],
        ret: I::Bool,
    },
    Intrinsic {
        name: "http_form",
        params: &[I::Int, I::Str],
        ret: I::Str,
    },
    Intrinsic {
        name: "http_cookie_has",
        params: &[I::Int, I::Str],
        ret: I::Bool,
    },
    Intrinsic {
        name: "http_cookie",
        params: &[I::Int, I::Str],
        ret: I::Str,
    },
    Intrinsic {
        name: "http_header_has",
        params: &[I::Int, I::Str],
        ret: I::Bool,
    },
    Intrinsic {
        name: "http_header",
        params: &[I::Int, I::Str],
        ret: I::Str,
    },
    Intrinsic {
        name: "http_body",
        params: &[I::Int],
        ret: I::Str,
    },
    Intrinsic {
        name: "http_respond",
        params: &[I::Int, I::Int, I::Str, I::Str, I::Str],
        ret: I::Unit,
    },
    Intrinsic {
        name: "http_respond_file",
        params: &[I::Int, I::Int, I::Str, I::Str, I::Str],
        ret: I::Unit,
    },
];

pub fn find(name: &str) -> Option<&'static Intrinsic> {
    INTRINSICS.iter().find(|i| i.name == name)
}

/// The LLVM declarations of the intrinsics.
pub fn declarations() -> String {
    let mut out = String::new();
    for intrinsic in INTRINSICS {
        let params: Vec<String> = intrinsic
            .params
            .iter()
            .map(|p| match p {
                I::Bool => "i1 zeroext".to_string(),
                other => other.llvm().to_string(),
            })
            .collect();
        let ret = match intrinsic.ret {
            I::Bool => "zeroext i1".to_string(),
            I::Str | I::Strs | I::Bytes => "nonnull ptr".to_string(),
            other => other.llvm().to_string(),
        };
        out.push_str(&format!(
            "declare {ret} @koo_{}({})\n",
            intrinsic.name,
            params.join(", ")
        ));
    }
    out
}
