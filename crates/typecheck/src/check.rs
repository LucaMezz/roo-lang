use std::collections::HashMap;

use ast::{
    Block, Expr, ExprKind, FnRetTy, Item, ItemKind, LitKind, Local, LocalKind, Pat, PatKind, Span,
    Stmt, StmtKind, visit::Visitor,
};
use diagnostics::Related;
use unify::{Term, UnifyError, term};

use crate::call_graph::{CallGraph, CallGraphCollector, strongly_connected_components};
use crate::errors::{
    ArgumentCountMismatch, CyclicType, NotCallable, TypeMismatch, expected_because_of,
    expected_due_to, generic_note, provenance,
};
use crate::types::Type;
use crate::{
    Namespace, PatDeclKind, ScopeId, SymbolId, SymbolKind, TermId, TyCon, TypeCheckContext,
};

impl TypeCheckContext {
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
                return term!(self.uni_cx, TyCon::Err);
            }
        }
        actual
    }

    fn check_expr_kind(&mut self, kind: &ExprKind, expected: Option<TermId>) -> TermId {
        match kind {
            ExprKind::Err => term!(self.uni_cx, TyCon::Err),
            ExprKind::Lit(lit) => match lit.kind {
                LitKind::Bool(_) => term!(self.uni_cx, TyCon::Bool),
                LitKind::Char(_) => term!(self.uni_cx, TyCon::Char),
                LitKind::Int(_) => term!(self.uni_cx, TyCon::Int),
                LitKind::Float(_) => term!(self.uni_cx, TyCon::Float),
                LitKind::Str(_) => term!(self.uni_cx, TyCon::Str),
            },
            ExprKind::Paren(expr) => self.check_expr(expr, expected),
            ExprKind::If(cond, body, els) => {
                let bool_term = term!(self.uni_cx, TyCon::Bool);
                self.check_expr(cond, Some(bool_term));

                let body_ty = self.check_block(body, expected);

                let unit_term = term!(self.uni_cx, TyCon::Tuple);
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

                if let Err(err) = self.uni_cx.unify_because(body_ty, els_ty, body_span) {
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
                                expected_due_to: els
                                    .is_some()
                                    .then(|| expected_because_of(body_span)),
                                generic_note: None,
                                expected_provenance,
                                found_provenance,
                            });
                        }
                    }
                }

                self.prefer_non_never(body_ty, els_ty)
            }
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

                term!(self.uni_cx, TyCon::Tuple => args)
            }
            ExprKind::Ret(expr) => {
                if let Some(expr) = expr {
                    self.check_expr(expr, None);
                }
                term!(self.uni_cx, TyCon::Never)
            }
            ExprKind::Path(qself, path) => {
                if qself.is_some() {
                    unimplemented!();
                }

                match self.resolve_path(path, Namespace::Value) {
                    Some(symbol) => {
                        self.record_path_reference(path, symbol);
                        self.instantiate_path(symbol, path)
                    }
                    None => term!(self.uni_cx, TyCon::Err),
                }
            }
            ExprKind::Call(callee, args) => {
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
                let fn_shape = match self.uni_cx.term(resolved_callee) {
                    Some(Term::App {
                        constructor: TyCon::Fn,
                        args: fn_args,
                    }) => Some((fn_args[0], fn_args[1])),
                    _ => None,
                };
                let known_inputs = if let Some((inputs_term, output_term)) = fn_shape {
                    let resolved_inputs = self.uni_cx.resolve(inputs_term);
                    match self.uni_cx.term(resolved_inputs) {
                        Some(Term::App {
                            constructor: TyCon::Tuple,
                            args: input_tys,
                        }) => Some((input_tys.clone(), output_term)),
                        _ => None,
                    }
                } else {
                    None
                };

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
                    let inputs_term = term!(self.uni_cx, TyCon::Tuple => arg_tys);
                    let ret_var = self.uni_cx.fresh_var();
                    let ret_term = term!(self.uni_cx, var ret_var);
                    let fn_term = term!(self.uni_cx, TyCon::Fn => [inputs_term, ret_term]);
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

                    term!(self.uni_cx, TyCon::Err)
                }
            }
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
                        term!(self.uni_cx, var var)
                    }),
                };

                term!(self.uni_cx, TyCon::Array => [elem_ty])
            }
            ExprKind::Assign(lhs, rhs, _) => {
                let lhs = self.check_expr(lhs, None);

                self.check_expr(rhs, Some(lhs));

                term!(self.uni_cx, TyCon::Tuple)
            }
            _ => unimplemented!(),
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
        let mut ty = term!(self.uni_cx, TyCon::Tuple);
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
            term!(self.uni_cx, TyCon::Never)
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
            term!(self.uni_cx, var var)
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
                                    term!(self.uni_cx, var var)
                                });
                            self.check_pat(pat, expected, decl_kind)
                        })
                        .collect();

                term!(self.uni_cx, TyCon::Tuple => args)
            }
            _ => unimplemented!(),
        }
    }
}

pub(crate) struct Checker<'a> {
    cx: &'a mut TypeCheckContext,
}

impl<'a> Checker<'a> {
    pub(crate) fn new(cx: &'a mut TypeCheckContext) -> Self {
        Self { cx }
    }

    fn with_scope(&mut self, scope: ScopeId, f: impl FnOnce(&mut Self)) {
        let parent = self.cx.current_scope;
        self.cx.current_scope = scope;
        f(self);
        self.cx.current_scope = parent;
    }

