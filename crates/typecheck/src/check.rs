use ast::{
    Block, Expr, ExprKind, FnRetTy, Item, ItemKind, LitKind, Local, LocalKind, ModKind, Pat,
    PatKind, Span, Stmt, StmtKind,
};
use diagnostics::Related;
use unify::{Term, UnifyError};

use crate::errors::{
    ArgumentCountMismatch, CyclicType, NotCallable, TypeMismatch, UnresolvedValue,
    expected_because_of, expected_due_to, generic_note, provenance,
};
use crate::types::Type;
use crate::{
    Namespace, PatDeclKind, SymbolId, SymbolKind, TermId, TyCon, TypeCheckContext, display_path,
};

impl<'ast> TypeCheckContext<'ast> {
    /// Returns the span of the final expression in the block which
    /// produces the return value of the function, if it exists.
    /// Otherwise, the function returns the empty tuple `()`, and
    /// it falls back to the span of the entire block.
    fn block_value_span(block: &Block) -> Span {
        match block.stmts.last() {
            Some(Stmt {
                kind: StmtKind::Expr(expr),
                ..
            }) => expr.span,
            _ => block.span,
        }
    }

    /// Returns a handle to a Term representing the type of an Expr
    /// node from the AST, with the added constraint that the term
    /// *must* be equal to an expected Term, if one is provided.
    ///
    /// This method is simply a wrapper of
    /// [`Self::check_expr_expecting`]. See for more info.
    pub(crate) fn check_expr(&mut self, expr: &Expr, expected: Option<TermId>) -> TermId {
        self.check_expr_expecting(expr, expected, None)
    }

    /// Returns the name of a generic parameter, if such a generic
    /// parameter exists.
    fn generic_name_of(&mut self, term: TermId) -> Option<String> {
        let resolved = self.uni_cx.resolve(term);
        match self.uni_cx.term(resolved)? {
            Term::App {
                constructor: TyCon::Generic(id),
                ..
            } => self.generic_names.get(id).cloned(),
            _ => None,
        }
    }

