//! Move and borrow checking on THIR.
//!
//! * **Moves**: a forward dataflow over "maybe moved" paths reports uses of
//!   moved values and moves out of borrowed parameters.
//! * **Views**: slices, `str` and other pointy non-owning values have *roots*:
//!   the locals whose storage they point into. Roots are computed
//!   flow-insensitively, then checked when a view is stored into a longer
//!   lived local, returned, or written through a parameter.
//! * **Invalidation**: lending an owner in-out (to a `&mut` parameter, or
//!   to a closure that changes what it captured), replacing it, or moving it
//!   *invalidates* its root: the callee may shrink, clear or replace it. A
//!   backward liveness analysis reports any view of that root that is still
//!   used afterwards. A `&mut +T` parameter promises to only grow its
//!   argument, so lending to one invalidates nothing; inside the function,
//!   such a parameter may itself only be grown.

use crate::Program;
use crate::thir::*;
use crate::ty::{ArrKind, Ty};
use ln_diag::{Diagnostic, Diagnostics, Span, codes::borrow, codes::lint};
use std::collections::{BTreeSet, HashMap, HashSet};

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
enum Root {
    /// The storage a local owns (an owner's elements, a fixed array).
    Local(LocalId),
    /// The local variable itself: a closure that borrows a capture points
    /// at it.
    Slot(LocalId),
    /// Whatever the argument of parameter `.0` points into: the caller's
    /// data, which outlives the call. Tracked so that storing it into an
    /// in-out argument can be checked against `kept`.
    Arg(LocalId),
    /// A statement-scoped temporary.
    Temp,
}

type Roots = BTreeSet<Root>;
type LiveSet = BTreeSet<LocalId>;

/// A place path: root local plus field projections.
type PathP = (LocalId, Vec<usize>);

/// Warn about unused variables and needless `mut`.
pub fn lint(p: &Program, body: &Body, diags: &mut Diagnostics) {
    let mut used = HashSet::new();
    let mut mutated = HashSet::new();
    fn root(e: &Expr) -> Option<LocalId> {
        match &e.kind {
            ExprKind::Local(l) => Some(*l),
            ExprKind::Field { base, .. } | ExprKind::Index { base, .. } => root(base),
            _ => None,
        }
    }
    let walk = |e: &Expr, used: &mut HashSet<LocalId>, mutated: &mut HashSet<LocalId>| lint_walk(p, e, used, mutated);
    fn lint_walk(p: &Program, e: &Expr, used: &mut HashSet<LocalId>, mutated: &mut HashSet<LocalId>) {
        let walk = |e: &Expr, used: &mut HashSet<LocalId>, mutated: &mut HashSet<LocalId>| lint_walk(p, e, used, mutated);
        match &e.kind {
            ExprKind::Local(l) => {
                used.insert(*l);
            }
            // Assigning to a variable is not a use of it (`x = 2;`,
            // `x += 1;`), as in Rust: only reads count. Writing into a part
            // of it (`a[i] = v`, `s.f = v`) does count, as in Rust.
            ExprKind::Assign { place, value } | ExprKind::AssignOp { place, value, .. } => {
                if let Some(l) = root(place) {
                    mutated.insert(l);
                }
                if !matches!(place.kind, ExprKind::Local(_)) {
                    walk(place, used, mutated);
                }
                walk(value, used, mutated);
                return;
            }
            ExprKind::Call { args, callee } => {
                for a in args {
                    if let Arg::InOut(p) = a
                        && let Some(l) = root(p)
                    {
                        mutated.insert(l);
                    }
                }
                if let Callee::Value { callee: f, kind: crate::ClosureKind::FnMut } = callee
                    && let Some(l) = root(f)
                {
                    mutated.insert(l);
                }
            }
            // A closure that changes a capture changes the variable (if it
            // borrowed it) or needs it `mut` to change its own copy.
            ExprKind::Closure { def, captures } => {
                let caps = p.closures.get(def).map(|c| c.captures.as_slice()).unwrap_or_default();
                for (a, cap) in captures.iter().zip(caps) {
                    let place = match a.expr() {
                        Expr { kind: ExprKind::Read { place, .. }, .. } => place,
                        x => x,
                    };
                    if cap.mode == crate::defs::ParamMode::InOut
                        && let Some(l) = root(place)
                    {
                        mutated.insert(l);
                    }
                }
            }
            ExprKind::Slice { base, mutable: true, .. } | ExprKind::Coerce { kind: Coercion::ToSlice { mutable: true }, expr: base } => {
                if let Some(l) = root(base) {
                    mutated.insert(l);
                }
            }
            _ => {}
        }
        visit_children(e, &mut |c| walk(c, used, mutated));
    }
    visit_block(&body.block, &mut |e| walk(e, &mut used, &mut mutated));
    // `mut x: T` is the function's own copy. Changing it without ever reading
    // it back almost always means the caller's variable was meant to change
    // (`x: &mut T`, which was spelled `mut x: T` before parameters took
    // ownership by default).
    for (i, l) in body.locals.iter().enumerate() {
        let id = LocalId(i as u32);
        if !matches!(l.kind, LocalKind::Param { owned: true, .. }) || !l.mutable || l.name.starts_with('_') || l.span.is_dummy() {
            continue;
        }
        if mutated.contains(&id) && !used.contains(&id) {
            diags.push(
                Diagnostic::warning(lint::MUT_PARAM_NOT_READ, l.span, format!("parameter `{}` is changed, but the new value is never used", l.name))
                    .with_label("this is the function's own copy")
                    .with_note("changes to a `mut` parameter do not reach the caller")
                    .with_help(format!("to change the caller's variable, take it in-out: `{}: &mut ...`", l.name)),
            );
        } else if !mutated.contains(&id) {
            diags.push(
                Diagnostic::warning(lint::UNUSED_MUT, l.span, format!("parameter `{}` does not need to be mutable", l.name))
                    .with_label("never mutated")
                    .with_help("remove the `mut`"),
            );
        }
    }
    for (i, l) in body.locals.iter().enumerate() {
        let id = LocalId(i as u32);
        if l.kind != LocalKind::Let || l.name.starts_with('_') || l.name.starts_with('<') || l.span.is_dummy() {
            continue;
        }
        if !used.contains(&id) {
            let (msg, label) = if mutated.contains(&id) {
                (format!("variable `{}` is assigned to, but never read", l.name), "assigned to but never read")
            } else {
                (format!("unused variable `{}`", l.name), "never used")
            };
            diags.push(
                Diagnostic::warning(lint::UNUSED_VARIABLE, l.span, msg)
                    .with_label(label)
                    .with_help(format!("if this is intentional, prefix it with an underscore: `_{}`", l.name)),
            );
        } else if l.mutable && !mutated.contains(&id) {
            diags.push(
                Diagnostic::warning(lint::UNUSED_MUT, l.span, format!("variable `{}` does not need to be mutable", l.name))
                    .with_label("never mutated")
                    .with_help("remove the `mut`"),
            );
        }
    }
    unreachable_block(&body.block, diags);
}

/// Whether evaluating `e` never finishes normally (`return`, `break`,
/// `continue`, a call returning `!`, a `loop` without `break`, or a block,
/// `if` or `match` all of whose paths do one of those).
fn diverges(e: &Expr) -> bool {
    match &e.kind {
        ExprKind::Coerce { kind: Coercion::NeverToAny, expr } => diverges(expr),
        _ => matches!(e.ty, Ty::Never),
    }
}

/// `lint::unreachable`: warn once per block, at the first statement (or the
/// tail) after one that always diverges. Code after that point is not
/// searched further, so one mistake gives one warning.
fn unreachable_block(b: &Block, diags: &mut Diagnostics) {
    for (i, s) in b.stmts.iter().enumerate() {
        let e = match s {
            Stmt::Let { init: Some(e), .. } | Stmt::Expr(e) => e,
            Stmt::Let { init: None, .. } => continue,
        };
        unreachable_expr(e, diags);
        if !diverges(e) {
            continue;
        }
        let next = match b.stmts.get(i + 1) {
            Some(Stmt::Let { span, .. }) => Some((*span, "statement")),
            Some(Stmt::Expr(n)) => Some((n.span, "statement")),
            None => b.tail.as_ref().map(|t| (t.span, "expression")),
        };
        if let Some((span, what)) = next
            && !span.is_dummy()
            && !e.span.is_dummy()
        {
            diags.push(
                Diagnostic::warning(lint::UNREACHABLE, span, format!("unreachable {what}"))
                    .with_label(format!("this {what} can never run"))
                    .with_secondary(e.span, "any code following this expression is unreachable, because it never finishes"),
            );
        }
        return;
    }
    if let Some(t) = &b.tail {
        unreachable_expr(t, diags);
    }
}

