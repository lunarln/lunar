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
    /// `$`: a macro metavariable (`$x:expr`) or repetition (`$(...)*`).
    #[token("$")]
    Dollar,
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
    #[token("crater")]
    Crater,
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
    #[token("macro")]
    Macro,
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
    /// `b'a'`: a `u8`.
    #[regex(r#"b'([^'\\\n]|\\[^\n][^'\n]*)'"#)]
    Byte,
    /// `/// text`: documentation for the item that follows. Exactly three
    /// slashes; `////` is an ordinary comment. The parser attaches it to the
    /// next item as `#[doc = "text"]`.
    #[regex(r"///([^/\n][^\n]*)?", priority = 10, allow_greedy = true)]
    DocComment,
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

/// Nested `/* ... */` comments. An unterminated comment is an error
/// covering the rest of the input.
fn lex_block_comment(lex: &mut Lexer<Token>) -> logos::FilterResult<(), ()> {
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
    if depth > 0 { logos::FilterResult::Error(()) } else { logos::FilterResult::Skip }
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
    pub kind: LexErrorKind,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LexErrorKind {
    /// Characters that start no token (or a malformed literal).
    Invalid,
    /// A `/*` comment without its closing `*/`; the range runs to the end.
    UnterminatedComment,
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
            Err(()) if src[range.clone()].starts_with("/*") => errs.push(LexError { range, kind: LexErrorKind::UnterminatedComment }),
            Err(()) => match errs.last_mut() {
                // Merge runs of adjacent bad characters into one error.
                Some(last) if last.range.end == range.start && last.kind == LexErrorKind::Invalid => last.range.end = range.end,
                _ => errs.push(LexError { range, kind: LexErrorKind::Invalid }),
            },
        }
    }
    (toks, errs)
}

/// A version control conflict region left in a file by an unfinished merge
/// or rebase. Each field is the byte range of a marker line (without its
/// newline).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Conflict {
    /// `<<<<<<< ours`
    pub start: Range<usize>,
    /// `||||||| base` (diff3 style), if present.
    pub base: Option<Range<usize>>,
    /// `=======`
    pub mid: Option<Range<usize>>,
    /// `>>>>>>> theirs`
    pub end: Option<Range<usize>>,
}

/// Which conflict marker, if any, the line starting at `line` is.
fn marker_at(line: &str) -> Option<u8> {
    let b = line.as_bytes();
    let c = *b.first()?;
    if !matches!(c, b'<' | b'|' | b'=' | b'>') || b.len() < 7 || !b[..7].iter().all(|x| *x == c) {
        return None;
    }
    // Exactly seven marker characters, then the end of the line or a space
    // (`<<<<<<< HEAD`). `=======` stands alone.
    match b.get(7) {
        None | Some(b'\r') => Some(c),
        Some(b' ') | Some(b'\t') if c != b'=' => Some(c),
        _ => None,
    }
}

/// Find conflict markers (`<<<<<<<`, `|||||||`, `=======`, `>>>>>>>` at
/// the start of a line). Checked before lexing so that a half-merged file
/// gets one clear error instead of a cascade of syntax errors.
pub fn conflict_markers(src: &str) -> Vec<Conflict> {
    let mut out: Vec<Conflict> = Vec::new();
    let mut open: Option<Conflict> = None;
    let mut pos = 0;
    for line in src.split_inclusive('\n') {
        let text = line.trim_end_matches(['\n', '\r']);
        let range = pos..pos + text.len();
        pos += line.len();
        let Some(m) = marker_at(text) else { continue };
        match (m, &mut open) {
            (b'<', _) => {
                if let Some(c) = open.take() {
                    out.push(c);
                }
                open = Some(Conflict { start: range, base: None, mid: None, end: None });
            }
            (b'|', Some(c)) if c.mid.is_none() => c.base = Some(range),
            (b'=', Some(c)) if c.mid.is_none() => c.mid = Some(range),
            (b'>', Some(c)) => {
                c.end = Some(range);
                out.push(open.take().unwrap());
            }
            // A stray marker outside a `<<<<<<<` region: report it alone.
            (_, None) => out.push(Conflict { start: range, base: None, mid: None, end: None }),
            _ => {}
        }
    }
    out.extend(open);
    out
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
            Dollar => "`$`",
            Placeholder => "`_`",
            As => "`as`",
            Break => "`break`",
            Const => "`const`",
            Continue => "`continue`",
            Crater => "`crater`",
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
            Macro => "`macro`",
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
            Byte => "byte literal",
            DocComment => "doc comment",
            BlockComment => "comment",
        }
    }

    /// Whether this token is a keyword that may also appear as a path segment.
    pub fn is_path_keyword(self) -> bool {
        matches!(self, Token::Crater | Token::SelfValue | Token::SelfType | Token::Super)
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
        assert_eq!(kinds("b'a' b'\\xff' b 'c' b2'd'"), vec![Byte, Byte, Ident, Char, Ident, Char]);
    }

    #[test]
    fn doc_comments() {
        use Token::*;
        assert_eq!(kinds("/// docs\nfn"), vec![DocComment, Fn]);
        assert_eq!(kinds("///\nx"), vec![DocComment, Ident]);
        assert_eq!(kinds("//// not docs\n// nor this\nx"), vec![Ident]);
        let (toks, _) = lex("/// a b\n");
        assert_eq!(toks[0].range, 0..7);
    }

    #[test]
    fn generics_do_not_fuse_closers() {
        use Token::*;
        assert_eq!(kinds("A::<B::<C>>"), vec![Ident, PathSep, LAngle, Ident, PathSep, LAngle, Ident, RAngle, RAngle]);
    }

    #[test]
    fn macros() {
        use Token::*;
        assert_eq!(
            kinds("macro m($x:expr) { $($x),* }"),
            vec![Macro, Ident, LParen, Dollar, Ident, Colon, Ident, RParen, LBrace, Dollar, LParen, Dollar, Ident, RParen, Comma, Star, RBrace]
        );
        assert_eq!(kinds("vec![1] macros"), vec![Ident, Bang, LBracket, Number(NumKind::Int), RBracket, Ident]);
    }

    #[test]
    fn unterminated_block_comment() {
        let (toks, errs) = lex("a /* x /* y */ b");
        assert_eq!(toks.len(), 1);
        assert_eq!(errs, vec![LexError { range: 2..16, kind: LexErrorKind::UnterminatedComment }]);
        let (_, errs) = lex("a /* x /* y */ b */ c");
        assert!(errs.is_empty());
    }

    #[test]
    fn conflict_markers_found_only_at_line_start() {
        let src = "fn f() {\n<<<<<<< HEAD\n    1\n=======\n    2\n>>>>>>> topic\n}\n";
        let c = conflict_markers(src);
        assert_eq!(c.len(), 1);
        assert_eq!(&src[c[0].start.clone()], "<<<<<<< HEAD");
        assert_eq!(&src[c[0].mid.clone().unwrap()], "=======");
        assert_eq!(&src[c[0].end.clone().unwrap()], ">>>>>>> topic");
        // CRLF line endings.
        assert_eq!(conflict_markers(&src.replace('\n', "\r\n")).len(), 1);
        // Not markers: indented, too many characters, inside an expression.
        assert!(conflict_markers("  <<<<<<< x\n========\na <<<<<<< b\n").is_empty());
        // A stray marker on its own is still reported.
        assert_eq!(conflict_markers("x\n>>>>>>> other\n").len(), 1);
    }
}