    /// Performs type inference and type checking on the given function
    /// items in a certain scope.
    ///
    /// Generalisation also works in the scenario where functions may
    /// be recursive or mutually recursive with other functions.
    /// This is achieved using call-graph SCC analysis, which
    /// performs generalisation on each strongly connected component
    /// of the call graph at the same time. For example, consider
    /// the scenario
    /// ```ignore
    /// fn first(x) {
    ///     second(x)
    /// }
    ///
    /// fn second(x) {
    ///     first(x)
    /// }
    /// ```
    /// In this case, the functions `first` and `second` make up a
    /// single strongly connected component within the call-graph.
    /// Hence, we perform inference across both functions before
    /// making an attempt to generalise them. Any inference variables
    /// `?a` that are still free then cause a new generic parameter
    /// to be introduced. So the signatures would become:
    /// ```ignore
    /// fn first<T>(x: T) -> T
    /// ```
    /// and
    /// ```ignore
    /// fn second<T>(x: T) -> T
    /// ```
    pub(crate) fn check_functions(&mut self, items: &[&Item]) {
        // Checks that all the passed items are indeed functions,
        // since this function only is concerned with functions,
        // and collects their symbols.
        let mut fns: Vec<(SymbolId, &Item)> = Vec::new();
        for &item in items {
            if let ItemKind::Fn(f) = &item.kind {
                let name = self.cx.names.id(&f.ident.name);
                if let Some(symbol) =
                    self.cx
                        .lookup_in_scope(self.cx.current_scope, name, Namespace::Value)
                {
                    fns.push((symbol, item));
                }
            }
        }
        if fns.is_empty() {
            return;
        }

        let mut graph = CallGraph::new();
        for &(symbol, item) in &fns {
            self.declare_fn_params(symbol, item);
            self.collect_fn_calls(symbol, item, &mut graph);
        }

        let items_by_symbol: HashMap<SymbolId, &Item> = fns.into_iter().collect();

        for component in strongly_connected_components(&graph) {
            for &symbol in &component {
                if let Some(&item) = items_by_symbol.get(&symbol) {
                    self.check_fn_body(symbol, item);
                }
            }
            self.cx.generalize_group(&component);
        }
    }

    fn fn_signature_parts(&mut self, symbol: SymbolId) -> Option<(ScopeId, Vec<TermId>, TermId)> {
        let scope = match &self.cx.symbols[symbol].kind {
            SymbolKind::Fn(fn_data) => fn_data.scope,
            _ => return None,
        };

        let symbol_ty = self.cx.symbols[symbol].ty;
        let resolved = self.cx.uni_cx.resolve(symbol_ty);
        let fn_args = match self.cx.uni_cx.term(resolved) {
            Some(Term::App {
                constructor: TyCon::Fn,
                args,
            }) => Some((args[0], args[1])),
            _ => None,
        };
        let (inputs_term, output_term) = fn_args?;

        let resolved_inputs = self.cx.uni_cx.resolve(inputs_term);
        let input_tys = match self.cx.uni_cx.term(resolved_inputs) {
            Some(Term::App {
                constructor: TyCon::Tuple,
                args,
            }) => args.clone(),
            _ => return None,
        };

        Some((scope, input_tys, output_term))
    }

    fn declare_fn_params(&mut self, symbol: SymbolId, item: &Item) {
        let ItemKind::Fn(f) = &item.kind else {
            return;
        };
        if f.body.is_none() {
            return;
        }
        let Some((scope, input_tys, _)) = self.fn_signature_parts(symbol) else {
            return;
        };

        self.with_scope(scope, |this| {
            for (param, input_ty) in f.sig.inputs.iter().zip(&input_tys) {
                this.cx.check_pat(&param.pat, *input_ty, PatDeclKind::Param);
            }
        });
    }

    fn collect_fn_calls(&mut self, symbol: SymbolId, item: &Item, graph: &mut CallGraph) {
        graph.declare(symbol);

        let ItemKind::Fn(f) = &item.kind else {
            return;
        };
        if f.body.is_none() {
            return;
        }
        let scope = match &self.cx.symbols[symbol].kind {
            SymbolKind::Fn(fn_data) => fn_data.scope,
            _ => return,
        };

        self.with_scope(scope, |this| {
            let mut collector = CallGraphCollector::new(symbol, graph, this.cx);
            collector.visit_item(item);
        });
    }

    fn check_fn_body(&mut self, symbol: SymbolId, item: &Item) {
        let ItemKind::Fn(f) = &item.kind else {
            return;
        };
        let Some(body) = f.body.as_ref() else {
            return;
        };
        let Some((scope, _input_tys, output_term)) = self.fn_signature_parts(symbol) else {
            return;
        };

        // Push the current function onto the checking stack. If there
        // are any other functions defined *within* the function body
        // currently being checked, then any inference variable `?a`
        // which is free in both this function and the nested function
        // won't be generalised as part of the nested function.
        // It will wait until this function itself finishes checking and
        // get's generalised.
        self.cx.checking_stack.push(symbol);

        self.with_scope(scope, |this| {
            let nested: Vec<&Item> = body
                .stmts
                .iter()
                .filter_map(|stmt| match &stmt.kind {
                    StmtKind::Item(nested) => Some(nested.as_ref()),
                    _ => None,
                })
                .collect();
            this.check_functions(&nested);

            let output_span = match &f.sig.output {
                FnRetTy::Default(span) => *span,
                FnRetTy::Ty(ty) => ty.span,
            };
            this.cx
                .check_block_expecting(body, Some(output_term), Some(output_span));
        });

        self.cx.checking_stack.pop();
    }
}