fn unreachable_expr(e: &Expr, diags: &mut Diagnostics) {
    match &e.kind {
        ExprKind::Block(b) | ExprKind::Loop { body: b, .. } => unreachable_block(b, diags),
        ExprKind::While { cond, body, .. } => {
            unreachable_expr(cond, diags);
            unreachable_block(body, diags);
        }
        ExprKind::ForRange { start, end, body, .. } => {
            unreachable_expr(start, diags);
            unreachable_expr(end, diags);
            unreachable_block(body, diags);
        }
        ExprKind::ForArray { array, body, .. } => {
            unreachable_expr(array, diags);
            unreachable_block(body, diags);
        }
        _ => visit_children(e, &mut |c| unreachable_expr(c, diags)),
    }
}

/// Check `body`, whose generic parameters have the bounds `env`.
pub fn check(p: &Program, body: &Body, env: &[crate::defs::Pred], diags: &mut Diagnostics) {
    let mut cx =
        Cx { p, body, env, diags, roots: HashMap::new(), reported: HashSet::new(), loop_exit: HashMap::new(), loop_head: HashMap::new(), held: Vec::new() };
    // A parameter that can hold a view points into its caller's data.
    for &l in &body.params {
        if cx.has_view(&cx.local(l).ty) {
            cx.roots.insert(l, [Root::Arg(l)].into());
        }
    }
    cx.compute_roots();
    cx.check_views_block(&body.block);
    if let Some(t) = &body.block.tail {
        cx.check_escape(t, t.span);
    }
    {
        let mut mv = Moves { cx: &mut cx, loop_states: HashMap::new() };
        let mut st = Some(BTreeSet::new());
        mv.block(&body.block, &mut st);
    }
    let _ = cx.live_block(&body.block, LiveSet::new());
}

struct Cx<'p, 'd> {
    p: &'p Program,
    body: &'p Body,
    env: &'p [crate::defs::Pred],
    diags: &'d mut Diagnostics,
    roots: HashMap<LocalId, Roots>,
    reported: HashSet<(Span, u32)>,
    loop_exit: HashMap<LoopId, LiveSet>,
    loop_head: HashMap<LoopId, LiveSet>,
    /// Roots of the other arguments of the call being checked.
    held: Vec<Roots>,
}

