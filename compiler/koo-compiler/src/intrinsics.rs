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
        }
    }

    fn llvm(self) -> &'static str {
        match self {
            I::Unit => "void",
            I::Int => "i64",
            I::Float => "double",
            I::Bool => "i1",
            I::Str | I::Strs | I::Bytes => "ptr",
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
