use std::collections::{HashMap, HashSet};

use ast::visit::Visitor;
use ast::{
    Block, Expr, ExprKind, FnRetTy, Item, ItemKind, LitKind, Local, LocalKind, Pat, PatKind, Span,
    Stmt, StmtKind,
};
use unify::{Term, UnifyError, term};

use crate::call_graph::{CallGraphCollector, collect_pat_names, strongly_connected_components};
use crate::{
    Diagnostic, Namespace, PatDeclKind, ScopeId, SymbolId, SymbolKind, TermId, TyCon,
    TypeCheckContext,
};

impl TypeCheckContext {
    fn block_value_span(block: &Block) -> Span {
        match block.stmts.last() {
            Some(Stmt {
                kind: StmtKind::Expr(expr),
                ..
            }) => expr.span,
            _ => block.span,
        }
    }

    pub(crate) fn check_expr(&mut self, expr: &Expr, expected: Option<TermId>) -> TermId {
        self.check_expr_expecting(expr, expected, None)
    }

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

    fn with_first_provenance_note(
        &mut self,
        diagnostic: Diagnostic,
        candidates: &[(TermId, String)],
    ) -> Diagnostic {
        for (term, label) in candidates {
            let Some(Term::Var(v)) = self.uni_cx.term(*term).cloned() else {
                continue;
            };
            if let Some(&span) = self.uni_cx.provenance(v) {
                return diagnostic.with_related(span, format!("{label} was inferred here"));
            }
        }
        diagnostic
    }

