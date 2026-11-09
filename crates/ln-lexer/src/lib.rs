//! Lexer for the Lunar programming language.
//!
//! Tokens carry no payload; consumers slice the source text with the token's
//! byte range. `>` is never fused with a following `>`/`=` here (so generic
//! argument lists like `A::<B::<C>>` lex cleanly); the parser re-joins
//! adjacent `>` tokens into `>>`, `>=` and `>>=` where an operator is expected.

use logos::{Lexer, Logos};
use std::ops::Range;

/// Classification of a numeric literal.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum NumKind {
    Int,
    Float,
}

#[derive(Logos, Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[logos(skip r"[ \t\r\n\f]+")]
#[logos(skip(r"//[^\n]*", allow_greedy = true))]
pub enum Token {
    // ----- delimiters -----
    #[token("(")]
    LParen,
    #[token(")")]
    RParen,
    #[token("[")]
    LBracket,
    #[token("]")]
    RBracket,
    #[token("{")]
    LBrace,
    #[token("}")]
    RBrace,

    // ----- punctuation -----
    #[token("<")]
    LAngle,
    #[token(">")]
    RAngle,
    #[token("<=")]
    Le,
    #[token("<<")]
    Shl,
    #[token("<<=")]
    ShlEq,
    #[token("::")]
    PathSep,
    #[token(":")]
    Colon,
    #[token(";")]
    Semi,
    #[token(",")]
    Comma,
    #[token(".")]
    Dot,
    #[token("..")]
    DotDot,
    #[token("..=")]
    DotDotEq,
    #[token("...")]
    DotDotDot,
    #[token("==")]
    EqEq,
    #[token("!=")]
    BangEq,
    #[token("=")]
    Eq,
    #[token("=>")]
    FatArrow,
    #[token("->")]
    Arrow,
    #[token("!")]
    Bang,
    #[token("&&")]
    AmpAmp,
    #[token("&")]
    Amp,
    #[token("&=")]
    AmpEq,
    #[token("||")]
    PipePipe,
    #[token("|")]
    Pipe,
    #[token("|=")]
    PipeEq,
    #[token("^")]
    Caret,
    #[token("^=")]
    CaretEq,
    #[token("+")]
    Plus,
    #[token("+=")]
    PlusEq,
    #[token("-")]
    Minus,
    #[token("-=")]
    MinusEq,
    #[token("*")]
    Star,
    #[token("*=")]
    StarEq,
    #[token("/")]
    Slash,
    #[token("/=")]
    SlashEq,
    #[token("%")]
    Percent,
    #[token("%=")]
    PercentEq,
    #[token("~")]
    Tilde,
    #[token("?")]
    Question,
    #[token("#")]
    Hash,
    #[token("@")]
    At,
    #[token("_", priority = 20)]
    Placeholder,

    // ----- keywords -----
    #[token("as")]
    As,
    #[token("break")]
    Break,
    #[token("const")]
    Const,
    #[token("continue")]
    Continue,
    #[token("crate")]
    Crate,
    #[token("else")]
    Else,
    #[token("enum")]
    Enum,
    #[token("extern")]
    Extern,
    #[token("false")]
    False,
    #[token("fn")]
    Fn,
    #[token("for")]
    For,
    #[token("if")]
    If,
    #[token("impl")]
    Impl,
    #[token("in")]
    In,
    #[token("let")]
    Let,
    #[token("loop")]
    Loop,
    #[token("match")]
    Match,
    #[token("mod")]
    Mod,
    #[token("mut")]
    Mut,
    #[token("pub")]
    Pub,
    #[token("return")]
    Return,
    #[token("self")]
    SelfValue,
    #[token("Self")]
    SelfType,
    #[token("static")]
    Static,
    #[token("struct")]
    Struct,
    #[token("super")]
    Super,
    #[token("trait")]
    Trait,
    #[token("true")]
    True,
    #[token("type")]
    Type,
    #[token("unsafe")]
    Unsafe,
    #[token("use")]
    Use,
    #[token("while")]
    While,

    // ----- literals & identifiers -----
    #[regex("[a-zA-Z_][a-zA-Z0-9_]*")]
    Ident,
    #[regex("[0-9]", lex_number)]
    Number(NumKind),
    #[regex(r#""([^"\\]|\\(.|\n))*""#)]
    Str,
    #[regex(r#"'([^'\\\n]|\\[^\n][^'\n]*)'"#)]
    Char,
    #[token("/*", lex_block_comment)]
    BlockComment,
}

