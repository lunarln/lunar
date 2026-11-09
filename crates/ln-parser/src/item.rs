//! Items: functions, structs, enums, traits, impls, uses, modules, ...

use crate::{NEST, P};
use ln_ast::*;
use ln_diag::{Diagnostic, codes::syntax};
use ln_lexer::{NumKind, Token as T};
use ln_parse_kit::{Expected, Fatal, PResult};

/// Tokens that can begin an item (used for recovery and statement dispatch).
pub(crate) const ITEM_START: &[T] = &[T::Fn, T::Struct, T::Enum, T::Trait, T::Impl, T::Use, T::Mod, T::Const, T::Static, T::Extern, T::Type, T::Pub, T::Hash];

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

    pub(crate) fn attrs(&mut self) -> PResult<Vec<Attr>> {
        let mut out = Vec::new();
        while self.k.at(T::Hash) {
            let start = self.start();
            self.k.bump();
            self.k.expect(T::LBracket)?;
            let meta = self.meta_full()?;
            self.k.expect(T::RBracket)?;
            out.push(Attr { meta, span: self.span_from(start) });
        }
        Ok(out)
    }

    fn meta_name(&mut self) -> PResult<Ident> {
        // Attribute names may be keywords such as `unsafe`.
        let t = match self.k.current() {
            Some(t) if t.kind == T::Ident || self.tok_text(t).chars().all(|c| c.is_ascii_alphanumeric() || c == '_') => t,
            _ => return Err(self.k.error_expected_named("attribute name")),
        };
        self.k.bump();
        Ok(Ident::new(self.tok_text(t), self.tok_span(t)))
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
        let mut out = vec![self.path_ty_style()?];
        while self.k.eat(T::Plus) {
            out.push(self.path_ty_style()?);
        }
        Ok(out)
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
        let sig_span = self.span_from(sig_start);
        let body = if self.k.eat(T::Semi) { None } else { Some(self.block()?) };
        Ok(FnDecl { name, generics, params, ret, body, is_unsafe, variadic, sig_span })
    }

    fn param(&mut self) -> PResult<Param> {
        let start = self.start();
        let is_mut = self.k.eat(T::Mut);
        if self.k.eat(T::SelfValue) {
            let ty = if self.k.eat(T::Colon) { Some(self.ty()?) } else { None };
            return Ok(Param { is_mut, kind: ParamKind::SelfParam, ty, span: self.span_from(start) });
        }
        let kind = if self.k.eat(T::Placeholder) { ParamKind::Wild } else { ParamKind::Named(self.ident()?) };
        self.k.expect(T::Colon)?;
        let ty = self.ty()?;
        Ok(Param { is_mut, kind, ty: Some(ty), span: self.span_from(start) })
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
            let start = self.start();
            let is_pub = self.k.eat(T::Pub);
            let name = self.ident()?;
            self.k.expect(T::Colon)?;
            let ty = self.ty()?;
            out.push(FieldDecl { is_pub, name, ty, span: self.span_from(start) });
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
            let start = self.start();
            let vname = self.ident()?;
            let data = if self.k.at(T::LParen) {
                self.tuple_fields()?
            } else if self.k.at(T::LBrace) {
                self.named_fields()?
            } else {
                VariantData::Unit
            };
            variants.push(VariantDecl { name: vname, data, span: self.span_from(start) });
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
                Ok(Ident::new(self.tok_text(tok), self.tok_span(tok)))
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