    fn check_expr_expecting(
        &mut self,
        expr: &Expr,
        expected: Option<TermId>,
        expected_span: Option<Span>,
    ) -> TermId {
        let actual = self.check_expr_kind(&expr.kind, expected);
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
            if let Err(err) = self.uni_cx.unify_because(actual, expected, reason) {
                let expected_rendered = self.renderer().render_term(expected);
                let actual_rendered = self.renderer().render_term(actual);
                match err {
                    UnifyError::OccursCheck(_) => {
                        self.diagnostics.push(Diagnostic::cyclic_type(
                            expr.span,
                            &expected_rendered,
                            &actual_rendered,
                        ));
                    }
                    UnifyError::ConstructorMismatch { t1, t2, .. }
                    | UnifyError::ArityMismatch { t1, t2, .. } => {
                        let lead = "expected `";
                        let mid = "`, found `";
                        let tail = "`";
                        let (expected_highlighted, expected_range) =
                            self.renderer().render_term_highlighting(expected, t2);
                        let (actual_highlighted, actual_range) =
                            self.renderer().render_term_highlighting(actual, t1);
                        let mut diagnostic = Diagnostic::error(
                            expr.span,
                            format!("{lead}{expected_highlighted}{mid}{actual_highlighted}{tail}"),
                        );
                        if let Some(range) = expected_range {
                            let offset = lead.len();
                            diagnostic =
                                diagnostic.with_emphasis(offset + range.start..offset + range.end);
                        }
                        if let Some(range) = actual_range {
                            let offset = lead.len() + expected_highlighted.len() + mid.len();
                            diagnostic =
                                diagnostic.with_emphasis(offset + range.start..offset + range.end);
                        }
                        if let Some(name) = generic_on_expected {
                            diagnostic = diagnostic.with_note(format!(
                                "`{name}` is generic here and must work for every type, not just `{actual_rendered}`"
                            ));
                        } else if let Some(name) = generic_on_actual {
                            diagnostic = diagnostic.with_note(format!(
                                "`{name}` is generic here and must work for every type, not just `{expected_rendered}`"
                            ));
                        }
                        let diagnostic = match expected_span {
                            Some(span) => diagnostic.with_related(span, "expected due to this"),
                            None => diagnostic,
                        };
                        let t2_rendered = self.renderer().render_term(t2);
                        let t1_rendered = self.renderer().render_term(t1);
                        let diagnostic = self.with_first_provenance_note(
                            diagnostic,
                            &[
                                (t2, format!("expected `{t2_rendered}`")),
                                (expected, format!("expected `{expected_rendered}`")),
                            ],
                        );
                        let diagnostic = self.with_first_provenance_note(
                            diagnostic,
                            &[
                                (t1, format!("found `{t1_rendered}`")),
                                (actual, format!("found `{actual_rendered}`")),
                            ],
                        );
                        self.diagnostics.push(diagnostic);
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
                    let body_rendered = self.renderer().render_term(body_ty);
                    let els_rendered = self.renderer().render_term(els_ty);
                    match err {
                        UnifyError::OccursCheck(_) => {
                            self.diagnostics.push(Diagnostic::cyclic_type(
                                els_span,
                                &body_rendered,
                                &els_rendered,
                            ));
                        }
                        UnifyError::ConstructorMismatch { t1, t2, .. }
                        | UnifyError::ArityMismatch { t1, t2, .. } => {
                            let lead = "expected `";
                            let mid = "`, found `";
                            let tail = "`";
                            let (body_highlighted, body_range) =
                                self.renderer().render_term_highlighting(body_ty, t1);
                            let (els_highlighted, els_range) =
                                self.renderer().render_term_highlighting(els_ty, t2);
                            let mut diagnostic = Diagnostic::error(
                                els_span,
                                format!("{lead}{body_highlighted}{mid}{els_highlighted}{tail}"),
                            );
                            if let Some(range) = body_range {
                                let offset = lead.len();
                                diagnostic = diagnostic
                                    .with_emphasis(offset + range.start..offset + range.end);
                            }
                            if let Some(range) = els_range {
                                let offset = lead.len() + body_highlighted.len() + mid.len();
                                diagnostic = diagnostic
                                    .with_emphasis(offset + range.start..offset + range.end);
                            }
                            let diagnostic = if els.is_some() {
                                diagnostic.with_related(body_span, "expected because of this")
                            } else {
                                diagnostic
                            };
                            let t1_rendered = self.renderer().render_term(t1);
                            let t2_rendered = self.renderer().render_term(t2);
                            let diagnostic = self.with_first_provenance_note(
                                diagnostic,
                                &[
                                    (t1, format!("expected `{t1_rendered}`")),
                                    (body_ty, format!("expected `{body_rendered}`")),
                                ],
                            );
                            let diagnostic = self.with_first_provenance_note(
                                diagnostic,
                                &[
                                    (t2, format!("found `{t2_rendered}`")),
                                    (els_ty, format!("found `{els_rendered}`")),
                                ],
                            );
                            self.diagnostics.push(diagnostic);
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
                        let message = format!(
                            "this function takes {expected} argument{} but {actual} argument{} {} supplied",
                            if expected == 1 { "" } else { "s" },
                            if actual == 1 { "" } else { "s" },
                            if actual == 1 { "was" } else { "were" },
                        );
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
                        self.diagnostics.push(Diagnostic::error(span, message));
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
                        let expected_rendered = self.renderer().render_term(fn_term);
                        let actual_rendered = self.renderer().render_term(callee_ty);
                        self.diagnostics.push(Diagnostic::cyclic_type(
                            callee.span,
                            &expected_rendered,
                            &actual_rendered,
                        ));
                    }
                    ret_term
                } else {
                    let found = self.renderer().render_term(callee_ty);
                    self.diagnostics.push(Diagnostic::error(
                        callee.span,
                        format!("expected a function, found `{found}`"),
                    ));

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

    pub(crate) fn check_items(&mut self, items: &[&Item]) {
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

        let sibling_names: HashMap<&str, SymbolId> = fns
            .iter()
            .map(|&(symbol, item)| {
                let ItemKind::Fn(f) = &item.kind else {
                    unreachable!("fns only ever holds ItemKind::Fn items")
                };
                (f.ident.name.as_str(), symbol)
            })
            .collect();

        let nodes: Vec<SymbolId> = fns.iter().map(|&(symbol, _)| symbol).collect();
        let mut edges: HashMap<SymbolId, Vec<SymbolId>> = HashMap::new();
        for &(symbol, item) in &fns {
            let ItemKind::Fn(f) = &item.kind else {
                unreachable!("fns only ever holds ItemKind::Fn items")
            };
            if let Some(body) = f.body.as_ref() {
                let mut shadowed = HashSet::new();
                for param in &f.sig.inputs {
                    collect_pat_names(&param.pat, &mut shadowed);
                }
                let mut collector = CallGraphCollector {
                    sibling_names: &sibling_names,
                    shadowed,
                    edges: Vec::new(),
                };
                collector.visit_block(body);
                edges.insert(symbol, collector.edges);
            }
        }

        let items_by_symbol: HashMap<SymbolId, &Item> = fns.into_iter().collect();

        for component in strongly_connected_components(&nodes, &edges) {
            for &symbol in &component {
                if let Some(&item) = items_by_symbol.get(&symbol) {
                    self.check_fn_body(symbol, item);
                }
            }
            self.cx.generalize_group(&component);
        }
    }

    fn check_fn_body(&mut self, symbol: SymbolId, item: &Item) {
        let ItemKind::Fn(f) = &item.kind else {
            return;
        };
        let scope = match &self.cx.symbols[symbol].kind {
            SymbolKind::Fn(fn_data) => fn_data.scope,
            _ => return,
        };
        let Some(body) = f.body.as_ref() else {
            return;
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
        let Some((inputs_term, output_term)) = fn_args else {
            return;
        };

        let resolved_inputs = self.cx.uni_cx.resolve(inputs_term);
        let input_tys = match self.cx.uni_cx.term(resolved_inputs) {
            Some(Term::App {
                constructor: TyCon::Tuple,
                args,
            }) => args.clone(),
            _ => return,
        };

        self.cx.checking_stack.push(symbol);

        self.with_scope(scope, |this| {
            for (param, input_ty) in f.sig.inputs.iter().zip(&input_tys) {
                this.cx.check_pat(&param.pat, *input_ty, PatDeclKind::Param);
            }

            let output_span = match &f.sig.output {
                FnRetTy::Default(span) => *span,
                FnRetTy::Ty(ty) => ty.span,
            };
            this.cx
                .check_block_expecting(body, Some(output_term), Some(output_span));

            let nested: Vec<&Item> = body
                .stmts
                .iter()
                .filter_map(|stmt| match &stmt.kind {
                    StmtKind::Item(nested) => Some(nested.as_ref()),
                    _ => None,
                })
                .collect();
            this.check_items(&nested);
        });

        self.cx.checking_stack.pop();
    }
}