/// Consume the rest of a number after its first digit. Handles `0x`/`0o`/`0b`
/// prefixes, fractions (but not `1..2` or `1.method()`), exponents and type
/// suffixes such as `10u8` / `1.5f32`.
fn lex_number(lex: &mut Lexer<Token>) -> NumKind {
    let first = lex.slice().as_bytes()[0];
    let rest = lex.remainder().as_bytes();
    let mut i = 0;
    let mut kind = NumKind::Int;
    let is_digit = |c: u8| c.is_ascii_digit() || c == b'_';
    let is_ident_start = |c: u8| c.is_ascii_alphabetic() || c == b'_';

    if first == b'0' && rest.len() >= 2 && matches!(rest[0], b'x' | b'o' | b'b') && rest[1].is_ascii_hexdigit() {
        i = 1;
        while i < rest.len() && (rest[i].is_ascii_hexdigit() || rest[i] == b'_') {
            i += 1;
        }
    } else {
        while i < rest.len() && is_digit(rest[i]) {
            i += 1;
        }
        // Fraction: `1.` / `1.5` but not `1..2` or `1.foo`.
        if i < rest.len() && rest[i] == b'.' {
            let next = rest.get(i + 1).copied();
            let continues = match next {
                Some(b'.') => false,
                Some(c) if is_ident_start(c) => false,
                _ => true,
            };
            if continues {
                kind = NumKind::Float;
                i += 1;
                while i < rest.len() && is_digit(rest[i]) {
                    i += 1;
                }
            }
        }
        // Exponent.
        if i < rest.len() && matches!(rest[i], b'e' | b'E') {
            let mut j = i + 1;
            if j < rest.len() && matches!(rest[j], b'+' | b'-') {
                j += 1;
            }
            if j < rest.len() && rest[j].is_ascii_digit() {
                kind = NumKind::Float;
                i = j;
                while i < rest.len() && is_digit(rest[i]) {
                    i += 1;
                }
            }
        }
    }
    // Suffix.
    if i < rest.len() && is_ident_start(rest[i]) {
        let s = i;
        while i < rest.len() && (rest[i].is_ascii_alphanumeric() || rest[i] == b'_') {
            i += 1;
        }
        if rest[s] == b'f' && !(first == b'0' && matches!(rest.first(), Some(b'x'))) {
            kind = NumKind::Float;
        }
    }
    lex.bump(i);
    kind
}

/// Nested `/* ... */` comments. Unterminated comments run to end of input.
fn lex_block_comment(lex: &mut Lexer<Token>) -> logos::Skip {
    let rest = lex.remainder().as_bytes();
    let mut depth = 1;
    let mut i = 0;
    while i < rest.len() && depth > 0 {
        if rest[i] == b'/' && rest.get(i + 1) == Some(&b'*') {
            depth += 1;
            i += 2;
        } else if rest[i] == b'*' && rest.get(i + 1) == Some(&b'/') {
            depth -= 1;
            i += 2;
        } else {
            i += 1;
        }
    }
    lex.bump(i);
    logos::Skip
}

/// A lexed token and its byte range in the source.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Spanned {
    pub token: Token,
    pub range: Range<usize>,
}

/// A character sequence the lexer could not understand.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LexError {
    pub range: Range<usize>,
}

/// Lex a whole source file.
pub fn lex(src: &str) -> (Vec<Spanned>, Vec<LexError>) {
    let mut toks = Vec::new();
    let mut errs: Vec<LexError> = Vec::new();
    let mut lexer = Token::lexer(src);
    while let Some(res) = lexer.next() {
        let range = lexer.span();
        match res {
            Ok(token) => toks.push(Spanned { token, range }),
            Err(()) => match errs.last_mut() {
                // Merge runs of adjacent bad characters into one error.
                Some(last) if last.range.end == range.start => last.range.end = range.end,
                _ => errs.push(LexError { range }),
            },
        }
    }
    (toks, errs)
}