impl<'p> Cx<'p, '_> {
    fn local(&self, l: LocalId) -> &'p Local {
        self.body.local(l)
    }

    fn err(&mut self, d: Diagnostic) {
        self.diags.push(d);
    }

    /// Whether values of `t` point into storage they do not own.
    fn has_view(&self, t: &Ty) -> bool {
        self.has_view_rec(t, 0)
    }

    fn has_view_rec(&self, t: &Ty, depth: u32) -> bool {
        if depth > 32 {
            return false;
        }
        match t {
            Ty::Str | Ty::FmtArgs => true,
            Ty::Array(e, k) => k.is_slice() || self.has_view_rec(e, depth + 1),
            Ty::Tuple(ts) => ts.iter().any(|t| self.has_view_rec(t, depth + 1)),
            Ty::Adt(d, args) => {
                let adt = self.p.adt(*d);
                adt.variants.iter().any(|v| v.fields.iter().any(|f| self.has_view_rec(&f.ty.subst(args), depth + 1)))
            }
            Ty::Grow(t) => self.has_view_rec(t, depth + 1),
            Ty::Closure(d, args) => {
                self.p.closures.get(d).is_some_and(|c| c.captures.iter().any(|cap| cap.by_ref || self.has_view_rec(&cap.ty.subst(args), depth + 1)))
            }
            // Generic values may be instantiated with views, and `impl Trait`
            // may stand for one, unless bounded by `Dull`.
            Ty::Param(..) | Ty::Opaque(..) => !self.p.is_dull(t, self.env),
            _ => false,
        }
    }

    /// Whether a value of `t` has storage a view could point into: an owned
    /// array or string, or an inline array, anywhere inside it. A value
    /// without (a number, a `str`, an iterator holding only views) can only
    /// be pointed *through*.
    fn has_storage(&self, t: &Ty, depth: u32) -> bool {
        if depth > 32 {
            return true;
        }
        match t {
            Ty::String => true,
            // Owning and inline arrays are storage; a slice only points.
            Ty::Array(_, k) => !k.is_slice(),
            Ty::Tuple(ts) => ts.iter().any(|t| self.has_storage(t, depth + 1)),
            Ty::Adt(d, args) => {
                let adt = self.p.adt(*d);
                adt.variants.iter().any(|v| v.fields.iter().any(|f| self.has_storage(&f.ty.subst(args), depth + 1)))
            }
            Ty::Grow(t) => self.has_storage(t, depth + 1),
            Ty::Closure(d, args) => self.p.closures.get(d).is_none_or(|c| c.captures.iter().any(|cap| !cap.by_ref && self.has_storage(&cap.ty.subst(args), depth + 1))),
            // `impl Trait`: look at the type behind it once its function is
            // checked (like an auto trait, this is not part of the bounds).
            Ty::Opaque(d, i, args) => match self.p.opaques.get(&(*d, *i)).and_then(|o| o.concrete.as_ref()) {
                Some(c) => self.has_storage(&c.subst(args), depth + 1),
                None => !self.p.is_dull(t, self.env),
            },
            // Dull types hold no arrays; anything else generic might.
            Ty::Param(..) | Ty::Infer(_) => !self.p.is_dull(t, self.env),
            _ => false,
        }
    }

    /// Whether a declared type (in a signature) can hold a view by itself,
    /// not counting the function's generic parameters: their values are
    /// the caller's, copied in and out.
    fn has_own_view(&self, t: &Ty, depth: u32) -> bool {
        if depth > 32 {
            return true;
        }
        match t {
            Ty::Str | Ty::FmtArgs => true,
            Ty::Array(e, k) => k.is_slice() || self.has_own_view(e, depth + 1),
            Ty::Tuple(ts) => ts.iter().any(|t| self.has_own_view(t, depth + 1)),
            Ty::Adt(d, args) => {
                let adt = self.p.adt(*d);
                adt.variants.iter().any(|v| v.fields.iter().any(|f| self.has_own_view(&f.ty.subst(args), depth + 1)))
            }
            Ty::Grow(t) => self.has_own_view(t, depth + 1),
            Ty::Param(..) => false,
            Ty::Closure(..) | Ty::Opaque(..) => true,
            _ => false,
        }
    }

    // ===================================================================== roots

    fn place_root(&self, e: &Expr) -> Option<Root> {
        match &e.kind {
            ExprKind::Local(l) => Some(Root::Local(*l)),
            ExprKind::Field { base, .. } | ExprKind::Index { base, .. } => self.place_root(base),
            ExprKind::Temp(_) => Some(Root::Temp),
            _ => None,
        }
    }

    fn is_view_container(&self, t: &Ty) -> bool {
        matches!(t.strip_grow(), Ty::Array(_, k) if k.is_slice()) || matches!(t, Ty::Str)
    }

    /// Roots of a borrow of `place` (slicing it, passing an owner handle).
    fn borrow_roots(&self, place: &Expr) -> Roots {
        // Slicing a view stored in a field (`self.rest[i..]`): the data lives
        // wherever that view points, not in the struct holding it.
        if let ExprKind::Field { .. } = place.kind
            && self.is_view_container(&place.ty)
        {
            return self.value_roots_of_place(place);
        }
        match &place.kind {
            ExprKind::Local(l) => {
                if self.is_view_container(&self.local(*l).ty) {
                    // Slicing a slice: the data lives wherever the slice points.
                    self.roots.get(l).cloned().unwrap_or_default()
                } else {
                    // The data lives in `l` (unless it has no storage of its
                    // own, like an iterator holding only a `str`); if it
                    // holds views (an array of `str`), the borrow also
                    // reaches what they point into.
                    let mut r = Roots::new();
                    if self.has_storage(&self.local(*l).ty, 0) {
                        r.insert(Root::Local(*l));
                    }
                    if self.has_view(&self.local(*l).ty) {
                        r.extend(self.roots.get(l).into_iter().flatten().copied());
                    }
                    r
                }
            }
            ExprKind::Field { base, .. } | ExprKind::Index { base, .. } => {
                if self.is_view_container(&base.ty) {
                    self.value_roots_of_place(base)
                } else {
                    self.borrow_roots(base)
                }
            }
            ExprKind::Temp(inner) => {
                let mut r = self.value_roots(inner);
                // Slicing a temporary view (`s.as_bytes()[0..2]`) points
                // wherever that view points; only a temporary owner or
                // fixed array holds the data itself.
                if !self.is_view_container(&place.ty) {
                    r.insert(Root::Temp);
                }
                r
            }
            _ => Roots::new(),
        }
    }

    /// Roots of the value currently stored in `place`.
    fn value_roots_of_place(&self, place: &Expr) -> Roots {
        match self.place_root(place) {
            Some(Root::Local(l)) => self.roots.get(&l).cloned().unwrap_or_default(),
            Some(Root::Temp) => match &place.kind {
                ExprKind::Temp(inner) => self.value_roots(inner),
                _ => [Root::Temp].into(),
            },
            Some(Root::Slot(_) | Root::Arg(_)) | None => Roots::new(),
        }
    }

    fn value_roots(&self, e: &Expr) -> Roots {
        if !self.has_view(&e.ty) {
            return Roots::new();
        }
        match &e.kind {
            ExprKind::Read { place, .. } => self.value_roots_of_place(place),
            ExprKind::Slice { base, .. } => self.borrow_roots(base),
            ExprKind::Coerce { kind: Coercion::ToSlice { .. } | Coercion::StringToStr, expr } => self.borrow_roots(expr),
            ExprKind::Coerce { expr, .. } => self.value_roots(expr),
            ExprKind::Call { args, callee } => {
                let mut r = Roots::new();
                // What a closure returns may point at what it captured.
                if let Callee::Value { callee: f, .. } = callee {
                    r.extend(if f.is_place() { self.value_roots_of_place(f) } else { self.value_roots(f) });
                }
                // A plain function whose declared return type has no view of
                // its own (`pop(&mut self): Option::<T>`) cannot point into
                // its arguments' storage: it can only hand back copies of
                // items, which point where the items point. Trait methods
                // stay conservative: an implementation may return views
                // into itself.
                let copies_only = match callee {
                    Callee::Fn { def, .. } | Callee::Intrinsic { def, .. } => !self.has_own_view(&self.p.fn_data(*def).sig.ret, 0),
                    _ => false,
                };
                for a in args {
                    match a {
                        Arg::InOut(p) if copies_only => r.extend(self.value_roots_of_place(p)),
                        Arg::InOut(p) => r.extend(self.borrow_roots(p)),
                        Arg::Value(Expr { kind: ExprKind::Read { place, mode: ReadMode::Borrow }, .. }) if copies_only => {
                            r.extend(self.value_roots_of_place(place));
                        }
                        Arg::Value(v) => r.extend(self.arg_roots(v)),
                    }
                }
                r
            }
            ExprKind::Tuple(es) | ExprKind::Array(es) => es.iter().flat_map(|x| self.value_roots(x)).collect(),
            ExprKind::Adt { fields, .. } => fields.iter().flat_map(|(_, x)| self.value_roots(x)).collect(),
            // A borrowing closure points at the variables it borrows (and at
            // whatever they point into); one that took a value holds that
            // value's roots.
            ExprKind::Closure { captures, .. } => {
                let mut r = Roots::new();
                for a in captures {
                    match a {
                        Arg::InOut(p) | Arg::Value(p) if p.is_place() => {
                            match self.place_root(p) {
                                Some(Root::Local(l)) => {
                                    r.insert(Root::Slot(l));
                                }
                                Some(other) => {
                                    r.insert(other);
                                }
                                None => {}
                            }
                            if self.has_view(&p.ty) {
                                r.extend(self.value_roots_of_place(p));
                            }
                        }
                        Arg::InOut(_) => {}
                        Arg::Value(v) => r.extend(self.value_roots(v)),
                    }
                }
                r
            }
            ExprKind::Block(b) => b.tail.as_ref().map(|t| self.value_roots(t)).unwrap_or_default(),
            ExprKind::If { then, else_, .. } => {
                let mut r = self.value_roots(then);
                if let Some(e) = else_ {
                    r.extend(self.value_roots(e));
                }
                r
            }
            ExprKind::Match { arms, .. } => arms.iter().flat_map(|a| self.value_roots(&a.body)).collect(),
            ExprKind::FmtArgs(pieces) => pieces
                .iter()
                .flat_map(|p| match p {
                    FmtPiece::Arg(x, _) => self.arg_roots(x),
                    FmtPiece::Text(_) => Roots::new(),
                })
                .collect(),
            _ => Roots::new(),
        }
    }

    /// Which parameters of what is called are `kept`, when known.
    fn callee_kept(&self, c: &Callee) -> Option<Vec<bool>> {
        let def = match c {
            Callee::Fn { def, .. } | Callee::Intrinsic { def, .. } => *def,
            Callee::Trait { trait_ref, method, .. } => self.p.trait_data(trait_ref.trait_id).methods.iter().find(|m| m.name == *method)?.def,
            Callee::Indirect(_) | Callee::Value { .. } => return None,
        };
        Some(self.p.fn_data(def).sig.params.iter().map(|p| p.kept).collect())
    }

    /// The storage a write into `place` changes: the root local, or, for a
    /// write through a slice (`s[i] = v`), whatever the slice points into
    /// (an owner `Local`, or a parameter's data `Arg`). A slice variable
    /// `lent` in-out may be both reassigned and written through.
    fn write_targets(&self, place: &Expr, lent: bool) -> Vec<Root> {
        fn through(cx: &Cx, e: &Expr) -> Option<Roots> {
            match &e.kind {
                ExprKind::Field { base, .. } | ExprKind::Index { base, .. } => {
                    if cx.is_view_container(&base.ty) {
                        Some(cx.value_roots_of_place(base))
                    } else {
                        through(cx, base)
                    }
                }
                _ => None,
            }
        }
        let Some(Root::Local(l)) = self.place_root(place) else { return Vec::new() };
        let mut out = match through(self, place) {
            Some(roots) => roots.into_iter().collect(),
            None => vec![Root::Local(l)],
        };
        if lent && matches!(place.kind, ExprKind::Local(_)) && self.is_view_container(&self.local(l).ty) {
            out.extend(self.roots.get(&l).into_iter().flatten().copied());
        }
        out.retain(|r| matches!(r, Root::Local(_) | Root::Arg(_)));
        out
    }

    /// What a call may store where: a view from a `kept` argument may end up
    /// in any in-out argument that can hold one (`h.set(s)` doing
    /// `self.rest = s`, or `push` onto an array of views), or be written
    /// through a mutable slice argument (`s.fill(v)`); a closure may store
    /// any argument or capture into what it borrowed. Pairs of a target (a
    /// `Local`, or a parameter's data `Arg`) and the roots it may gain.
    fn call_flows(&self, callee: &Callee, args: &[Arg]) -> Vec<(Root, Roots)> {
        let mut targets: Vec<Root> = Vec::new();
        for a in args {
            match a {
                Arg::InOut(p) if self.has_view(&p.ty) => targets.extend(self.write_targets(p, true)),
                Arg::Value(v) if matches!(&v.ty, Ty::Array(e, ArrKind::SliceMut) if self.has_view(e)) => {
                    targets.extend(self.value_roots(v).into_iter().filter(|r| matches!(r, Root::Local(_) | Root::Arg(_))));
                }
                _ => {}
            }
        }
        let mut given = Roots::new();
        if let Callee::Value { callee: f, .. } = callee {
            let fr = if f.is_place() { self.value_roots_of_place(f) } else { self.value_roots(f) };
            for r in fr {
                match r {
                    Root::Slot(l) if self.has_view(&self.local(l).ty) => targets.push(Root::Local(l)),
                    Root::Slot(_) => {}
                    // A view the closure captured by value.
                    other => {
                        given.insert(other);
                    }
                }
            }
        }
        if targets.is_empty() {
            return Vec::new();
        }
        let kept = self.callee_kept(callee);
        for (i, a) in args.iter().enumerate() {
            if kept.as_ref().is_some_and(|k| !k.get(i).copied().unwrap_or(true)) {
                continue;
            }
            match a {
                Arg::Value(v) => given.extend(self.arg_roots(v)),
                Arg::InOut(p) if self.has_view(&p.ty) => given.extend(self.value_roots_of_place(p)),
                Arg::InOut(_) => {}
            }
        }
        let mut seen = BTreeSet::new();
        targets
            .into_iter()
            .filter(|t| seen.insert(*t))
            .filter_map(|t| {
                let (Root::Local(l) | Root::Arg(l)) = t else { return None };
                let r: Roots = given.iter().copied().filter(|r| !matches!(r, Root::Local(x) | Root::Slot(x) | Root::Arg(x) if *x == l)).collect();
                (!r.is_empty()).then_some((t, r))
            })
            .collect()
    }

    /// Check storing a view with `roots` into `target` (from
    /// [`write_targets`] or [`call_flows`]).
    fn check_store_into(&mut self, target: Root, roots: &Roots, span: Span) {
        match target {
            Root::Local(l) if matches!(self.local(l).kind, LocalKind::Param { owned: false, .. }) => self.check_store_into_param(l, roots, span),
            Root::Local(l) => self.check_store(l, roots, span),
            Root::Arg(p) => self.check_store_into_param(p, roots, span),
            Root::Slot(_) | Root::Temp => {}
        }
    }

    /// Whether local `l` is a parameter declared `kept`.
    fn is_kept_param(&self, l: LocalId) -> bool {
        match self.local(l).kind {
            LocalKind::Param { index, .. } => self.p.fn_data(self.body.def).sig.params.get(index).is_some_and(|p| p.kept),
            _ => false,
        }
    }

    /// Storing a view with `roots` into in-out parameter `dest` hands it to
    /// the caller: allowed for what `dest` already pointed into and for
    /// `kept` parameters' data, never for this function's own locals.
    fn check_store_into_param(&mut self, dest: LocalId, roots: &Roots, span: Span) {
        for r in roots {
            let (l, of_param) = match *r {
                Root::Local(x) | Root::Slot(x) => (Some(x), matches!(self.local(x).kind, LocalKind::Param { owned: false, .. })),
                Root::Arg(x) => (Some(x), true),
                Root::Temp => (None, false),
            };
            if l == Some(dest) || l.is_some_and(|x| of_param && self.is_kept_param(x)) {
                continue;
            }
            if !self.reported.insert((span, 9)) {
                return;
            }
            let dn = self.local(dest).name.clone();
            let d = match l {
                Some(x) if of_param => {
                    let n = self.local(x).name.clone();
                    let ls = self.local(x).span;
                    Diagnostic::error(borrow::ESCAPES_VIA_PARAM, span, format!("a view from parameter `{n}` may be stored into parameter `{dn}`"))
                        .with_secondary(ls, format!("`{n}` is not declared `kept`, so callers do not expect `{dn}` to point into it"))
                        .with_help(format!("declare it `kept {n}: ...`"))
                }
                _ => Diagnostic::error(borrow::ESCAPES_VIA_PARAM, span, format!("cannot store a view of a local value into parameter `{dn}`"))
                    .with_note("the caller's data may outlive the value being viewed"),
            };
            self.err(d);
            return;
        }
    }

    /// Roots contributed by a by-value argument: a borrowed owner handle (or
    /// fixed array) points into its owner.
    fn arg_roots(&self, v: &Expr) -> Roots {
        match &v.kind {
            ExprKind::Read { place, mode: ReadMode::Borrow } if self.p.has_owner(&place.ty) => self.borrow_roots(place),
            _ => self.value_roots(v),
        }
    }

    fn compute_roots(&mut self) {
        for _ in 0..16 {
            let mut changed = false;
            let mut assigns: Vec<(LocalId, Roots)> = Vec::new();
            collect_assigns_block(&self.body.block, self, &mut assigns);
            for (l, r) in assigns {
                let e = self.roots.entry(l).or_default();
                let before = e.len();
                e.extend(r);
                changed |= e.len() != before;
            }
            if !changed {
                break;
            }
        }
    }

    // ===================================================================== view checks

    fn root_name(&self, r: Root) -> String {
        match r {
            Root::Local(l) | Root::Slot(l) => format!("`{}`", self.local(l).name),
            Root::Arg(l) => format!("what `{}` points into", self.local(l).name),
            Root::Temp => "a temporary value".into(),
        }
    }

    /// Storing a view with `roots` into local `dest`.
    fn check_store(&mut self, dest: LocalId, roots: &Roots, span: Span) {
        let dd = self.local(dest).depth;
        let dname = self.local(dest).name.clone();
        for r in roots {
            match *r {
                Root::Temp => {
                    if !self.reported.insert((span, u32::MAX)) {
                        continue;
                    }
                    self.err(
                        Diagnostic::error(
                            borrow::OUTLIVES_ROOT,
                            span,
                            format!("`{dname}` would point into a temporary value that is freed at the end of this statement"),
                        )
                        .with_help("bind the value to a variable first so it lives long enough"),
                    );
                }
                Root::Local(l) | Root::Slot(l) => {
                    let rl = self.local(l);
                    if rl.depth > dd && !matches!(rl.kind, LocalKind::Param { .. }) {
                        if !self.reported.insert((span, l.0)) {
                            continue;
                        }
                        let rn = rl.name.clone();
                        let rs = rl.span;
                        self.err(
                            Diagnostic::error(borrow::OUTLIVES_ROOT, span, format!("`{rn}` does not live long enough"))
                                .with_label(format!("`{dname}` would keep pointing into `{rn}` after `{rn}` goes out of scope"))
                                .with_secondary(rs, format!("`{rn}` is declared in an inner scope here")),
                        );
                    }
                }
                // The caller's data outlives every local.
                Root::Arg(_) => {}
            }
        }
    }

    /// A value leaving the function (return / tail).
    fn check_escape(&mut self, v: &Expr, span: Span) {
        let roots = self.value_roots(v);
        for r in roots {
            let ok = match r {
                Root::Local(l) => match self.local(l).kind {
                    // A view into a borrowed (or in-out) owner points into the
                    // caller's value; an owned parameter is freed on return.
                    LocalKind::Param { owned, .. } => !owned,
                    _ => false,
                },
                Root::Arg(_) => true,
                Root::Slot(_) | Root::Temp => false,
            };
            if let Root::Slot(l) = r
                && self.reported.insert((span, 7))
            {
                let n = self.local(l).name.clone();
                self.err(
                    Diagnostic::error(
                        borrow::ESCAPES_FUNCTION,
                        span,
                        format!("cannot return a closure that borrows `{n}`, which is freed when the function returns"),
                    )
                    .with_help(format!("make it a `move` closure, so it takes `{n}` with it")),
                );
                continue;
            }
            if !ok && self.reported.insert((span, 7)) {
                let n = self.root_name(r);
                let mut d = Diagnostic::error(borrow::ESCAPES_FUNCTION, span, format!("cannot return a view of {n}, which is freed when the function returns"));
                if let Root::Local(l) = r
                    && let LocalKind::Param { owned, .. } = self.local(l).kind
                {
                    d = if owned && self.p.has_owner(&self.local(l).ty) {
                        d.with_note(format!("this function takes ownership of {n}, so it is freed when the function returns"))
                            .with_help("borrow the parameter instead (`: &T`), or return the owner itself")
                    } else {
                        d.with_note(format!("{n} has a Dull type, so it is passed as a copy owned by this function"))
                            .with_help("take a slice parameter or an in-out parameter (`: &mut T`) instead")
                    };
                }
                self.err(d);
            }
        }
    }

    fn check_views_block(&mut self, b: &Block) {
        for s in &b.stmts {
            match s {
                Stmt::Let { pat, init: Some(init), .. } => {
                    self.check_views(init);
                    if self.has_view(&init.ty) {
                        let roots = self.value_roots(init);
                        let mut binds = Vec::new();
                        pat_bindings(pat, &mut binds);
                        for l in binds {
                            self.check_store(l, &roots, init.span);
                        }
                    }
                }
                Stmt::Let { .. } => {}
                Stmt::Expr(e) => self.check_views(e),
            }
        }
        if let Some(t) = &b.tail {
            self.check_views(t);
        }
    }

    fn check_views(&mut self, e: &Expr) {
        match &e.kind {
            ExprKind::Assign { place, value } => {
                self.check_views(place);
                self.check_views(value);
                if self.has_view(&value.ty) {
                    let roots = self.value_roots(value);
                    if matches!(place.kind, ExprKind::Static(_)) {
                        if !roots.is_empty() {
                            self.err(Diagnostic::error(borrow::ESCAPES_FUNCTION, value.span, "cannot store a view of a local value into a static"));
                        }
                    } else {
                        for t in self.write_targets(place, false) {
                            let r: Roots = roots.iter().copied().filter(|r| !matches!((r, t), (Root::Local(x) | Root::Arg(x), Root::Local(y) | Root::Arg(y)) if *x == y)).collect();
                            self.check_store_into(t, &r, value.span);
                        }
                    }
                }
            }
            ExprKind::Call { callee, args } => {
                visit_children(e, &mut |c| self.check_views(c));
                for (t, roots) in self.call_flows(callee, args) {
                    self.check_store_into(t, &roots, e.span);
                }
            }
            ExprKind::Return(Some(v)) => {
                self.check_views(v);
                self.check_escape(v, v.span);
            }
            ExprKind::Block(b) | ExprKind::Loop { body: b, .. } => self.check_views_block(b),
            ExprKind::While { cond, body, .. } => {
                self.check_views(cond);
                self.check_views_block(body);
            }
            ExprKind::ForRange { start, end, body, .. } => {
                self.check_views(start);
                self.check_views(end);
                self.check_views_block(body);
            }
            ExprKind::ForArray { array, body, .. } => {
                self.check_views(array);
                self.check_views_block(body);
            }
            _ => visit_children(e, &mut |c| self.check_views(c)),
        }
    }

    fn place_through_view(&self, place: &Expr) -> bool {
        match &place.kind {
            ExprKind::Field { base, .. } | ExprKind::Index { base, .. } => self.is_view_container(&base.ty) || self.place_through_view(base),
            _ => false,
        }
    }

    // ===================================================================== liveness & invalidation

    fn live_block(&mut self, b: &Block, after: LiveSet) -> LiveSet {
        let mut live = after;
        if let Some(t) = &b.tail {
            live = self.live(t, live);
        }
        for s in b.stmts.iter().rev() {
            live = match s {
                Stmt::Let { pat, init, .. } => {
                    let mut binds = Vec::new();
                    pat_bindings(pat, &mut binds);
                    for l in binds {
                        live.remove(&l);
                    }
                    match init {
                        Some(i) => self.live(i, live),
                        None => live,
                    }
                }
                Stmt::Expr(e) => self.live(e, live),
            };
        }
        live
    }

    fn use_place(&mut self, place: &Expr, mut live: LiveSet) -> LiveSet {
        match &place.kind {
            ExprKind::Local(l) => {
                live.insert(*l);
                live
            }
            ExprKind::Field { base, .. } => self.use_place(base, live),
            ExprKind::Index { base, index } => {
                let live = self.live(index, live);
                self.use_place(base, live)
            }
            ExprKind::Deref(p) => self.live(p, live),
            ExprKind::Temp(inner) => self.live(inner, live),
            _ => live,
        }
    }

    /// Report views of `root` that are live after an invalidating event.
    fn invalidate(&mut self, root: LocalId, live_after: &LiveSet, span: Span, what: &str) {
        let rname = self.local(root).name.clone();
        // A closure borrowing the variable itself only minds it being moved:
        // replacing or shrinking it changes what the closure sees.
        let violators: Vec<LocalId> = live_after
            .iter()
            .copied()
            .filter(|x| *x != root && self.roots.get(x).is_some_and(|r| r.contains(&Root::Local(root)) || (what == "move" && r.contains(&Root::Slot(root)))))
            .collect();
        for v in violators {
            if !self.reported.insert((span, v.0)) {
                continue;
            }
            let vn = self.local(v).name.clone();
            let vs = self.local(v).span;
            let (code, verb, help) = match what {
                "shrink" => (
                    borrow::SHRINK_WHILE_BORROWED,
                    "may be shrunk or replaced",
                    format!("finish using `{vn}` before this point, or take a new slice afterwards; if this call only grows `{rname}`, declare that parameter `&mut +`"),
                ),
                "move" => (borrow::MOVE_WHILE_BORROWED, "is moved", format!("finish using `{vn}` before this point, or take a new slice afterwards")),
                _ => (borrow::ASSIGN_WHILE_BORROWED, "is replaced", format!("finish using `{vn}` before this point, or take a new slice afterwards")),
            };
            self.err(
                Diagnostic::error(code, span, format!("`{rname}` {verb} here while `{vn}` still points into it"))
                    .with_secondary(vs, format!("`{vn}` borrows from `{rname}` and is used later"))
                    .with_help(help),
            );
        }
        let held_conflict = self.held.iter().any(|r| r.contains(&Root::Local(root)));
        if held_conflict && what == "shrink" && self.reported.insert((span, u32::MAX - 1)) {
            self.err(Diagnostic::error(
                borrow::SHRINK_WHILE_BORROWED,
                span,
                format!("`{rname}` may be shrunk or replaced here while another argument of this call points into it"),
            ));
        }
    }

    /// `place` (an owner) is about to be lent where it may be shrunk or
    /// replaced (`what`): its root local, to invalidate. A `&mut +`
    /// parameter may only grow, so that is an error.
    fn may_shrink(&mut self, place: &Expr, span: Span, what: &str) -> Option<LocalId> {
        let Some(Root::Local(root)) = self.place_root(place) else { return None };
        if self.place_through_view(place) {
            return None;
        }
        self.check_grow_only(root, span, what);
        Some(root)
    }

    /// Report shrinking or replacing (`what`) local `l` if it is a `&mut +`
    /// parameter.
    fn check_grow_only(&mut self, l: LocalId, span: Span, what: &str) {
        let local = self.local(l);
        if let LocalKind::Param { grow: true, .. } = local.kind
            && self.reported.insert((span, 1 << 29))
        {
            let n = local.name.clone();
            let ls = local.span;
            self.err(
                Diagnostic::error(borrow::GROW_ONLY, span, format!("`{n}` may only grow, but {what}"))
                    .with_secondary(ls, format!("`{n}` is declared `&mut +`, so callers keep their views into it across the call"))
                    .with_help(format!("only grow `{n}` (with `&mut +` methods such as `push`), or drop the `+` so it may be shrunk")),
            );
        }
    }

    /// The parameter types of what is called, when known.
    fn callee_params(&self, c: &Callee) -> Option<Vec<Ty>> {
        match c {
            Callee::Fn { def, .. } | Callee::Intrinsic { def, .. } => Some(self.p.fn_data(*def).sig.params.iter().map(|p| p.ty.clone()).collect()),
            Callee::Trait { trait_ref, method, .. } => {
                let t = self.p.trait_data(trait_ref.trait_id);
                let m = t.methods.iter().find(|m| m.name == *method)?;
                Some(self.p.fn_data(m.def).sig.params.iter().map(|p| p.ty.clone()).collect())
            }
            Callee::Indirect(f) => match &f.ty {
                Ty::FnPtr(ps, _) => Some(ps.iter().map(|(_, t)| t.clone()).collect()),
                _ => None,
            },
            Callee::Value { .. } => None,
        }
    }

    /// Whether a closure value of type `t` may change what it captured when
    /// called (conservatively yes, unless it is known to be an `Fn` closure).
    fn may_change_captures(&self, t: &Ty) -> bool {
        match t {
            Ty::Closure(d, _) => self.p.closures.get(d).is_none_or(|c| c.kind != crate::ClosureKind::Fn),
            _ => true,
        }
    }

    /// Calling (or handing on) a closure whose roots are `roots` may shrink
    /// or replace the owners it borrowed in-out.
    fn invalidate_captures(&mut self, roots: &Roots, after: &LiveSet, span: Span) {
        let slots: Vec<LocalId> = roots
            .iter()
            .filter_map(|r| match r {
                Root::Slot(l) if self.p.has_owner(&self.local(*l).ty) => Some(*l),
                _ => None,
            })
            .collect();
        for l in slots {
            self.check_grow_only(l, span, "a closure that may shrink or replace it is called here");
            self.invalidate(l, after, span, "shrink");
        }
    }

    fn live(&mut self, e: &Expr, after: LiveSet) -> LiveSet {
        use ExprKind as K;
        match &e.kind {
            K::Local(_) | K::Field { .. } | K::Index { .. } | K::Deref(_) | K::Temp(_) | K::Static(_) => self.use_place(e, after),
            K::Read { place, mode } => {
                if *mode == ReadMode::Move
                    && self.p.has_owner(&place.ty)
                    && let Some(Root::Local(r)) = self.place_root(place)
                {
                    self.invalidate(r, &after, e.span, "move");
                }
                self.use_place(place, after)
            }
            K::Continue { id } => self.loop_head.get(id).cloned().unwrap_or_default(),
            K::Lit(_) | K::FnRef { .. } | K::Error => after,
            K::Call { callee, args } => {
                let params = self.callee_params(callee);
                let arg_roots: Vec<Roots> = args
                    .iter()
                    .map(|a| match a {
                        Arg::InOut(p) => self.borrow_roots(p),
                        Arg::Value(v) => self.arg_roots(v),
                    })
                    .collect();
                for (i, a) in args.iter().enumerate() {
                    // Lent in-out to a parameter that is not `+`: the callee
                    // may shrink or replace it.
                    let grows = params.as_ref().and_then(|ps| ps.get(i)).is_some_and(|t| matches!(t, Ty::Grow(_)));
                    if let Arg::InOut(p) = a
                        && !grows
                        && self.p.has_owner(&p.ty)
                        && let Some(root) = self.may_shrink(p, p.span, "this call may shrink or replace it")
                    {
                        self.held = arg_roots.iter().enumerate().filter(|(j, _)| *j != i).map(|(_, r)| r.clone()).collect();
                        self.invalidate(root, &after, p.span, "shrink");
                        self.held.clear();
                    }
                    // A closure that changes what it borrowed may be called
                    // by the callee.
                    let ty = match a {
                        Arg::InOut(p) => &p.ty,
                        Arg::Value(v) => &v.ty,
                    };
                    if self.may_change_captures(ty) {
                        let roots = arg_roots[i].clone();
                        self.invalidate_captures(&roots, &after, e.span);
                    }
                }
                if let Callee::Value { callee: f, kind } = callee
                    && *kind != crate::ClosureKind::Fn
                {
                    let roots = if f.is_place() { self.value_roots_of_place(f) } else { self.value_roots(f) };
                    self.invalidate_captures(&roots, &after, e.span);
                }
                let mut live = after;
                for a in args.iter().rev() {
                    live = match a {
                        Arg::InOut(p) => self.use_place(p, live),
                        Arg::Value(v) => self.live(v, live),
                    };
                }
                match callee {
                    Callee::Indirect(f) => live = self.live(f, live),
                    Callee::Value { callee: f, .. } => {
                        live = if f.is_place() { self.use_place(f, live) } else { self.live(f, live) };
                    }
                    _ => {}
                }
                live
            }
            K::Closure { captures, .. } => captures.iter().rev().fold(after, |l, a| match a {
                Arg::InOut(p) => self.use_place(p, l),
                Arg::Value(v) if v.is_place() => self.use_place(v, l),
                Arg::Value(v) => self.live(v, l),
            }),
            K::Assign { place, value } => {
                let mut live = after.clone();
                if self.p.has_owner(&place.ty)
                    && let Some(Root::Local(r)) = self.place_root(place)
                    && !self.place_through_view(place)
                {
                    self.check_grow_only(r, e.span, "this replaces part of it");
                    self.invalidate(r, &after, e.span, "assign");
                }
                if let K::Local(l) = place.kind {
                    live.remove(&l);
                } else {
                    live = self.use_place(place, live);
                }
                self.live(value, live)
            }
            K::AssignOp { place, value, .. } => {
                let live = self.use_place(place, after);
                self.live(value, live)
            }
            K::Unary(_, x) | K::Cast(x) | K::Coerce { expr: x, .. } => self.live(x, after),
            K::Binary(_, a, b) => {
                let l = self.live(b, after);
                self.live(a, l)
            }
            K::Logical { lhs, rhs, .. } => {
                let mut l = self.live(rhs, after.clone());
                l.extend(after);
                self.live(lhs, l)
            }
            K::Tuple(es) | K::Array(es) => es.iter().rev().fold(after, |l, x| self.live(x, l)),
            K::Adt { fields, .. } => fields.iter().rev().fold(after, |l, (_, x)| self.live(x, l)),
            K::ArrayRepeat { value, count } => {
                let l = self.live(count, after);
                self.live(value, l)
            }
            K::Slice { base, start, end, .. } => {
                let mut l = after;
                if let Some(x) = end {
                    l = self.live(x, l);
                }
                if let Some(x) = start {
                    l = self.live(x, l);
                }
                self.use_place(base, l)
            }
            K::Block(b) => self.live_block(b, after),
            K::If { cond, then, else_ } => {
                let mut l = self.live(then, after.clone());
                match else_ {
                    Some(x) => l.extend(self.live(x, after)),
                    None => l.extend(after),
                }
                self.live(cond, l)
            }
            K::Match { scrut, arms } => {
                let mut l = LiveSet::new();
                for a in arms {
                    let mut x = self.live(&a.body, after.clone());
                    if let Some(g) = &a.guard {
                        x = self.live(g, x);
                    }
                    l.extend(x);
                }
                self.use_place(scrut, l)
            }
            K::Loop { body, id } => {
                self.loop_exit.insert(*id, after);
                let mut head = LiveSet::new();
                for _ in 0..8 {
                    self.loop_head.insert(*id, head.clone());
                    let new = self.live_block(body, head.clone());
                    if new.is_subset(&head) {
                        break;
                    }
                    head.extend(new);
                }
                head
            }
            K::While { cond, body, id } => {
                self.loop_exit.insert(*id, after.clone());
                let mut head = after.clone();
                for _ in 0..8 {
                    self.loop_head.insert(*id, head.clone());
                    let mut b = self.live_block(body, head.clone());
                    b.extend(after.iter().copied());
                    let new = self.live(cond, b);
                    if new.is_subset(&head) {
                        break;
                    }
                    head.extend(new);
                }
                head
            }
            K::ForRange { start, end, body, id, .. } => {
                self.loop_exit.insert(*id, after.clone());
                let mut head = self.live(end, after.clone());
                for _ in 0..8 {
                    self.loop_head.insert(*id, head.clone());
                    let mut b = self.live_block(body, head.clone());
                    b.extend(after.iter().copied());
                    let new = self.live(end, b);
                    if new.is_subset(&head) {
                        break;
                    }
                    head.extend(new);
                }
                self.live(start, head)
            }
            K::ForArray { array, body, id, .. } => {
                self.loop_exit.insert(*id, after.clone());
                let mut head = self.use_place(array, after.clone());
                for _ in 0..8 {
                    self.loop_head.insert(*id, head.clone());
                    let mut b = self.live_block(body, head.clone());
                    b.extend(after.iter().copied());
                    let new = self.use_place(array, b);
                    if new.is_subset(&head) {
                        break;
                    }
                    head.extend(new);
                }
                head
            }
            K::Break { id, value } => {
                let exit = self.loop_exit.get(id).cloned().unwrap_or_default();
                match value {
                    Some(v) => self.live(v, exit),
                    None => exit,
                }
            }
            K::Return(v) => match v {
                Some(v) => self.live(v, LiveSet::new()),
                None => LiveSet::new(),
            },
            K::FmtArgs(pieces) => pieces.iter().rev().fold(after, |l, p| match p {
                FmtPiece::Arg(x, _) => self.live(x, l),
                FmtPiece::Text(_) => l,
            }),
        }
    }
}

