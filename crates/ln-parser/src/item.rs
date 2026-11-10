//! Items: functions, structs, enums, traits, impls, uses, modules, ...

use crate::{NEST, P};
use ln_ast::*;
use ln_diag::{Diagnostic, codes::syntax};
use ln_lexer::{NumKind, Token as T};
use ln_parse_kit::{Expected, Fatal, PResult};

/// Tokens that can begin an item (used for recovery and statement dispatch).
pub(crate) const ITEM_START: &[T] =
    &[T::Fn, T::Struct, T::Enum, T::Trait, T::Impl, T::Use, T::Mod, T::Const, T::Static, T::Extern, T::Type, T::Pub, T::Hash, T::Macro];

impl P<'_, '_, '_> {
    pub(crate) fn items_until_eof(&mut self) -> Vec<Item> {
        let mut items = Vec::new();
        while !self.k.at_eof() {
            match self.item() {
                Ok(i) => items.push(i),
                Err(Fatal) => self.recover_item(),
            }
        }
        items
    }

    fn items_until_brace(&mut self) -> PResult<Vec<Item>> {
        self.k.expect(T::LBrace)?;
        let mut items = Vec::new();
        loop {
            if self.k.eat(T::RBrace) {
                return Ok(items);
            }
            if self.k.at_eof() {
                return Err(self.k.error_expected());
            }
            match self.item() {
                Ok(i) => items.push(i),
                Err(Fatal) => {
                    self.recover_item();
                }
            }
        }
    }

    /// Skip to the next plausible item start.
    fn recover_item(&mut self) {
        // Always make progress.
        if !self.k.at_eof() {
            let t = self.k.bump();
            if t.kind == T::LBrace {
                self.k.recover_until(&[T::RBrace], NEST);
                self.k.eat(T::RBrace);
            }
        }
        let mut stop: Vec<T> = ITEM_START.to_vec();
        stop.push(T::Unsafe);
        stop.push(T::RBrace);
        self.k.recover_until(&stop, NEST);
    }

    pub(crate) fn at_item_start(&mut self) -> bool {
        match self.k.peek() {
            Some(T::Unsafe) => self.k.nth_at(1, T::Fn),
            Some(t) => ITEM_START.contains(&t),
            None => false,
        }
    }

    /// Attributes, including doc comments (as `#[doc = "..."]`), before and
    /// between `#[...]`s.
    pub(crate) fn attrs(&mut self) -> PResult<Vec<Attr>> {
        let mut out = Vec::new();
        loop {
            out.extend(self.doc_attrs());
            if !self.k.at(T::Hash) {
                return Ok(out);
            }
            let start = self.start();
            self.k.bump();
            self.k.expect(T::LBracket)?;
            let meta = self.meta_full()?;
            self.k.expect(T::RBracket)?;
            out.push(Attr { meta, span: self.span_from(start) });
        }
    }

    /// The doc comments directly before the current token, as `doc`
    /// attributes: `/// text` is `#[doc = "text"]`, one per line.
    pub(crate) fn doc_attrs(&mut self) -> Vec<Attr> {
        let Some(t) = self.k.current() else { return Vec::new() };
        let Some(lines) = self.docs.remove(&t.start) else { return Vec::new() };
        lines
            .into_iter()
            .map(|(start, end)| {
                let raw = self.text_of(start, end);
                let text = raw.strip_prefix("///").unwrap_or(raw).trim_end_matches('\r');
                let text = text.strip_prefix(' ').unwrap_or(text);
                let span = ln_diag::Span::new(self.file, start, end);
                Attr { meta: Meta::NameValue(Ident::new("doc", span), text.to_string()), span }
            })
            .collect()
    }

    fn meta_name(&mut self) -> PResult<Ident> {
        // Attribute names may be keywords such as `unsafe`.
        let t = match self.k.current() {
            Some(t) if t.kind == T::Ident || self.tok_text(t).chars().all(|c| c.is_ascii_alphanumeric() || c == '_') => t,
            _ => return Err(self.k.error_expected_named("attribute name")),
        };
        self.k.bump();
        Ok(self.tok_ident(t))
    }

    pub(crate) fn item(&mut self) -> PResult<Item> {
        let start = self.start();
        let attrs = self.attrs()?;
        let is_pub = self.k.eat(T::Pub);
        let kind = match self.k.peek() {
            Some(T::Fn) => ItemKind::Fn(self.fn_decl(false)?),
            Some(T::Unsafe) if self.k.nth_at(1, T::Fn) => {
                self.k.bump();
                ItemKind::Fn(self.fn_decl(true)?)
            }
            Some(T::Struct) => ItemKind::Struct(self.struct_decl()?),
            Some(T::Enum) => ItemKind::Enum(self.enum_decl()?),
            Some(T::Trait) => ItemKind::Trait(self.trait_decl()?),
            Some(T::Impl) => ItemKind::Impl(self.impl_decl()?),
            Some(T::Use) => {
                self.k.bump();
                let tree = self.use_tree()?;
                self.expect_item_semi("`use` declaration")?;
                ItemKind::Use(tree)
            }
            Some(T::Mod) => {
                self.k.bump();
                let name = self.ident()?;
                let items = if self.k.at(T::LBrace) {
                    Some(self.items_until_brace()?)
                } else {
                    self.expect_item_semi("module declaration")?;
                    None
                };
                ItemKind::Mod(ModDecl { name, items })
            }
            Some(T::Const) => {
                self.k.bump();
                let name = self.ident()?;
                self.k.expect(T::Colon)?;
                let ty = self.ty()?;
                self.k.expect(T::Eq)?;
                let value = self.expr()?;
                self.expect_item_semi("constant")?;
                ItemKind::Const(ConstDecl { name, ty, value })
            }
            Some(T::Static) => {
                self.k.bump();
                let is_mut = self.k.eat(T::Mut);
                let name = self.ident()?;
                self.k.expect(T::Colon)?;
                let ty = self.ty()?;
                self.k.expect(T::Eq)?;
                let value = self.expr()?;
                self.expect_item_semi("static")?;
                ItemKind::Static(StaticDecl { name, is_mut, ty, value })
            }
            Some(T::Extern) => {
                self.k.bump();
                let abi = if self.k.at(T::Str) {
                    let t = self.k.bump();
                    let raw = self.tok_text(t);
                    Some(raw[1..raw.len() - 1].to_string())
                } else {
                    None
                };
                let items = self.items_until_brace()?;
                ItemKind::Extern(ExternBlock { abi, items })
            }
            Some(T::Type) => {
                self.k.bump();
                let name = self.ident()?;
                let generics = self.generics_decl()?;
                self.k.expect(T::Eq)?;
                let ty = self.ty()?;
                self.expect_item_semi("type alias")?;
                ItemKind::TypeAlias(TypeAlias { name, generics, ty })
            }
            Some(T::Macro) => ItemKind::Macro(self.macro_def()?),
            // `path!(...)` in item position.
            Some(T::Ident | T::Crater | T::SelfValue | T::Super) => {
                let path = self.path_ty_style()?;
                if !self.at_mac_bang() {
                    self.k.expect_note(Expected::Named("item"));
                    return Err(self.k.error_expected());
                }
                let call = self.mac_call(path)?;
                if call.delim != Delim::Brace {
                    self.expect_item_semi("macro invocation")?;
                }
                ItemKind::MacCall(call)
            }
            _ => {
                self.k.expect_note(Expected::Named("item"));
                return Err(self.k.error_expected());
            }
        };
        Ok(Item { attrs, is_pub, kind, span: self.span_from(start) })
    }

    /// `::<T: A + B, U>` after a declaration name (or after `impl`).
    pub(crate) fn generics_decl(&mut self) -> PResult<Generics> {
        let start = self.start();
        if !(self.k.at(T::PathSep) && self.k.nth_at(1, T::LAngle)) {
            return Ok(Generics { params: Vec::new(), span: self.span(start, start) });
        }
        self.k.bump();
        self.k.bump();
        let mut params = Vec::new();
        loop {
            if self.k.eat(T::RAngle) {
                break;
            }
            let name = self.ident()?;
            let mut bounds = Vec::new();
            if self.k.eat(T::Colon) {
                bounds = self.bounds()?;
            }
            params.push(GenericParam { name, bounds });
            if !self.k.eat(T::Comma) {
                self.k.expect(T::RAngle)?;
                break;
            }
        }
        Ok(Generics { params, span: self.span_from(start) })
    }

    /// `A + B::<C> + D`
    pub(crate) fn bounds(&mut self) -> PResult<Vec<Path>> {
        let mut out = vec![self.bound()?];
        while self.k.eat(T::Plus) {
            out.push(self.bound()?);
        }
        Ok(out)
    }

    /// A trait in a bound. `Fn(A, &B): R` (and `FnMut`, `FnOnce`, or any
    /// path followed by a parameter list) is the trait with the signature
    /// `fn(A, &B): R` as its argument.
    pub(crate) fn bound(&mut self) -> PResult<Path> {
        let mut path = self.path_ty_style()?;
        if self.k.at(T::LParen) {
            let start = self.start();
            let kind = self.fn_sig_tail()?;
            let sig = Ty { kind, span: self.span_from(start), id: self.id() };
            path.span = path.span.to(sig.span);
            path.segments.last_mut().unwrap().args = Some(vec![sig]);
        }
        Ok(path)
    }

    fn fn_decl(&mut self, is_unsafe: bool) -> PResult<FnDecl> {
        let sig_start = self.start();
        self.k.expect(T::Fn)?;
        let name = self.ident()?;
        let generics = self.generics_decl()?;
        self.k.expect(T::LParen)?;
        let mut params = Vec::new();
        let mut variadic = false;
        loop {
            if self.k.eat(T::RParen) {
                break;
            }
            if self.k.eat(T::DotDotDot) {
                variadic = true;
                self.k.expect(T::RParen)?;
                break;
            }
            match self.param() {
                Ok(p) => params.push(p),
                Err(Fatal) => self.k.recover_until(&[T::Comma, T::RParen], NEST),
            }
            if !self.k.eat(T::Comma) {
                self.k.expect(T::RParen)?;
                break;
            }
        }
        let ret = if self.k.eat(T::Colon) { Some(self.ret_ty()?) } else { None };
        // `: T from a, b`, or `: T from static` (into no parameter). `from`
        // is a word only here, right after a return type and before a name,
        // so `From::from` and `fn from` still work.
        let mut ret_from = Vec::new();
        let from_word = matches!(self.k.nth_tok(0), Some(t) if t.kind == T::Ident && self.tok_text(t) == "from")
            && matches!(self.k.nth(1), Some(T::Ident | T::SelfValue | T::Static));
        if ret.is_some() && from_word {
            self.k.bump();
            if self.k.at(T::Static) {
                let t = self.k.expect(T::Static)?;
                ret_from.push(self.tok_ident(t));
            } else {
                loop {
                    let t = if self.k.at(T::SelfValue) { self.k.expect(T::SelfValue)? } else { self.k.expect(T::Ident)? };
                    ret_from.push(self.tok_ident(t));
                    if !self.k.eat(T::Comma) {
                        break;
                    }
                }
            }
        }
        let sig_span = self.span_from(sig_start);
        let body = if self.k.eat(T::Semi) { None } else { Some(self.block()?) };
        Ok(FnDecl { name, generics, params, ret, ret_from, body, is_unsafe, variadic, sig_span })
    }

    fn param(&mut self) -> PResult<Param> {
        let start = self.start();
        // Receivers: `&self`, `&mut self`, `&mut +self`.
        let ref_self = self.k.at(T::Amp)
            && match self.k.nth(1) {
                Some(T::SelfValue) => true,
                Some(T::Mut) => matches!(self.k.nth(2), Some(T::SelfValue)) || (self.k.nth(2) == Some(T::Plus) && self.k.nth(3) == Some(T::SelfValue)),
                _ => false,
            };
        if ref_self {
            self.k.bump();
            let mode = if self.k.eat(T::Mut) { ParamMode::InOut } else { ParamMode::Borrow };
            let self_grow = self.k.eat(T::Plus);
            self.k.expect(T::SelfValue)?;
            return Ok(Param { is_mut: false, mode, kind: ParamKind::SelfParam, ty: None, self_grow, kept: false, span: self.span_from(start) });
        }
        // `kept x: T`. `kept` is a word only here, so a parameter may still
        // be named `kept`.
        let kept = matches!(self.k.nth_tok(0), Some(t) if t.kind == T::Ident && self.tok_text(t) == "kept")
            && matches!(self.k.nth(1), Some(T::Ident | T::Mut | T::Placeholder));
        if kept {
            self.k.bump();
        }
        let is_mut = self.k.eat(T::Mut);
        if !kept && self.k.eat(T::SelfValue) {
            let ty = if self.k.eat(T::Colon) { Some(self.ty()?) } else { None };
            return Ok(Param { is_mut, mode: ParamMode::Take, kind: ParamKind::SelfParam, ty, self_grow: false, kept: false, span: self.span_from(start) });
        }
        let kind = if self.k.eat(T::Placeholder) { ParamKind::Wild } else { ParamKind::Named(self.ident()?) };
        self.k.expect(T::Colon)?;
        // `x: &T` borrows, `x: &mut T` is in-out, `x: T` takes ownership.
        let mode = if self.k.eat(T::Amp) { if self.k.eat(T::Mut) { ParamMode::InOut } else { ParamMode::Borrow } } else { ParamMode::Take };
        let ty = self.ty()?;
        Ok(Param { is_mut, mode, kind, ty: Some(ty), self_grow: false, kept, span: self.span_from(start) })
    }

    fn struct_decl(&mut self) -> PResult<StructDecl> {
        self.k.expect(T::Struct)?;
        let name = self.ident()?;
        let generics = self.generics_decl()?;
        let kind = if self.k.at(T::LParen) {
            let d = self.tuple_fields()?;
            self.expect_item_semi("tuple struct")?;
            d
        } else if self.k.at(T::LBrace) {
            self.named_fields()?
        } else {
            self.expect_item_semi("unit struct")?;
            VariantData::Unit
        };
        Ok(StructDecl { name, generics, kind })
    }

    fn tuple_fields(&mut self) -> PResult<VariantData> {
        self.k.expect(T::LParen)?;
        let mut out = Vec::new();
        loop {
            if self.k.eat(T::RParen) {
                break;
            }
            let is_pub = self.k.eat(T::Pub);
            out.push(TupleField { is_pub, ty: self.ty()? });
            if !self.k.eat(T::Comma) {
                self.k.expect(T::RParen)?;
                break;
            }
        }
        Ok(VariantData::Tuple(out))
    }

    fn named_fields(&mut self) -> PResult<VariantData> {
        self.k.expect(T::LBrace)?;
        let mut out = Vec::new();
        loop {
            if self.k.eat(T::RBrace) {
                break;
            }
            let docs = self.doc_attrs();
            let start = self.start();
            let is_pub = self.k.eat(T::Pub);
            let name = self.ident()?;
            self.k.expect(T::Colon)?;
            let ty = self.ty()?;
            out.push(FieldDecl { attrs: docs, is_pub, name, ty, span: self.span_from(start) });
            if !self.k.eat(T::Comma) {
                self.k.expect(T::RBrace)?;
                break;
            }
        }
        Ok(VariantData::Named(out))
    }

    fn enum_decl(&mut self) -> PResult<EnumDecl> {
        self.k.expect(T::Enum)?;
        let name = self.ident()?;
        let generics = self.generics_decl()?;
        self.k.expect(T::LBrace)?;
        let mut variants = Vec::new();
        loop {
            if self.k.eat(T::RBrace) {
                break;
            }
            // Doc comments, and attributes for derives such as `#[default]`.
            let attrs = self.attrs()?;
            let start = self.start();
            let vname = self.ident()?;
            let data = if self.k.at(T::LParen) {
                self.tuple_fields()?
            } else if self.k.at(T::LBrace) {
                self.named_fields()?
            } else {
                VariantData::Unit
            };
            variants.push(VariantDecl { attrs, name: vname, data, span: self.span_from(start) });
            if !self.k.eat(T::Comma) {
                self.k.expect(T::RBrace)?;
                break;
            }
        }
        Ok(EnumDecl { name, generics, variants })
    }

    fn trait_decl(&mut self) -> PResult<TraitDecl> {
        self.k.expect(T::Trait)?;
        let name = self.ident()?;
        let generics = self.generics_decl()?;
        let supertraits = if self.k.eat(T::Colon) { self.bounds()? } else { Vec::new() };
        let items = self.items_until_brace()?;
        Ok(TraitDecl { name, generics, supertraits, items })
    }

    fn impl_decl(&mut self) -> PResult<ImplDecl> {
        self.k.expect(T::Impl)?;
        let generics = self.generics_decl()?;
        let negative = self.k.eat(T::Bang);
        let first = self.ty()?;
        let (trait_ref, self_ty) = if self.k.eat(T::For) {
            let TyKind::Path(path) = first.kind else {
                return Err(self.k.error_at(first.span.start, first.span.end, "expected a trait path before `for`"));
            };
            (Some(path), self.ty()?)
        } else {
            if negative {
                return Err(self.k.error_at(first.span.start, first.span.end, "negative impls must name a trait: `impl !Trait for Type`"));
            }
            (None, first)
        };
        let items = self.items_until_brace()?;
        Ok(ImplDecl { generics, negative, trait_ref, self_ty, items })
    }

    fn use_tree(&mut self) -> PResult<UseTree> {
        let start = self.start();
        let mut prefix = Vec::new();
        loop {
            if self.k.at(T::Star) {
                self.k.bump();
                return Ok(UseTree { prefix, kind: UseKind::Glob, span: self.span_from(start) });
            }
            if self.k.at(T::LBrace) {
                self.k.bump();
                let mut nested = Vec::new();
                loop {
                    if self.k.eat(T::RBrace) {
                        break;
                    }
                    nested.push(self.use_tree()?);
                    if !self.k.eat(T::Comma) {
                        self.k.expect(T::RBrace)?;
                        break;
                    }
                }
                return Ok(UseTree { prefix, kind: UseKind::Nested(nested), span: self.span_from(start) });
            }
            prefix.push(self.path_segment_ident()?);
            if !self.k.eat(T::PathSep) {
                break;
            }
        }
        let rename = if self.k.eat(T::As) { Some(self.ident()?) } else { None };
        Ok(UseTree { prefix, kind: UseKind::Simple(rename), span: self.span_from(start) })
    }

    /// An identifier or one of the path keywords (`crate`, `self`, `super`, `Self`).
    pub(crate) fn path_segment_ident(&mut self) -> PResult<Ident> {
        match self.k.peek() {
            Some(t) if t == T::Ident || t.is_path_keyword() => {
                let tok = self.k.bump();
                Ok(self.tok_ident(tok))
            }
            _ => {
                self.k.expect_note(Expected::Token(T::Ident));
                Err(self.k.error_expected())
            }
        }
    }
}

