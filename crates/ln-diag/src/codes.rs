//! Registry of every scoped diagnostic code the compiler can emit.
//!
//! Codes are grouped by scope (`syntax`, `name`, `type`, ...) and rendered as
//! `err(scope::name)`.

use crate::Code;

macro_rules! codes {
    ($( $scope:ident ($sname:literal) => { $( $(#[$m:meta])* $konst:ident = $name:literal ),* $(,)? } )*) => {
        $(
            #[allow(non_snake_case)]
            pub mod $scope {
                use super::Code;
                $( $(#[$m])* pub const $konst: Code = Code::new($sname, $name); )*
            }
        )*
        /// Every registered code, for documentation and `lunar explain`.
        pub const ALL_CODES: &[Code] = &[ $( $( $scope::$konst, )* )* ];
    };
}

codes! {
    // `type` is a keyword in Rust, so that scope lives in the `ty` module but is
    // rendered as `type`.
    ty ("type") => {
        MISMATCH = "mismatch",
        ARRAY_POINTY_ITEMS = "array_pointy_items",
        BANG_OUTSIDE_PARAM = "bang_outside_param",
        BANG_NOT_SHRINKABLE = "bang_not_shrinkable",
        BANG_WITHOUT_MUT = "bang_without_mut",
        FMTARGS_OUTSIDE_PARAM = "fmtargs_outside_param",
        ANNOTATIONS_NEEDED = "annotations_needed",
        NOT_CALLABLE = "not_callable",
        ARG_COUNT = "arg_count",
        GENERIC_COUNT = "generic_count",
        NO_FIELD = "no_field",
        NO_METHOD = "no_method",
        NOT_INDEXABLE = "not_indexable",
        BAD_OPERAND = "bad_operand",
        BAD_CAST = "bad_cast",
        UNSATISFIED_BOUND = "unsatisfied_bound",
        INFINITE_SIZE = "infinite_size",
        ARRAY_LEN = "array_len",
        NOT_A_PLACE = "not_a_place",
        IMMUTABLE = "immutable",
        MISSING_FIELD = "missing_field",
        DUPLICATE_FIELD = "duplicate_field",
        NON_EXHAUSTIVE = "non_exhaustive",
        BAD_PATTERN = "bad_pattern",
        RETURN_MISSING = "return_missing",
        BREAK_OUTSIDE_LOOP = "break_outside_loop",
        LITERAL_RANGE = "literal_range",
        NOT_ITERABLE = "not_iterable",
        FFI_TYPE = "ffi_type",
        BAD_MAIN = "bad_main",
        BAD_TEST = "bad_test",
        CONST_EVAL = "const_eval",
        VARIADIC_NOT_EXTERN = "variadic_not_extern",
    }
    syntax ("syntax") => {
        UNEXPECTED_TOKEN = "unexpected_token",
        UNKNOWN_CHAR = "unknown_char",
        UNTERMINATED = "unterminated",
        BAD_ESCAPE = "bad_escape",
        BAD_NUMBER = "bad_number",
        BAD_FORMAT = "bad_format",
        MISSING_SEMI = "missing_semi",
        MISSING_BODY = "missing_body",
        BAD_CHAR = "bad_char",
        BAD_ATTRIBUTE = "bad_attribute",
        CONFLICT_MARKER = "conflict_marker",
    }
    name ("name") => {
        UNRESOLVED = "unresolved",
        DUPLICATE = "duplicate",
        PRIVATE = "private",
        NOT_A_TYPE = "not_a_type",
        NOT_A_VALUE = "not_a_value",
        NOT_A_TRAIT = "not_a_trait",
        MODULE_FILE = "module_file",
        BAD_SELF = "bad_self",
    }
    trait_ ("trait") => {
        DULL_IMPL = "dull_impl",
        NEGATIVE_IMPL = "negative_impl",
        MISSING_ITEM = "missing_item",
        EXTRA_ITEM = "extra_item",
        SIGNATURE = "signature",
        CONFLICTING_IMPL = "conflicting_impl",
        AMBIGUOUS_METHOD = "ambiguous_method",
        INHERENT_FOREIGN = "inherent_foreign",
    }
    borrow ("borrow") => {
        USE_AFTER_MOVE = "use_after_move",
        MOVE_OUT_OF_BORROW = "move_out_of_borrow",
        SHRINK_NOT_OWNER = "shrink_not_owner",
        SHRINK_WHILE_BORROWED = "shrink_while_borrowed",
        MOVE_WHILE_BORROWED = "move_while_borrowed",
        ASSIGN_WHILE_BORROWED = "assign_while_borrowed",
        OUTLIVES_ROOT = "outlives_root",
        ESCAPES_FUNCTION = "escapes_function",
        ESCAPES_VIA_PARAM = "escapes_via_param",
    }
    unsafe_ ("unsafe") => {
        OUTSIDE_UNSAFE = "outside_unsafe",
        IN_UNSAFE_FN = "in_unsafe_fn",
    }
    lint ("lint") => {
        UNUSED_VARIABLE = "unused_variable",
        UNREACHABLE = "unreachable",
        UNUSED_MUT = "unused_mut",
        UNKNOWN_CFG = "unknown_cfg",
    }
    build ("build") => {
        LINK = "link",
        LLVM = "llvm",
        IO = "io",
        NO_MAIN = "no_main",
        MISSING_LANG_ITEM = "missing_lang_item",
    }
}