fn pat_bindings(p: &Pat, out: &mut Vec<LocalId>) {
    match &p.kind {
        PatKind::Binding { local, sub, .. } => {
            out.push(*local);
            if let Some(s) = sub {
                pat_bindings(s, out);
            }
        }
        PatKind::Tuple(ps) | PatKind::Or(ps) => ps.iter().for_each(|p| pat_bindings(p, out)),
        PatKind::Adt { fields, .. } => fields.iter().for_each(|(_, p)| pat_bindings(p, out)),
        _ => {}
    }
}

/// Call `f` on each direct child expression.
pub(crate) fn visit_children(e: &Expr, f: &mut dyn FnMut(&Expr)) {
    use ExprKind as K;
    match &e.kind {
        K::Local(_) | K::Static(_) | K::Lit(_) | K::FnRef { .. } | K::Continue { .. } | K::Error => {}
        K::Field { base, .. } => f(base),
        K::Index { base, index } => {
            f(base);
            f(index);
        }
        K::Deref(x) | K::Temp(x) | K::Unary(_, x) | K::Cast(x) | K::Coerce { expr: x, .. } => f(x),
        K::Read { place, .. } => f(place),
        K::Call { callee, args } => {
            if let Callee::Indirect(c) | Callee::Value { callee: c, .. } = callee {
                f(c);
            }
            for a in args {
                f(a.expr());
            }
        }
        K::Closure { captures, .. } => captures.iter().for_each(|a| f(a.expr())),
        K::Binary(_, a, b) | K::Logical { lhs: a, rhs: b, .. } | K::Assign { place: a, value: b } | K::AssignOp { place: a, value: b, .. } => {
            f(a);
            f(b);
        }
        K::Tuple(es) | K::Array(es) => es.iter().for_each(|x| f(x)),
        K::Adt { fields, .. } => fields.iter().for_each(|(_, x)| f(x)),
        K::ArrayRepeat { value, count } => {
            f(value);
            f(count);
        }
        K::Slice { base, start, end, .. } => {
            f(base);
            if let Some(x) = start {
                f(x);
            }
            if let Some(x) = end {
                f(x);
            }
        }
        K::Block(b) | K::Loop { body: b, .. } => visit_block(b, f),
        K::If { cond, then, else_ } => {
            f(cond);
            f(then);
            if let Some(x) = else_ {
                f(x);
            }
        }
        K::Match { scrut, arms } => {
            f(scrut);
            for a in arms {
                if let Some(g) = &a.guard {
                    f(g);
                }
                f(&a.body);
            }
        }
        K::While { cond, body, .. } => {
            f(cond);
            visit_block(body, f);
        }
        K::ForRange { start, end, body, .. } => {
            f(start);
            f(end);
            visit_block(body, f);
        }
        K::ForArray { array, body, .. } => {
            f(array);
            visit_block(body, f);
        }
        K::Break { value, .. } | K::Return(value) => {
            if let Some(x) = value {
                f(x);
            }
        }
        K::FmtArgs(pieces) => {
            for p in pieces {
                if let FmtPiece::Arg(x, _) = p {
                    f(x);
                }
            }
        }
    }
}