// Attribute contents: `name`, `name = value`, `name(meta, ...)`.
impl P<'_, '_, '_> {
    pub(crate) fn meta_list(&mut self) -> PResult<Vec<Meta>> {
        self.k.expect(T::LParen)?;
        let mut out = Vec::new();
        loop {
            if self.k.eat(T::RParen) {
                return Ok(out);
            }
            out.push(self.meta_full()?);
            if !self.k.eat(T::Comma) {
                self.k.expect(T::RParen)?;
                return Ok(out);
            }
        }
    }

    pub(crate) fn meta_full(&mut self) -> PResult<Meta> {
        let name = self.meta_name()?;
        if self.k.eat(T::Eq) {
            let t = match self.k.peek() {
                Some(T::Str) | Some(T::Number(NumKind::Int)) | Some(T::Ident) => self.k.bump(),
                _ => return Err(self.k.error_expected_named("attribute value")),
            };
            let raw = self.tok_text(t);
            let v = if t.kind == T::Str { crate::lit::unescape(&raw[1..raw.len() - 1]).unwrap_or_default() } else { raw.to_string() };
            return Ok(Meta::NameValue(name, v));
        }
        if self.k.at(T::LParen) {
            let list = self.meta_list()?;
            return Ok(Meta::List(name, list));
        }
        Ok(Meta::Word(name))
    }

    /// The `;` that ends an item such as `use`, `const` or `struct S;`.
    /// When the next token starts another item (or closes the enclosing
    /// block, or ends the file), the `;` was simply forgotten:
    /// `syntax::missing_semi`, and parsing goes on. Otherwise the item is
    /// followed by something unexpected, reported as such.
    fn expect_item_semi(&mut self, what: &str) -> PResult<()> {
        if self.k.eat(T::Semi) {
            return Ok(());
        }
        if self.at_item_start() || self.k.at(T::RBrace) || self.k.at_eof() {
            let at = self.span(self.k.prev_end(), self.k.prev_end());
            self.error(Diagnostic::error(syntax::MISSING_SEMI, at, format!("expected `;` after {what}")).with_label("add `;` here"));
            return Ok(());
        }
        self.k.expect(T::Semi)?;
        Ok(())
    }
}