impl Token {
    /// Human-readable description used in "expected X" diagnostics.
    pub fn describe(self) -> &'static str {
        use Token::*;
        match self {
            LParen => "`(`",
            RParen => "`)`",
            LBracket => "`[`",
            RBracket => "`]`",
            LBrace => "`{`",
            RBrace => "`}`",
            LAngle => "`<`",
            RAngle => "`>`",
            Le => "`<=`",
            Shl => "`<<`",
            ShlEq => "`<<=`",
            PathSep => "`::`",
            Colon => "`:`",
            Semi => "`;`",
            Comma => "`,`",
            Dot => "`.`",
            DotDot => "`..`",
            DotDotEq => "`..=`",
            DotDotDot => "`...`",
            EqEq => "`==`",
            BangEq => "`!=`",
            Eq => "`=`",
            FatArrow => "`=>`",
            Arrow => "`->`",
            Bang => "`!`",
            AmpAmp => "`&&`",
            Amp => "`&`",
            AmpEq => "`&=`",
            PipePipe => "`||`",
            Pipe => "`|`",
            PipeEq => "`|=`",
            Caret => "`^`",
            CaretEq => "`^=`",
            Plus => "`+`",
            PlusEq => "`+=`",
            Minus => "`-`",
            MinusEq => "`-=`",
            Star => "`*`",
            StarEq => "`*=`",
            Slash => "`/`",
            SlashEq => "`/=`",
            Percent => "`%`",
            PercentEq => "`%=`",
            Tilde => "`~`",
            Question => "`?`",
            Hash => "`#`",
            At => "`@`",
            Placeholder => "`_`",
            As => "`as`",
            Break => "`break`",
            Const => "`const`",
            Continue => "`continue`",
            Crate => "`crate`",
            Else => "`else`",
            Enum => "`enum`",
            Extern => "`extern`",
            False => "`false`",
            Fn => "`fn`",
            For => "`for`",
            If => "`if`",
            Impl => "`impl`",
            In => "`in`",
            Let => "`let`",
            Loop => "`loop`",
            Match => "`match`",
            Mod => "`mod`",
            Mut => "`mut`",
            Pub => "`pub`",
            Return => "`return`",
            SelfValue => "`self`",
            SelfType => "`Self`",
            Static => "`static`",
            Struct => "`struct`",
            Super => "`super`",
            Trait => "`trait`",
            True => "`true`",
            Type => "`type`",
            Unsafe => "`unsafe`",
            Use => "`use`",
            While => "`while`",
            Ident => "identifier",
            Number(NumKind::Int) => "integer literal",
            Number(NumKind::Float) => "float literal",
            Str => "string literal",
            Char => "character literal",
            BlockComment => "comment",
        }
    }

    /// Whether this token is a keyword that may also appear as a path segment.
    pub fn is_path_keyword(self) -> bool {
        matches!(self, Token::Crate | Token::SelfValue | Token::SelfType | Token::Super)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kinds(src: &str) -> Vec<Token> {
        let (t, e) = lex(src);
        assert!(e.is_empty(), "lex errors: {e:?}");
        t.into_iter().map(|s| s.token).collect()
    }

    #[test]
    fn numbers_and_ranges() {
        use Token::*;
        assert_eq!(kinds("0..200"), vec![Number(NumKind::Int), DotDot, Number(NumKind::Int)]);
        assert_eq!(kinds("0."), vec![Number(NumKind::Float)]);
        assert_eq!(kinds("[0.; 8]"), vec![LBracket, Number(NumKind::Float), Semi, Number(NumKind::Int), RBracket]);
        assert_eq!(
            kinds("1.5e3 2e-1 7u8 1f32 0xffu8"),
            vec![Number(NumKind::Float), Number(NumKind::Float), Number(NumKind::Int), Number(NumKind::Float), Number(NumKind::Int)]
        );
        assert_eq!(kinds("x.0"), vec![Ident, Dot, Number(NumKind::Int)]);
        assert_eq!(kinds("1.max"), vec![Number(NumKind::Int), Dot, Ident]);
    }

    #[test]
    fn array_types() {
        use Token::*;
        assert_eq!(kinds("[T; -..+]"), vec![LBracket, Ident, Semi, Minus, DotDot, Plus, RBracket]);
        assert_eq!(kinds("![T; &mut]"), vec![Bang, LBracket, Ident, Semi, Amp, Mut, RBracket]);
    }

    #[test]
    fn comments_and_strings() {
        use Token::*;
        assert_eq!(kinds("/* a /* b */ c */ x // y\n\"s\\\"{x}\" 'a' '\\n'"), vec![Ident, Str, Char, Char]);
    }

    #[test]
    fn generics_do_not_fuse_closers() {
        use Token::*;
        assert_eq!(kinds("A::<B::<C>>"), vec![Ident, PathSep, LAngle, Ident, PathSep, LAngle, Ident, RAngle, RAngle]);
    }
}