pub(crate) fn visit_block(b: &Block, f: &mut dyn FnMut(&Expr)) {
    for s in &b.stmts {
        match s {
            Stmt::Let { init: Some(i), .. } => f(i),
            Stmt::Let { .. } => {}
            Stmt::Expr(e) => f(e),
        }
    }
    if let Some(t) = &b.tail {
        f(t);
    }
}

fn collect_assigns_block(b: &Block, cx: &Cx, out: &mut Vec<(LocalId, Roots)>) {
    for s in &b.stmts {
        match s {
            Stmt::Let { pat, init: Some(init), .. } => {
                let r = cx.value_roots(init);
                if !r.is_empty() {
                    let mut binds = Vec::new();
                    pat_bindings(pat, &mut binds);
                    for l in binds {
                        out.push((l, r.clone()));
                    }
                }
                collect_assigns(init, cx, out);
            }
            Stmt::Let { .. } => {}
            Stmt::Expr(e) => collect_assigns(e, cx, out),
        }
    }
    if let Some(t) = &b.tail {
        collect_assigns(t, cx, out);
    }
}

fn collect_assigns(e: &Expr, cx: &Cx, out: &mut Vec<(LocalId, Roots)>) {
    match &e.kind {
        ExprKind::Assign { place, value } => {
            let r = cx.value_roots(value);
            if !r.is_empty() {
                // Into the variable, or through a slice into what it views.
                for t in cx.write_targets(place, false) {
                    if let Root::Local(l) = t {
                        out.push((l, r.iter().copied().filter(|x| *x != Root::Local(l)).collect()));
                    }
                }
            }
        }
        ExprKind::Call { callee, args } => {
            for (t, r) in cx.call_flows(callee, args) {
                if let Root::Local(l) = t {
                    out.push((l, r));
                }
            }
        }
        ExprKind::Match { scrut, arms } => {
            // Bindings inside arms view whatever the scrutinee views.
            let r = if cx.has_view(&scrut.ty) { cx.value_roots_of_place(scrut) } else { Roots::new() };
            if !r.is_empty() {
                for a in arms {
                    let mut binds = Vec::new();
                    pat_bindings(&a.pat, &mut binds);
                    for l in binds {
                        if cx.has_view(&cx.local(l).ty) {
                            out.push((l, r.clone()));
                        }
                    }
                }
            }
        }
        _ => {}
    }
    match &e.kind {
        ExprKind::Block(b) | ExprKind::Loop { body: b, .. } => collect_assigns_block(b, cx, out),
        ExprKind::While { body, cond, .. } => {
            collect_assigns(cond, cx, out);
            collect_assigns_block(body, cx, out);
        }
        ExprKind::ForRange { body, .. } | ExprKind::ForArray { body, .. } => collect_assigns_block(body, cx, out),
        _ => visit_children(e, &mut |c| collect_assigns(c, cx, out)),
    }
}

