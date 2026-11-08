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

// `type` is a keyword in Rust, so that scope lives in the `ty` module but is
// rendered as `type`.
pub mod ty {
    use super::Code;
    macro_rules! c { ($k:ident = $n:literal) => { pub const $k: Code = Code::new("type", $n); }; }
    c!(MISMATCH = "mismatch");
    c!(ARRAY_POINTY_ITEMS = "array_pointy_items");
    c!(BANG_OUTSIDE_PARAM = "bang_outside_param");
    c!(BANG_NOT_SHRINKABLE = "bang_not_shrinkable");
    c!(FMTARGS_OUTSIDE_PARAM = "fmtargs_outside_param");
    c!(ANNOTATIONS_NEEDED = "annotations_needed");
    c!(NOT_CALLABLE = "not_callable");
    c!(ARG_COUNT = "arg_count");
    c!(GENERIC_COUNT = "generic_count");
    c!(NO_FIELD = "no_field");
    c!(NO_METHOD = "no_method");
    c!(NOT_INDEXABLE = "not_indexable");
    c!(BAD_OPERAND = "bad_operand");
    c!(BAD_CAST = "bad_cast");
    c!(UNSATISFIED_BOUND = "unsatisfied_bound");
    c!(INFINITE_SIZE = "infinite_size");
    c!(ARRAY_LEN = "array_len");
    c!(NOT_A_PLACE = "not_a_place");
    c!(IMMUTABLE = "immutable");
    c!(MISSING_FIELD = "missing_field");
    c!(DUPLICATE_FIELD = "duplicate_field");
    c!(NON_EXHAUSTIVE = "non_exhaustive");
    c!(BAD_PATTERN = "bad_pattern");
    c!(RETURN_MISSING = "return_missing");
    c!(BREAK_OUTSIDE_LOOP = "break_outside_loop");
    c!(LITERAL_RANGE = "literal_range");
    c!(NOT_ITERABLE = "not_iterable");
    c!(FFI_TYPE = "ffi_type");
    c!(BAD_MAIN = "bad_main");
    c!(BAD_TEST = "bad_test");
    c!(CONST_EVAL = "const_eval");
    c!(INTERPOLATION = "interpolation");
    c!(RECURSIVE_OWNER = "recursive_owner");
}

codes! {
    syntax ("syntax") => {
        UNEXPECTED_TOKEN = "unexpected_token",
        UNKNOWN_CHAR = "unknown_char",
        UNTERMINATED = "unterminated",
        BAD_ESCAPE = "bad_escape",
        BAD_NUMBER = "bad_number",
        BAD_FORMAT = "bad_format",
        MISSING_SEMI = "missing_semi",
        BAD_ATTRIBUTE = "bad_attribute",
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
        MUT_SLICE_OF_IMMUTABLE = "mut_slice_of_immutable",
    }
    unsafe_ ("unsafe") => {
        OUTSIDE_UNSAFE = "outside_unsafe",
        IN_UNSAFE_FN = "in_unsafe_fn",
    }
    lint ("lint") => {
        UNUSED_VARIABLE = "unused_variable",
        UNREACHABLE = "unreachable",
        UNUSED_MUT = "unused_mut",
    }
    build ("build") => {
        LINK = "link",
        LLVM = "llvm",
        IO = "io",
        NO_MAIN = "no_main",
    }
}