    fn first_provenance(
        &mut self,
        candidates: &[(TermId, &'static str, &Type)],
    ) -> Option<Related> {
        for &(term, side, kind) in candidates {
            let Some(Term::Var(v)) = self.uni_cx.term(term).cloned() else {
                continue;
            };
            if let Some(&span) = self.uni_cx.provenance(v) {
                return Some(provenance(span, side, kind));
            }
        }
        None
    }

    /// Typechecks the given expression, and then attempts to unify
    /// it with the expected term if one is given. If unification
    /// fails, then one of two diagnostics is emitted:
    ///
    /// ```text
    ///     1. CyclicType: indicates that the only way to unify the
    ///         expected and actual terms is to create a cyclic
    ///         term of infinite size. For example, unifying
    ///             
    ///             Array(?a) ~ ?a
    ///         
    ///         has a solution, but its size is
    ///         infinite:
    ///         
    ///             Array(Array(...))
    ///
    ///         Which can't exist in practice, hence the error.
    ///
    ///     2. TypeMismatch: indicates that unification failed, and
    ///         there does not exist any set of substitutions that
    ///         would cause the two terms to become equal.
    ///
    ///         One reason is that somewhere in the term, two
    ///         corresponding constructors are not equal to one
    ///         another. For example:
    ///
    ///             ...Array(...)... != ...Fn(...)...
    ///
    ///         Then clearly these terms cannot be made the same
    ///         by making inference variable substitutions, so this
    ///         raises an error.
    ///
    ///         Another reason is that the 'arities' of two
    ///         corresponding constructors are not equal, i.e. they
    ///         have a differing number of constructors. For example:
    ///
    ///             ...Tuple(?a, ?b)... != ...Tuple(?a, ?b, ...)...
    ///             
    ///         They are clearly not equal, hence the error.
    /// ```
    ///
    /// Specifically, this function calls [`Self::check_expr_kind`]
    /// which is what handles the type checking of the expression
    /// itself before we do the final unification which can result
    /// in the above errors.
    ///
    /// Note that [`Self::check_expr_kind`] also may attempt to
    /// perform unifications, and hence may emit its own error.
    fn check_expr_expecting(
        &mut self,
        expr: &Expr,
        expected: Option<TermId>,
        expected_span: Option<Span>,
    ) -> TermId {
        let actual = self.check_expr_kind(&expr.kind, expected);
        // Record locations of primitive types in expressions.
        // Useful for LSP.
        if let ExprKind::Lit(lit) = &expr.kind {
            let name = match lit.kind {
                LitKind::Bool(_) => "bool",
                LitKind::Char(_) => "char",
                LitKind::Int(_) => "int",
                LitKind::Float(_) => "float",
                LitKind::Str(_) => "String",
            };
            self.positions.record_primitive(expr.span, name);
        }
        if let Some(expected) = expected {
            let generic_on_expected = self.generic_name_of(expected);
            let generic_on_actual = if generic_on_expected.is_none() {
                self.generic_name_of(actual)
            } else {
                None
            };
            let reason = expected_span.unwrap_or(expr.span);

            // Attempt to
            if let Err(err) = self.uni_cx.unify_because(actual, expected, reason) {
                let expected_ty = self.resolved(expected);
                let found_ty = self.resolved(actual);
                match err {
                    UnifyError::OccursCheck(_) => {
                        self.diagnostics
                            .push(CyclicType::new(expr.span, expected_ty, found_ty));
                    }
                    UnifyError::ConstructorMismatch { t1, t2, .. }
                    | UnifyError::ArityMismatch { t1, t2, .. } => {
                        let expected_highlight = self.resolved(t2);
                        let found_highlight = self.resolved(t1);

                        let diagnostic_generic_note = generic_on_expected
                            .map(|name| generic_note(name, &found_ty))
                            .or_else(|| {
                                generic_on_actual.map(|name| generic_note(name, &expected_ty))
                            });

                        let expected_provenance = self.first_provenance(&[
                            (t2, "expected", &expected_highlight),
                            (expected, "expected", &expected_ty),
                        ]);
                        let found_provenance = self.first_provenance(&[
                            (t1, "found", &found_highlight),
                            (actual, "found", &found_ty),
                        ]);

                        self.diagnostics.push(TypeMismatch {
                            span: expr.span,
                            expected: expected_ty,
                            found: found_ty,
                            expected_highlight,
                            found_highlight,
                            expected_due_to: expected_span.map(expected_due_to),
                            generic_note: diagnostic_generic_note,
                            expected_provenance,
                            found_provenance,
                        });
                    }
                }
                return self.term(TyCon::Err);
            }
        }
        actual
    }

    fn check_expr_kind(&mut self, kind: &ExprKind, expected: Option<TermId>) -> TermId {
        match kind {
            ExprKind::Err => self.term(TyCon::Err),
            ExprKind::Lit(lit) => match lit.kind {
                LitKind::Bool(_) => self.term(TyCon::Bool),
                LitKind::Char(_) => self.term(TyCon::Char),
                LitKind::Int(_) => self.term(TyCon::Int),
                LitKind::Float(_) => self.term(TyCon::Float),
                LitKind::Str(_) => self.term(TyCon::Str),
            },
            ExprKind::Paren(expr) => self.check_expr(expr, expected),
            ExprKind::If(cond, body, els) => self.check_if_expr(cond, body, els, expected),
            ExprKind::Block(block, _) => self.check_block(block, expected),
            ExprKind::Tup(exprs) => {
                let expected_args = expected.and_then(|expected| {
                    let resolved = self.uni_cx.resolve(expected);
                    match self.uni_cx.term(resolved) {
                        Some(Term::App {
                            constructor: TyCon::Tuple,
                            args,
                        }) if args.len() == exprs.len() => Some(args.clone()),
                        _ => None,
                    }
                });

                let args = exprs
                    .iter()
                    .enumerate()
                    .map(|(i, expr)| {
                        let expected = expected_args.as_ref().map(|args| args[i]);
                        self.check_expr(expr, expected)
                    })
                    .collect();

                self.term_app(TyCon::Tuple, args)
            }
            ExprKind::Ret(expr) => {
                if let Some(expr) = expr {
                    self.check_expr(expr, None);
                }
                self.term(TyCon::Never)
            }
            ExprKind::Path(qself, path) => {
                if qself.is_some() {
                    unimplemented!();
                }

                match self.resolve_path(path, Namespace::Value) {
                    Some(symbol) => {
                        self.record_path_reference(path, symbol);
                        self.check_referenced_fn(symbol);
                        self.instantiate_path(symbol, path)
                    }
                    None => {
                        self.diagnostics
                            .push(UnresolvedValue::new(path.span, display_path(path)));
                        self.term(TyCon::Err)
                    }
                }
            }
            ExprKind::Call(callee, args) => self.check_call_expr(callee, args),
            ExprKind::Cast(_expr, ty) => self.lower_ty(ty),
            ExprKind::Array(exprs) => {
                let expected_ty = expected.and_then(|expected| {
                    let resolved = self.uni_cx.resolve(expected);
                    match self.uni_cx.term(resolved) {
                        Some(Term::App {
                            constructor: TyCon::Array,
                            args,
                        }) if args.len() == 1 => Some(args[0]),
                        _ => None,
                    }
                });

                let mut exprs = exprs.iter();
                let elem_ty = match exprs.next() {
                    Some(first) => {
                        let first_ty = self.check_expr(first, expected_ty);

                        for rest in exprs {
                            self.check_expr(rest, Some(first_ty));
                        }
                        first_ty
                    }
                    None => expected_ty.unwrap_or_else(|| {
                        let var = self.uni_cx.fresh_var();
                        self.term_var(var)
                    }),
                };

                self.term_app(TyCon::Array, vec![elem_ty])
            }
            ExprKind::Assign(lhs, rhs, _) => {
                let lhs = self.check_expr(lhs, None);

                self.check_expr(rhs, Some(lhs));

                self.term(TyCon::Tuple)
            }
            _ => unimplemented!(),
        }
    }

    fn check_if_expr(
        &mut self,
        cond: &Expr,
        body: &Block,
        els: &Option<Box<Expr>>,
        expected: Option<TermId>,
    ) -> TermId {
        let bool_term = self.term(TyCon::Bool);
        self.check_expr(cond, Some(bool_term));

        let body_ty = self.check_block(body, expected);

        let unit_term = self.term(TyCon::Tuple);
        let els_ty = els
            .as_ref()
            .map(|els| self.check_expr(els, expected))
            .unwrap_or(unit_term);

        let body_span = Self::block_value_span(body);
        let els_span = match els.as_deref() {
            Some(Expr {
                kind: ExprKind::Block(block, _),
                ..
            }) => Self::block_value_span(block),
            Some(els) => els.span,
            None => body_span,
        };

        self.check_if_branches_agree(body_ty, els_ty, body_span, els_span, els.is_some());

        self.prefer_non_never(body_ty, els_ty)
    }

    fn check_if_branches_agree(
        &mut self,
        body_ty: TermId,
        els_ty: TermId,
        body_span: Span,
        els_span: Span,
        has_els: bool,
    ) {
        let Err(err) = self.uni_cx.unify_because(body_ty, els_ty, body_span) else {
            return;
        };

        let body_type = self.resolved(body_ty);
        let els_type = self.resolved(els_ty);
        match err {
            UnifyError::OccursCheck(_) => {
                self.diagnostics
                    .push(CyclicType::new(els_span, body_type, els_type));
            }
            UnifyError::ConstructorMismatch { t1, t2, .. }
            | UnifyError::ArityMismatch { t1, t2, .. } => {
                let expected_highlight = self.resolved(t1);
                let found_highlight = self.resolved(t2);

                let expected_provenance = self.first_provenance(&[
                    (t1, "expected", &expected_highlight),
                    (body_ty, "expected", &body_type),
                ]);
                let found_provenance = self.first_provenance(&[
                    (t2, "found", &found_highlight),
                    (els_ty, "found", &els_type),
                ]);

                self.diagnostics.push(TypeMismatch {
                    span: els_span,
                    expected: body_type,
                    found: els_type,
                    expected_highlight,
                    found_highlight,
                    expected_due_to: has_els.then(|| expected_because_of(body_span)),
                    generic_note: None,
                    expected_provenance,
                    found_provenance,
                });
            }
        }
    }

    fn resolved_app(&mut self, term: TermId, con: TyCon) -> Option<Vec<TermId>> {
        let resolved = self.uni_cx.resolve(term);
        match self.uni_cx.term(resolved) {
            Some(Term::App { constructor, args }) if *constructor == con => Some(args.clone()),
            _ => None,
        }
    }

    fn check_call_expr(&mut self, callee: &Expr, args: &[Box<Expr>]) -> TermId {
        let callee_ty = self.check_expr(callee, None);

        let callee_param_spans: Vec<Option<Span>> = match &callee.kind {
            ExprKind::Path(None, path) => self
                .resolve_path(path, Namespace::Value)
                .map(|symbol| match &self.symbols[symbol].kind {
                    SymbolKind::Fn(fn_data) => fn_data.param_spans.clone(),
                    _ => Vec::new(),
                })
                .unwrap_or_default(),
            _ => Vec::new(),
        };

        let resolved_callee = self.uni_cx.resolve(callee_ty);
        let known_inputs = self.resolved_app(callee_ty, TyCon::Fn).and_then(|fn_args| {
            let output_term = fn_args[1];
            self.resolved_app(fn_args[0], TyCon::Tuple)
                .map(|input_tys| (input_tys, output_term))
        });

        if let Some((input_tys, output_term)) = known_inputs {
            let expected = input_tys.len();
            let actual = args.len();
            if expected != actual {
                let span = if actual < expected {
                    let end = args
                        .last()
                        .map(|arg| arg.span.end)
                        .unwrap_or(callee.span.end);
                    Span {
                        start: callee.span.start,
                        end,
                    }
                } else {
                    Span {
                        start: args[expected].span.start,
                        end: args
                            .last()
                            .expect("actual > expected implies at least one arg")
                            .span
                            .end,
                    }
                };
                self.diagnostics.push(ArgumentCountMismatch {
                    span,
                    expected,
                    found: actual,
                });
            }

            for (i, arg) in args.iter().enumerate() {
                let expected_ty = input_tys.get(i).copied();
                let expected_span = callee_param_spans.get(i).copied().flatten();
                self.check_expr_expecting(arg, expected_ty, expected_span);
            }

            output_term
        } else if matches!(self.uni_cx.term(resolved_callee), None | Some(Term::Var(_))) {
            let arg_tys = args.iter().map(|arg| self.check_expr(arg, None)).collect();
            let inputs_term = self.term_app(TyCon::Tuple, arg_tys);
            let ret_var = self.uni_cx.fresh_var();
            let ret_term = self.term_var(ret_var);
            let fn_term = self.term_app(TyCon::Fn, vec![inputs_term, ret_term]);
            if let Err(UnifyError::OccursCheck(_)) =
                self.uni_cx.unify_because(callee_ty, fn_term, callee.span)
            {
                let expected_ty = self.resolved(fn_term);
                let found_ty = self.resolved(callee_ty);
                self.diagnostics
                    .push(CyclicType::new(callee.span, expected_ty, found_ty));
            }
            ret_term
        } else {
            let found = self.resolved(callee_ty);
            self.diagnostics.push(NotCallable {
                span: callee.span,
                found,
            });

            for arg in args {
                self.check_expr(arg, None);
            }

            self.term(TyCon::Err)
        }
    }

    pub(crate) fn check_block(&mut self, block: &Block, expected: Option<TermId>) -> TermId {
        self.check_block_expecting(block, expected, None)
    }

    fn check_block_expecting(
        &mut self,
        block: &Block,
        expected: Option<TermId>,
        expected_span: Option<Span>,
    ) -> TermId {
        let mut ty = self.term(TyCon::Tuple);
        let mut diverges = false;
        for (i, stmt) in block.stmts.iter().enumerate() {
            let is_last = i == block.stmts.len() - 1;
            match &stmt.kind {
                StmtKind::Let(local) => self.check_local(local),

                StmtKind::Expr(expr) if is_last => {
                    ty = self.check_expr_expecting(expr, expected, expected_span);
                }

                StmtKind::Expr(expr) | StmtKind::Semi(expr) => {
                    let stmt_ty = self.check_expr(expr, None);
                    if self.is_never(stmt_ty) {
                        diverges = true;
                    }
                }

                StmtKind::Item(_) | StmtKind::Empty => {}
            }
        }
        if diverges {
            self.term(TyCon::Never)
        } else {
            ty
        }
    }

    fn is_never(&mut self, term: TermId) -> bool {
        let resolved = self.uni_cx.resolve(term);
        matches!(
            self.uni_cx.term(resolved),
            Some(Term::App {
                constructor: TyCon::Never,
                ..
            })
        )
    }

    pub(crate) fn check_local(&mut self, local: &Local) {
        let ascribed = local.ty.as_ref().map(|ty| self.lower_ty(ty));
        let ascribed_span = local.ty.as_ref().map(|ty| ty.span);

        let expected = match &local.kind {
            LocalKind::Decl => ascribed,

            LocalKind::Init(init) => {
                let actual = self.check_expr_expecting(init, ascribed, ascribed_span);
                Some(ascribed.unwrap_or(actual))
            }

            LocalKind::InitElse(init, else_block) => {
                let actual = self.check_expr_expecting(init, ascribed, ascribed_span);
                self.check_block(else_block, None);
                Some(ascribed.unwrap_or(actual))
            }
        };

        let expected = expected.unwrap_or_else(|| {
            let var = self.uni_cx.fresh_var();
            self.term_var(var)
        });

        self.check_pat(&local.pat, expected, PatDeclKind::Let);
    }

    fn prefer_non_never(&mut self, a: TermId, b: TermId) -> TermId {
        if self.is_never(a) { b } else { a }
    }

    pub(crate) fn check_pat(
        &mut self,
        pat: &Pat,
        expected: TermId,
        decl_kind: PatDeclKind,
    ) -> TermId {
        let actual = self.check_pat_kind(&pat.kind, expected, decl_kind);
        let _ = self.uni_cx.unify_because(actual, expected, pat.span);
        actual
    }

    fn check_pat_kind(
        &mut self,
        kind: &PatKind,
        expected: TermId,
        decl_kind: PatDeclKind,
    ) -> TermId {
        match kind {
            PatKind::Wild => expected,
            PatKind::Ident(ident, sub) => {
                let symbol = self.declare(&ident.name, ident.span, decl_kind.symbol_kind());
                let _ = self
                    .uni_cx
                    .unify_because(self.symbols[symbol].ty, expected, ident.span);

                if let Some(sub) = sub {
                    self.check_pat(sub, expected, decl_kind);
                }

                expected
            }
            PatKind::Tuple(pats) => {
                let resolved = self.uni_cx.resolve(expected);

                let expected_args = match self.uni_cx.term(resolved) {
                    Some(Term::App {
                        constructor: TyCon::Tuple,
                        args,
                    }) if args.len() == pats.len() => Some(args.clone()),
                    _ => None,
                };

                let args =
                    pats.iter()
                        .enumerate()
                        .map(|(i, pat)| {
                            let expected = expected_args
                                .as_ref()
                                .map(|args| args[i])
                                .unwrap_or_else(|| {
                                    let var = self.uni_cx.fresh_var();
                                    self.term_var(var)
                                });
                            self.check_pat(pat, expected, decl_kind)
                        })
                        .collect();

                self.term_app(TyCon::Tuple, args)
            }
            _ => unimplemented!(),
        }
    }
}

impl<'ast> TypeCheckContext<'ast> {
    fn resolve_fn<'b>(&mut self, item: &'b Item) -> Option<(SymbolId, &'b Item)> {
        if let ItemKind::Fn(f) = &item.kind {
            let name = self.names.id(&f.ident.name);
            if let Some(symbol) = self.lookup_in_scope(self.current_scope, name, Namespace::Value) {
                Some((symbol, item))
            } else {
                None
            }
        } else {
            None
        }
    }

    pub(crate) fn check_function(&mut self, item: &'ast Item) {
        if let Some((symbol, item)) = self.resolve_fn(item) {
            if let Some(scc) = self.check_fn_body(symbol, item) {
                self.generalize_group(&scc);
            }
        }
    }

    pub(crate) fn check_module(&mut self, item: &'ast Item) {
        let ItemKind::Mod(ident, kind) = &item.kind else {
            return;
        };
        let name = self.names.id(&ident.name);
        let Some(symbol) = self.lookup_in_scope(self.current_scope, name, Namespace::Type) else {
            return;
        };
        let SymbolKind::Mod(scope) = self.symbols[symbol].kind else {
            return;
        };

        match kind {
            ModKind::Loaded(items) => {
                self.with_scope(scope, |this| {
                    this.check_items(items.iter().map(Box::as_ref))
                });
            }
            ModKind::Unloaded => unimplemented!(),
        }
    }

    fn record_edge(
        &mut self,
        from: SymbolId,
        to: SymbolId,
        check: impl FnOnce(&mut Self) -> Option<Vec<SymbolId>>,
    ) -> Option<Vec<SymbolId>> {
        if !self.sccc.is_visited(to) {
            let completed = check(self);
            if self.sccc.is_visited(to) {
                self.sccc.pull_lowlink(from, to);
            }
            completed
        } else {
            self.sccc.note_back_edge(from, to);
            None
        }
    }

    fn check_nested_functions(
        &mut self,
        items: impl IntoIterator<Item = &'ast Item>,
        from: SymbolId,
    ) {
        for item in items {
            let Some((symbol, item)) = self.resolve_fn(item) else {
                continue;
            };
            let scc = self.record_edge(from, symbol, |this| this.check_fn_body(symbol, item));
            if let Some(scc) = scc {
                self.generalize_group(&scc);
            }
        }
    }

    fn check_referenced_fn(&mut self, symbol: SymbolId) {
        let Some(&item) = self.items_by_symbol.get(&symbol) else {
            return;
        };

        let scc = match self.current_fn {
            Some(from) => {
                self.graph.call(from, symbol);
                self.record_edge(from, symbol, |this| this.check_fn_body(symbol, item))
            }
            None if !self.sccc.is_visited(symbol) => self.check_fn_body(symbol, item),
            None => None,
        };

        if let Some(scc) = scc {
            self.generalize_group(&scc);
        }
    }

    fn check_fn_body(&mut self, symbol: SymbolId, item: &'ast Item) -> Option<Vec<SymbolId>> {
        if self.sccc.is_visited(symbol) {
            return None;
        }

        let ItemKind::Fn(f) = &item.kind else {
            return None;
        };
        let scope = match &self.symbols[symbol].kind {
            SymbolKind::Fn(fn_data) => fn_data.scope,
            _ => return None,
        };
        let body = f.body.as_ref()?;

        let symbol_ty = self.symbols[symbol].ty;
        let resolved = self.uni_cx.resolve(symbol_ty);
        let fn_args = match self.uni_cx.term(resolved) {
            Some(Term::App {
                constructor: TyCon::Fn,
                args,
            }) => Some((args[0], args[1])),
            _ => None,
        };
        let (inputs_term, output_term) = fn_args?;
        let resolved_inputs = self.uni_cx.resolve(inputs_term);
        let input_tys = match self.uni_cx.term(resolved_inputs) {
            Some(Term::App {
                constructor: TyCon::Tuple,
                args,
            }) => args.clone(),
            _ => return None,
        };

        self.sccc.enter(symbol);
        let parent_fn = self.current_fn;
        self.current_fn = Some(symbol);
        self.checking_stack.push(symbol);

        self.with_scope(scope, |this| {
            for (param, input_ty) in f.sig.inputs.iter().zip(&input_tys) {
                this.check_pat(&param.pat, *input_ty, PatDeclKind::Param);
            }

            let output_span = match &f.sig.output {
                FnRetTy::Default(span) => *span,
                FnRetTy::Ty(ty) => ty.span,
            };
            this.check_block_expecting(body, Some(output_term), Some(output_span));

            this.check_nested_functions(nested_items(body), symbol);
        });

        self.checking_stack.pop();
        self.current_fn = parent_fn;
        self.sccc.exit(symbol)
    }
}

fn nested_items(body: &Block) -> impl Iterator<Item = &Item> {
    body.stmts.iter().filter_map(|stmt| match &stmt.kind {
        StmtKind::Item(item) => Some(item.as_ref()),
        _ => None,
    })
}

pub(crate) fn collect_fn_mod_items<'ast>(
    cx: &mut TypeCheckContext<'ast>,
    items: impl IntoIterator<Item = &'ast Item>,
) {
    for item in items {
        match &item.kind {
            ItemKind::Fn(f) => {
                let name = cx.names.id(&f.ident.name);
                let Some(symbol) = cx.lookup_in_scope(cx.current_scope, name, Namespace::Value)
                else {
                    continue;
                };
                cx.items_by_symbol.insert(symbol, item);

                let Some(body) = f.body.as_ref() else {
                    continue;
                };
                let scope = match &cx.symbols[symbol].kind {
                    SymbolKind::Fn(fn_data) => fn_data.scope,
                    _ => continue,
                };

                cx.with_scope(scope, |cx| collect_fn_mod_items(cx, nested_items(body)));
            }
            ItemKind::Mod(ident, kind) => {
                let name = cx.names.id(&ident.name);
                let Some(symbol) = cx.lookup_in_scope(cx.current_scope, name, Namespace::Type)
                else {
                    continue;
                };
                cx.items_by_symbol.insert(symbol, item);
                let scope = match cx.symbols[symbol].kind {
                    SymbolKind::Mod(scope) => scope,
                    _ => continue,
                };

                match kind {
                    ModKind::Loaded(items) => {
                        cx.with_scope(scope, |cx| {
                            collect_fn_mod_items(cx, items.iter().map(Box::as_ref))
                        });
                    }
                    ModKind::Unloaded => unimplemented!(),
                }
            }
            _ => {}
        }
    }
}