// ========================================================================= moves

type MoveState = Option<BTreeSet<PathP>>;

struct Moves<'c, 'p, 'd> {
    cx: &'c mut Cx<'p, 'd>,
    /// Per loop: (state at `continue`s, state at `break`s).
    loop_states: HashMap<LoopId, (MoveState, MoveState)>,
}

fn join(a: &mut MoveState, b: MoveState) {
    match (a.as_mut(), b) {
        (_, None) => {}
        (None, b) => *a = b,
        (Some(x), Some(y)) => x.extend(y),
    }
}

impl Moves<'_, '_, '_> {
    fn path_of(&self, e: &Expr) -> Option<PathP> {
        match &e.kind {
            ExprKind::Local(l) => Some((*l, vec![])),
            ExprKind::Field { base, field, variant } => {
                let (l, mut p) = self.path_of(base)?;
                p.push(*variant * 1000 + *field);
                Some((l, p))
            }
            _ => None,
        }
    }

    fn check_use(&mut self, place: &Expr, st: &MoveState, span: Span) {
        let Some(s) = st else { return };
        let path = self.path_of(place).or_else(|| match self.cx.place_root(place) {
            Some(Root::Local(l)) => Some((l, vec![])),
            _ => None,
        });
        let Some((l, path)) = path else { return };
        let moved = s.iter().any(|(ml, mp)| *ml == l && (mp.starts_with(&path) || path.starts_with(mp)));
        if moved && self.cx.reported.insert((span, 1 << 30)) {
            let n = self.cx.local(l).name.clone();
            self.cx.err(
                Diagnostic::error(borrow::USE_AFTER_MOVE, span, format!("use of moved value `{n}`"))
                    .with_note(format!("`{n}` owns its data, so binding, assigning, returning or passing it to a parameter that takes ownership moves it"))
                    .with_help("move it only once, or `.clone()` it before moving"),
            );
        }
    }

    fn read(&mut self, place: &Expr, mode: ReadMode, st: &mut MoveState, span: Span) {
        self.place_inner(place, st);
        self.check_use(place, st, span);
        if mode != ReadMode::Move {
            return;
        }
        match self.cx.place_root(place) {
            Some(Root::Local(l)) => {
                let local = self.cx.local(l);
                if let LocalKind::Param { owned: false, inout, .. } = local.kind {
                    let n = local.name.clone();
                    let borrow = if inout { "&mut " } else { "&" };
                    let take = if n == "self" {
                        format!("`self` rather than `{borrow}self`")
                    } else {
                        let ty = self.cx.p.ty_str(local.ty.strip_grow());
                        format!("`{n}: {ty}` rather than `{n}: {borrow}{ty}`")
                    };
                    if self.cx.reported.insert((span, 3 << 28)) {
                        self.cx.err(
                            Diagnostic::error(borrow::MOVE_OUT_OF_BORROW, span, format!("cannot move out of parameter `{n}`"))
                                .with_note("this parameter is borrowed from the caller, so its owned data cannot be taken")
                                .with_help(format!("take ownership instead ({take}), or use `.clone()` to make an owned copy")),
                        );
                    }
                    return;
                }
                if self.cx.place_through_view(place) || matches!(place.kind, ExprKind::Index { .. }) {
                    if self.cx.reported.insert((span, 5 << 27)) {
                        self.cx.err(Diagnostic::error(borrow::MOVE_OUT_OF_BORROW, span, "cannot move an owned value out of an array element"));
                    }
                } else if let Some(p) = self.path_of(place)
                    && let Some(s) = st
                {
                    s.insert(p);
                }
            }
            Some(Root::Temp | Root::Slot(_) | Root::Arg(_)) => {}
            None => {
                if self.cx.reported.insert((span, 6 << 27)) {
                    self.cx.err(Diagnostic::error(borrow::MOVE_OUT_OF_BORROW, span, "cannot move an owned value out of a static or through a pointer"));
                }
            }
        }
    }

    /// Visit sub-expressions inside a place (index expressions, temps).
    fn place_inner(&mut self, place: &Expr, st: &mut MoveState) {
        match &place.kind {
            ExprKind::Field { base, .. } => self.place_inner(base, st),
            ExprKind::Index { base, index } => {
                self.place_inner(base, st);
                self.expr(index, st);
            }
            ExprKind::Deref(p) => self.expr(p, st),
            ExprKind::Temp(inner) => self.expr(inner, st),
            _ => {}
        }
    }

    /// Variables bound by `pat` are freshly initialized (in a loop, the
    /// previous iteration may have moved them).
    fn bind(&mut self, pat: &Pat, st: &mut MoveState) {
        if let Some(s) = st {
            walk_pat(pat, &mut |p| {
                if let PatKind::Binding { local, .. } = p.kind {
                    s.retain(|(l, _)| *l != local);
                }
            });
        }
    }

    fn block(&mut self, b: &Block, st: &mut MoveState) {
        for s in &b.stmts {
            match s {
                Stmt::Let { pat, init, .. } => {
                    if let Some(i) = init {
                        self.expr(i, st);
                    }
                    self.bind(pat, st);
                }
                Stmt::Expr(e) => self.expr(e, st),
            }
        }
        if let Some(t) = &b.tail {
            self.expr(t, st);
        }
    }

    fn pat_moves(&mut self, scrut: &Expr, pat: &Pat, st: &mut MoveState) {
        let mut has_move = false;
        walk_pat(pat, &mut |p| {
            if let PatKind::Binding { mode: ReadMode::Move, .. } = p.kind {
                has_move = true;
            }
        });
        if has_move {
            self.read(scrut, ReadMode::Move, st, pat.span);
        }
    }

    fn expr(&mut self, e: &Expr, st: &mut MoveState) {
        use ExprKind as K;
        match &e.kind {
            K::Read { place, mode } => self.read(place, *mode, st, e.span),
            K::Local(_) | K::Field { .. } | K::Index { .. } | K::Deref(_) | K::Temp(_) | K::Static(_) => {
                self.place_inner(e, st);
                self.check_use(e, st, e.span);
            }
            K::Call { callee, args } => {
                match callee {
                    Callee::Indirect(f) => self.expr(f, st),
                    Callee::Value { callee: f, .. } => self.expr(f, st),
                    _ => {}
                }
                for a in args {
                    match a {
                        Arg::Value(v) => self.expr(v, st),
                        Arg::InOut(p) => {
                            self.place_inner(p, st);
                            self.check_use(p, st, p.span);
                        }
                    }
                }
            }
            K::Assign { place, value } => {
                self.expr(value, st);
                self.place_inner(place, st);
                if let Some(path) = self.path_of(place)
                    && let Some(s) = st
                {
                    s.retain(|(l, p)| !(*l == path.0 && p.starts_with(&path.1)));
                }
            }
            K::AssignOp { place, value, .. } => {
                self.expr(value, st);
                self.place_inner(place, st);
                self.check_use(place, st, place.span);
            }
            K::Slice { base, start, end, .. } => {
                self.place_inner(base, st);
                self.check_use(base, st, e.span);
                if let Some(x) = start {
                    self.expr(x, st);
                }
                if let Some(x) = end {
                    self.expr(x, st);
                }
            }
            K::Coerce { kind: Coercion::ToSlice { .. } | Coercion::StringToStr, expr } => {
                self.place_inner(expr, st);
                self.check_use(expr, st, e.span);
            }
            K::Coerce { expr, .. } | K::Unary(_, expr) | K::Cast(expr) => self.expr(expr, st),
            K::Binary(_, a, b) => {
                self.expr(a, st);
                self.expr(b, st);
            }
            K::Logical { lhs, rhs, .. } => {
                self.expr(lhs, st);
                let mut s2 = st.clone();
                self.expr(rhs, &mut s2);
                join(st, s2);
            }
            K::Tuple(es) | K::Array(es) => es.iter().for_each(|x| self.expr(x, st)),
            K::Adt { fields, .. } => fields.iter().for_each(|(_, x)| self.expr(x, st)),
            K::ArrayRepeat { value, count } => {
                self.expr(value, st);
                self.expr(count, st);
            }
            K::FmtArgs(pieces) => {
                for p in pieces {
                    if let FmtPiece::Arg(x, _) = p {
                        self.expr(x, st);
                    }
                }
            }
            K::If { cond, then, else_ } => {
                self.expr(cond, st);
                let mut a = st.clone();
                self.expr(then, &mut a);
                let mut b = st.clone();
                if let Some(x) = else_ {
                    self.expr(x, &mut b);
                }
                *st = a;
                join(st, b);
            }
            K::Match { scrut, arms } => {
                self.place_inner(scrut, st);
                self.check_use(scrut, st, scrut.span);
                let mut out: MoveState = None;
                for a in arms {
                    let mut s = st.clone();
                    self.pat_moves(scrut, &a.pat, &mut s);
                    self.bind(&a.pat, &mut s);
                    if let Some(g) = &a.guard {
                        self.expr(g, &mut s);
                    }
                    self.expr(&a.body, &mut s);
                    join(&mut out, s);
                }
                *st = out;
            }
            K::Block(b) => self.block(b, st),
            K::Loop { body, id } => self.looped(*id, st, &mut |m, s| m.block(body, s), false),
            K::While { cond, body, id } => {
                self.expr(cond, st);
                self.looped(
                    *id,
                    st,
                    &mut |m, s| {
                        m.block(body, s);
                        m.expr(cond, s);
                    },
                    true,
                );
            }
            K::ForRange { start, end, body, id, .. } => {
                self.expr(start, st);
                self.expr(end, st);
                self.looped(*id, st, &mut |m, s| m.block(body, s), true);
            }
            K::ForArray { pat, array, body, id, .. } => {
                self.place_inner(array, st);
                self.check_use(array, st, array.span);
                self.looped(
                    *id,
                    st,
                    &mut |m, s| {
                        m.bind(pat, s);
                        m.block(body, s);
                    },
                    true,
                );
            }
            K::Break { id, value } => {
                if let Some(v) = value {
                    self.expr(v, st);
                }
                let entry = self.loop_states.entry(*id).or_insert((None, None));
                join(&mut entry.1, st.clone());
                *st = None;
            }
            K::Continue { id } => {
                let entry = self.loop_states.entry(*id).or_insert((None, None));
                join(&mut entry.0, st.clone());
                *st = None;
            }
            K::Return(v) => {
                if let Some(v) = v {
                    self.expr(v, st);
                }
                *st = None;
            }
            K::Closure { captures, .. } => {
                for a in captures {
                    match a {
                        Arg::Value(v) if !v.is_place() => self.expr(v, st),
                        Arg::Value(p) | Arg::InOut(p) => {
                            self.place_inner(p, st);
                            self.check_use(p, st, p.span);
                        }
                    }
                }
            }
            K::Lit(_) | K::FnRef { .. } | K::Error => {}
        }
    }

    /// Run a loop body twice so moves in one iteration are seen by the next.
    fn looped(&mut self, id: LoopId, st: &mut MoveState, body: &mut dyn FnMut(&mut Self, &mut MoveState), may_skip: bool) {
        let mut s = st.clone();
        body(self, &mut s);
        let mut head = st.clone();
        join(&mut head, s);
        if let Some((cont, _)) = self.loop_states.get(&id).cloned() {
            join(&mut head, cont);
        }
        let mut s2 = head.clone();
        body(self, &mut s2);
        let breaks = self.loop_states.remove(&id).map(|x| x.1).unwrap_or(None);
        let mut exit: MoveState = if may_skip { head } else { None };
        if may_skip {
            join(&mut exit, s2);
        }
        join(&mut exit, breaks);
        *st = exit;
    }
}

fn walk_pat(p: &Pat, f: &mut dyn FnMut(&Pat)) {
    f(p);
    match &p.kind {
        PatKind::Binding { sub: Some(s), .. } => walk_pat(s, f),
        PatKind::Tuple(ps) | PatKind::Or(ps) => ps.iter().for_each(|x| walk_pat(x, f)),
        PatKind::Adt { fields, .. } => fields.iter().for_each(|(_, x)| walk_pat(x, f)),
        _ => {}
    }
}
