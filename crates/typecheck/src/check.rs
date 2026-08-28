use std::collections::HashMap;

use ast::{
    Block, Closure, Expr, ExprKind, FnRetTy, Ident, Item, ItemKind, Lit, LitKind, Local, LocalKind,
    ModKind, Pat, PatKind, Path, QSelf, Span, Stmt, StmtKind, StructExpr,
};
use diagnostics::Related;
use intern::Symbol;

use crate::errors::{
    ArgumentCountMismatch, CyclicType, InvalidFieldAccess, InvalidTupleIndex, MissingField,
    NotCallable, TupleIndexOutOfBounds, TypeMismatch, UnknownField, UnresolvedType,
    UnresolvedValue, expected_because_of, expected_due_to, generic_note, provenance,
};
use crate::inference::UnifyError;
use crate::types::{TyKind, Type};
use crate::{
    CxExt, DefId, DefKind, GenericId, PatDeclKind, StructDef, TyId, TypeCheckContext, display_path,
};

#[derive(Default)]
struct TypeMismatchExtras {
    expected_due_to: Option<Related>,
    generic_on_expected: Option<String>,
    generic_on_found: Option<String>,
}

impl TypeMismatchExtras {
    fn expected_due_to(mut self, related: Option<Related>) -> Self {
        self.expected_due_to = related;
        self
    }

    fn generic_on_expected(mut self, name: Option<String>) -> Self {
        self.generic_on_expected = name;
        self
    }

    fn generic_on_found(mut self, name: Option<String>) -> Self {
        self.generic_on_found = name;
        self
    }
}

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

    /// Returns a handle to a ty representing the type of an Expr
    /// node from the AST, with the added constraint that the ty
    /// *must* be equal to an expected ty, if one is provided.
    ///
    /// This method is simply a wrapper of
    /// [`Self::check_expr_expecting`]. See for more info.
    pub(crate) fn check_expr(&mut self, expr: &Expr, expected: Option<TyId>) -> TyId {
        self.check_expr_expecting(expr, expected, None)
    }

    /// Returns the name of a generic parameter, if such a generic
    /// parameter exists.
    fn generic_name_of(&mut self, ty: TyId) -> Option<String> {
        let resolved = self.inf.resolve(ty);
        match self.inf.ty(resolved)? {
            TyKind::Generic(id) => self.generic_names.get(id).cloned(),
            _ => None,
        }
    }

    fn first_provenance(&mut self, candidates: &[(TyId, &'static str, &Type)]) -> Option<Related> {
        for &(ty, side, kind) in candidates {
            let Some(TyKind::Var(v)) = self.inf.ty(ty).cloned() else {
                continue;
            };
            if let Some(span) = self.inf.provenance(v) {
                return Some(provenance(span, side, kind));
            }
        }
        None
    }

    fn check_expr_expecting(
        &mut self,
        expr: &Expr,
        expected: Option<TyId>,
        expected_span: Option<Span>,
    ) -> TyId {
        let actual = self.check_expr_kind(&expr.kind, expected);
        // Record locations of primitive types in expressions.
        // Useful for LSP.
        if let ExprKind::Lit(lit) = &expr.kind {
            let name = match lit.kind {
                LitKind::Bool(_) => "bool",
                LitKind::Int(_) => "int",
                LitKind::Float(_) => "float",
                LitKind::Str(_) => "String",
            };
            self.positions.record_primitive(expr.span, name);
        }
        if let Some(expected) = expected {
            // Whether either side is currently a bare generic type
            // parameter is only meaningful *before* the unification
            // attempt below, since a successful partial unification
            // could otherwise bind it to something else first.
            let generic_on_expected = self.generic_name_of(expected);
            let generic_on_found = if generic_on_expected.is_none() {
                self.generic_name_of(actual)
            } else {
                None
            };
            let reason = expected_span.unwrap_or(expr.span);
            let extras = TypeMismatchExtras::default()
                .expected_due_to(expected_span.map(expected_due_to))
                .generic_on_expected(generic_on_expected)
                .generic_on_found(generic_on_found);
            if let Err(err_ty) =
                self.unify_reporting_mismatch(expected, actual, expr.span, reason, extras)
            {
                return err_ty;
            }
        }
        actual
    }

    fn check_expr_kind(&mut self, kind: &ExprKind, expected: Option<TyId>) -> TyId {
        match kind {
            ExprKind::Err => self.ty(TyKind::Err),
            ExprKind::Lit(lit) => self.check_lit_expr(lit),
            ExprKind::Paren(expr) => self.check_expr(expr, expected),
            ExprKind::If(cond, body, els) => self.check_if_expr(cond, body, els, expected),
            ExprKind::Block(block, _) => self.check_block(block, expected),
            ExprKind::Tup(exprs) => self.check_tup_expr(exprs, expected),
            ExprKind::Ret(expr) => self.check_ret_expr(expr),
            ExprKind::Path(qself, path) => self.check_path_expr(qself, path),
            ExprKind::Call(callee, args) => self.check_call_expr(callee, args),
            ExprKind::Cast(_expr, ty) => self.lower_ty(ty),
            ExprKind::Array(exprs) => self.check_array_expr(exprs, expected),
            ExprKind::Assign(lhs, rhs, _) => self.check_assign_expr(lhs, rhs),
            ExprKind::MethodCall(..) => self.check_method_call_expr(),
            ExprKind::Binary(..) => self.check_binary_expr(),
            ExprKind::Unary(..) => self.check_unary_expr(),
            ExprKind::Let(..) => self.check_let_expr(),
            ExprKind::While(..) => self.check_while_expr(),
            ExprKind::ForLoop { .. } => self.check_for_loop_expr(),
            ExprKind::Loop(..) => self.check_loop_expr(),
            ExprKind::Match(..) => self.check_match_expr(),
            ExprKind::Closure(closure) => self.check_closure_expr(closure),
            ExprKind::AssignOp(..) => self.check_assign_op_expr(),
            ExprKind::Field(expr, index) => self.check_field_expr(expr, index),
            ExprKind::Index(..) => self.check_index_expr(),
            ExprKind::Range(..) => self.check_range_expr(),
            ExprKind::Underscore => self.check_underscore_expr(),
            ExprKind::Break(..) => self.check_break_expr(),
            ExprKind::Continue(..) => self.check_continue_expr(),
            ExprKind::Struct(expr) => self.check_struct_expr(expr),
            ExprKind::Try(..) => self.check_try_expr(),
        }
    }

    fn check_method_call_expr(&mut self) -> TyId {
        unimplemented!()
    }

    fn check_binary_expr(&mut self) -> TyId {
        unimplemented!()
    }

    fn check_unary_expr(&mut self) -> TyId {
        unimplemented!()
    }

    fn check_let_expr(&mut self) -> TyId {
        unimplemented!()
    }

    fn check_while_expr(&mut self) -> TyId {
        unimplemented!()
    }

    fn check_for_loop_expr(&mut self) -> TyId {
        unimplemented!()
    }

    fn check_loop_expr(&mut self) -> TyId {
        unimplemented!()
    }

    fn check_match_expr(&mut self) -> TyId {
        unimplemented!()
    }

    fn check_closure_expr(&mut self, _closure: &Closure) -> TyId {
        unimplemented!()
    }

    fn check_assign_op_expr(&mut self) -> TyId {
        unimplemented!()
    }

    fn check_field_expr(&mut self, expr: &Expr, ident: &Ident) -> TyId {
        let expr_ty = self.check_expr(expr, None);
        let resolved_ty = self.inf.resolve(expr_ty);
        let kind = self.inf.ty(resolved_ty).cloned().expect("valid TyId");

        match kind {
            TyKind::Struct(def, generics) => self.struct_field_ty(expr_ty, def, generics, ident),
            TyKind::Tuple(types) => self.tuple_field_ty(expr_ty, types, ident),
            _ => self.invalid_field_access(expr.span, expr_ty),
        }
    }

    fn struct_field_ty(
        &mut self,
        _expr_ty: TyId,
        def: DefId,
        generics: Vec<TyId>,
        ident: &Ident,
    ) -> TyId {
        let (field_ty, struct_generics, struct_name) = match &self.def(def).kind {
            DefKind::Struct(StructDef { variant, .. }) | DefKind::Variant(variant) => (
                variant.field(ident.symbol).map(|field| field.ty),
                variant.generics.clone(),
                variant.name,
            ),
            _ => unreachable!("TyKind::Struct must resolve to a struct or variant def"),
        };

        let Some(field_ty) = field_ty else {
            let name = self.symbols.resolve(ident.symbol).to_owned();
            let struct_name = self.symbols.resolve(struct_name).to_owned();
            self.diagnostics
                .push(UnknownField::new(ident.span, name, struct_name));
            return self.ty(TyKind::Err);
        };

        let mut subst: HashMap<GenericId, TyId> =
            struct_generics.into_iter().zip(generics).collect();
        self.instantiate_ty(field_ty, &mut subst)
    }

    fn tuple_field_ty(&mut self, expr_ty: TyId, types: Vec<TyId>, ident: &Ident) -> TyId {
        let name = self.symbols.resolve(ident.symbol).to_owned();

        let Ok(index) = name.parse::<usize>() else {
            let found = self.resolved(expr_ty);
            self.diagnostics
                .push(InvalidTupleIndex::new(ident.span, name, found));
            return self.ty(TyKind::Err);
        };

        let Some(&field_ty) = types.get(index) else {
            let found = self.resolved(expr_ty);
            self.diagnostics.push(TupleIndexOutOfBounds::new(
                ident.span,
                index,
                types.len(),
                found,
            ));
            return self.ty(TyKind::Err);
        };

        field_ty
    }

    fn invalid_field_access(&mut self, span: Span, expr_ty: TyId) -> TyId {
        let found = self.resolved(expr_ty);
        self.diagnostics.push(InvalidFieldAccess { span, found });
        self.ty(TyKind::Err)
    }

    fn check_index_expr(&mut self) -> TyId {
        unimplemented!()
    }

    fn check_range_expr(&mut self) -> TyId {
        unimplemented!()
    }

    fn check_underscore_expr(&mut self) -> TyId {
        self.fresh_var()
    }

    fn check_break_expr(&mut self) -> TyId {
        unimplemented!()
    }

    fn check_continue_expr(&mut self) -> TyId {
        unimplemented!()
    }

    fn check_struct_expr(&mut self, expr: &StructExpr) -> TyId {
        let resolved = self.resolve_path_to_struct(&expr.path);
        let resolved = match resolved {
            Some(resolved) => Some(resolved),
            None => self.resolve_path_to_variant(&expr.path),
        };
        let Some((def, variant)) = resolved else {
            self.diagnostics.push(UnresolvedType::new(
                expr.path.span,
                display_path(&expr.path, &self.symbols),
            ));
            return self.ty(TyKind::Err);
        };
        let generics = variant.generics.clone();
        let field_names: Vec<Symbol> = variant.fields.iter().map(|f| f.name).collect();
        let field_tys: Vec<TyId> = variant.fields.iter().map(|f| f.ty).collect();
        let parent = variant.parent;

        self.record_path_reference(&expr.path, def);

        let (instantiated, args) =
            self.instantiate_struct_fields(&generics, &expr.path, &field_tys);
        let declared_fields: Vec<(Symbol, TyId)> =
            field_names.into_iter().zip(instantiated).collect();

        let mut provided_fields = Vec::with_capacity(expr.fields.len());
        for field in &expr.fields {
            let expected = declared_fields
                .iter()
                .find(|(name, _)| *name == field.ident.symbol)
                .map(|(_, ty)| *ty);
            self.check_expr(&field.expr, expected);
            match expected {
                Some(_) => provided_fields.push(field.ident.symbol),
                None => self.diagnostics.push(UnknownField::new(
                    field.span,
                    self.symbols.resolve(field.ident.symbol).to_owned(),
                    display_path(&expr.path, &self.symbols),
                )),
            }
        }

        let ty = match parent {
            Some(enum_def) => self.ty(TyKind::Enum(enum_def, args)),
            None => self.ty(TyKind::Struct(def, args)),
        };

        if let Some(rest) = &expr.rest {
            self.check_expr(rest, Some(ty));
        } else {
            for &(name, _) in &declared_fields {
                if !provided_fields.contains(&name) {
                    self.diagnostics.push(MissingField::new(
                        expr.path.span,
                        self.symbols.resolve(name).to_owned(),
                        display_path(&expr.path, &self.symbols),
                    ));
                }
            }
        }

        ty
    }

    fn check_try_expr(&mut self) -> TyId {
        unimplemented!()
    }

    fn check_lit_expr(&mut self, lit: &Lit) -> TyId {
        match lit.kind {
            LitKind::Bool(_) => self.ty(TyKind::Bool),
            LitKind::Int(_) => self.ty(TyKind::Int),
            LitKind::Float(_) => self.ty(TyKind::Float),
            LitKind::Str(_) => self.ty(TyKind::Str),
        }
    }

    fn check_tup_expr(&mut self, exprs: &[Box<Expr>], expected: Option<TyId>) -> TyId {
        let expected_args =
            expected.and_then(|expected| self.resolved_tuple_with_arity(expected, exprs.len()));

        let args = exprs
            .iter()
            .enumerate()
            .map(|(i, expr)| {
                let expected = expected_args.as_ref().map(|args| args[i]);
                self.check_expr(expr, expected)
            })
            .collect();

        self.ty(TyKind::Tuple(args))
    }

    fn check_ret_expr(&mut self, expr: &Option<Box<Expr>>) -> TyId {
        if let Some(expr) = expr {
            self.check_expr(expr, None);
        }
        self.ty(TyKind::Never)
    }

    fn check_path_expr(&mut self, qself: &Option<Box<QSelf>>, path: &Path) -> TyId {
        if qself.is_some() {
            unimplemented!();
        }

        self.resolve_path_to_value(path)
            .map(|def| {
                self.record_path_reference(path, def);
                self.check_referenced_fn(def);
                self.instantiate_path(def, path)
            })
            .unwrap_or_else(|| {
                self.diagnostics.push(UnresolvedValue::new(
                    path.span,
                    display_path(path, &self.symbols),
                ));
                self.ty(TyKind::Err)
            })
    }

    fn check_array_expr(&mut self, exprs: &[Box<Expr>], expected: Option<TyId>) -> TyId {
        let expected_ty = expected.and_then(|expected| self.resolved_array(expected));

        let elem_ty = exprs
            .split_first()
            .map(|(first, rest)| {
                let first_ty = self.check_expr(first, expected_ty);
                rest.iter().for_each(|expr| {
                    self.check_expr(expr, Some(first_ty));
                });
                first_ty
            })
            .unwrap_or_else(|| self.or_fresh_var(expected_ty));

        self.ty(TyKind::Array(elem_ty))
    }

    fn check_assign_expr(&mut self, lhs: &Expr, rhs: &Expr) -> TyId {
        let lhs = self.check_expr(lhs, None);

        self.check_expr(rhs, Some(lhs));

        self.ty(TyKind::Tuple(Vec::new()))
    }

    fn check_if_expr(
        &mut self,
        cond: &Expr,
        body: &Block,
        els: &Option<Box<Expr>>,
        expected: Option<TyId>,
    ) -> TyId {
        let bool_ty = self.ty(TyKind::Bool);
        self.check_expr(cond, Some(bool_ty));

        let body_ty = self.check_block(body, expected);

        let unit_ty = self.ty(TyKind::Tuple(Vec::new()));
        let els_ty = els
            .as_ref()
            .map(|els| self.check_expr(els, expected))
            .unwrap_or(unit_ty);

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
        body_ty: TyId,
        els_ty: TyId,
        body_span: Span,
        els_span: Span,
        has_els: bool,
    ) {
        let extras = TypeMismatchExtras::default()
            .expected_due_to(has_els.then(|| expected_because_of(body_span)));
        let _ = self.unify_reporting_mismatch(body_ty, els_ty, els_span, body_span, extras);
    }

    fn resolved_array(&mut self, ty: TyId) -> Option<TyId> {
        let resolved = self.inf.resolve(ty);
        match self.inf.ty(resolved) {
            Some(&TyKind::Array(elem)) => Some(elem),
            _ => None,
        }
    }

    fn resolved_tuple(&mut self, ty: TyId) -> Option<Vec<TyId>> {
        let resolved = self.inf.resolve(ty);
        match self.inf.ty(resolved) {
            Some(TyKind::Tuple(args)) => Some(args.clone()),
            _ => None,
        }
    }

    fn resolved_tuple_with_arity(&mut self, ty: TyId, arity: usize) -> Option<Vec<TyId>> {
        self.resolved_tuple(ty).filter(|args| args.len() == arity)
    }

    fn resolved_fn_parts(&mut self, ty: TyId) -> Option<(Vec<TyId>, TyId)> {
        let resolved = self.inf.resolve(ty);
        match self.inf.ty(resolved) {
            Some(TyKind::Fn(params, ret)) => Some((params.clone(), *ret)),
            _ => None,
        }
    }

    pub(crate) fn unify_or_report_cycle(&mut self, expected: TyId, found: TyId, span: Span) {
        if let Err(UnifyError::OccursCheck(_)) = self.inf.unify_because(expected, found, span) {
            let expected_ty = self.resolved(expected);
            let found_ty = self.resolved(found);
            self.diagnostics
                .push(CyclicType::new(span, expected_ty, found_ty));
        }
    }

    fn unify_reporting_mismatch(
        &mut self,
        expected: TyId,
        found: TyId,
        span: Span,
        reason: Span,
        extras: TypeMismatchExtras,
    ) -> Result<(), TyId> {
        let Err(err) = self.inf.unify_because(expected, found, reason) else {
            return Ok(());
        };

        let expected_ty = self.resolved(expected);
        let found_ty = self.resolved(found);
        match err {
            UnifyError::OccursCheck(_) => {
                self.diagnostics
                    .push(CyclicType::new(span, expected_ty, found_ty));
            }
            UnifyError::ConstructorMismatch { t1, t2, .. }
            | UnifyError::ArityMismatch { t1, t2, .. } => {
                let expected_highlight = self.resolved(t1);
                let found_highlight = self.resolved(t2);

                let diagnostic_generic_note = extras
                    .generic_on_expected
                    .map(|name| generic_note(name, &found_ty))
                    .or_else(|| {
                        extras
                            .generic_on_found
                            .map(|name| generic_note(name, &expected_ty))
                    });

                let expected_provenance = self.first_provenance(&[
                    (t1, "expected", &expected_highlight),
                    (expected, "expected", &expected_ty),
                ]);
                let found_provenance = self.first_provenance(&[
                    (t2, "found", &found_highlight),
                    (found, "found", &found_ty),
                ]);

                self.diagnostics.push(TypeMismatch {
                    span,
                    expected: expected_ty,
                    found: found_ty,
                    expected_highlight,
                    found_highlight,
                    expected_due_to: extras.expected_due_to,
                    generic_note: diagnostic_generic_note,
                    expected_provenance,
                    found_provenance,
                });
            }
        }
        Err(self.ty(TyKind::Err))
    }

    fn check_call_expr(&mut self, callee: &Expr, args: &[Box<Expr>]) -> TyId {
        let callee_ty = self.check_expr(callee, None);

        let callee_param_spans: Vec<Option<Span>> = match &callee.kind {
            ExprKind::Path(None, path) => self
                .resolve_path_to_value(path)
                .map(|def| match &self.def(def).kind {
                    DefKind::Fn(fn_data) => fn_data.param_spans.clone(),
                    _ => Vec::new(),
                })
                .unwrap_or_default(),
            _ => Vec::new(),
        };

        let resolved_callee = self.inf.resolve(callee_ty);
        if let Some((input_tys, output_ty)) = self.resolved_fn_parts(callee_ty) {
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

            args.iter().enumerate().for_each(|(i, arg)| {
                let expected_ty = input_tys.get(i).copied();
                let expected_span = callee_param_spans.get(i).copied().flatten();
                self.check_expr_expecting(arg, expected_ty, expected_span);
            });

            output_ty
        } else if matches!(self.inf.ty(resolved_callee), None | Some(TyKind::Var(_))) {
            let arg_tys = args.iter().map(|arg| self.check_expr(arg, None)).collect();
            let ret_ty = self.fresh_var();
            let fn_ty = self.ty(TyKind::Fn(arg_tys, ret_ty));
            self.unify_or_report_cycle(fn_ty, callee_ty, callee.span);
            ret_ty
        } else {
            let found = self.resolved(callee_ty);
            self.diagnostics.push(NotCallable {
                span: callee.span,
                found,
            });

            args.iter().for_each(|arg| {
                self.check_expr(arg, None);
            });

            self.ty(TyKind::Err)
        }
    }

    pub(crate) fn check_block(&mut self, block: &Block, expected: Option<TyId>) -> TyId {
        self.check_block_expecting(block, expected, None)
    }

    fn check_block_expecting(
        &mut self,
        block: &Block,
        expected: Option<TyId>,
        expected_span: Option<Span>,
    ) -> TyId {
        let mut ty = self.ty(TyKind::Tuple(Vec::new()));
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
        if diverges { self.ty(TyKind::Never) } else { ty }
    }

    fn is_never(&mut self, ty: TyId) -> bool {
        let resolved = self.inf.resolve(ty);
        matches!(self.inf.ty(resolved), Some(TyKind::Never))
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

        let expected = self.or_fresh_var(expected);

        self.check_pat(&local.pat, expected, PatDeclKind::Let);
    }

    fn prefer_non_never(&mut self, a: TyId, b: TyId) -> TyId {
        if self.is_never(a) { b } else { a }
    }

    pub(crate) fn check_pat(&mut self, pat: &Pat, expected: TyId, decl_kind: PatDeclKind) -> TyId {
        let actual = self.check_pat_kind(&pat.kind, expected, decl_kind);
        let _ = self.inf.unify_because(actual, expected, pat.span);
        actual
    }

    fn check_pat_kind(&mut self, kind: &PatKind, expected: TyId, decl_kind: PatDeclKind) -> TyId {
        match kind {
            PatKind::Wild => expected,
            PatKind::Rest => expected,
            PatKind::Ident(ident, sub) => self.check_ident_pat(ident, sub, expected, decl_kind),
            PatKind::Tuple(pats) => self.check_tuple_pat(pats, expected, decl_kind),
            PatKind::Missing => self.check_missing_pat(),
            PatKind::Struct(..) => self.check_struct_pat(),
            PatKind::TupleStruct(..) => self.check_tuple_struct_pat(),
            PatKind::Or(..) => self.check_or_pat(),
            PatKind::Path(..) => self.check_path_pat(),
            PatKind::Expr(..) => self.check_expr_pat(),
            PatKind::Range(..) => self.check_range_pat(),
            PatKind::Array(..) => self.check_array_pat(),
            PatKind::Never => self.check_never_pat(),
            PatKind::Paren(..) => self.check_paren_pat(),
            PatKind::Err => self.check_err_pat(),
        }
    }

    fn check_ident_pat(
        &mut self,
        ident: &Ident,
        sub: &Option<Box<Pat>>,
        expected: TyId,
        decl_kind: PatDeclKind,
    ) -> TyId {
        let ty = self.fresh_var_at(Some(ident.span));
        self.declare(ident.symbol, ident.span, decl_kind.def_kind(ty));
        let _ = self.inf.unify_because(ty, expected, ident.span);

        if let Some(sub) = sub {
            self.check_pat(sub, expected, decl_kind);
        }

        expected
    }

    fn check_tuple_pat(&mut self, pats: &[Pat], expected: TyId, decl_kind: PatDeclKind) -> TyId {
        let expected_args = self.resolved_tuple_with_arity(expected, pats.len());

        let args = pats
            .iter()
            .enumerate()
            .map(|(i, pat)| {
                let expected = self.or_fresh_var(expected_args.as_ref().map(|args| args[i]));
                self.check_pat(pat, expected, decl_kind)
            })
            .collect();

        self.ty(TyKind::Tuple(args))
    }

    fn check_missing_pat(&mut self) -> TyId {
        unimplemented!()
    }

    fn check_struct_pat(&mut self) -> TyId {
        unimplemented!()
    }

    fn check_tuple_struct_pat(&mut self) -> TyId {
        unimplemented!()
    }

    fn check_or_pat(&mut self) -> TyId {
        unimplemented!()
    }

    fn check_path_pat(&mut self) -> TyId {
        unimplemented!()
    }

    fn check_expr_pat(&mut self) -> TyId {
        unimplemented!()
    }

    fn check_range_pat(&mut self) -> TyId {
        unimplemented!()
    }

    fn check_array_pat(&mut self) -> TyId {
        unimplemented!()
    }

    fn check_never_pat(&mut self) -> TyId {
        unimplemented!()
    }

    fn check_paren_pat(&mut self) -> TyId {
        unimplemented!()
    }

    fn check_err_pat(&mut self) -> TyId {
        unimplemented!()
    }
}

impl<'ast> TypeCheckContext<'ast> {
    fn resolve_fn_def(&mut self, f: &ast::Fn) -> Option<DefId> {
        let name = f.ident.symbol;
        self.with_value_def(name, |_, def| def)
    }

    fn resolve_fn<'b>(&mut self, item: &'b Item) -> Option<(DefId, &'b ast::Fn)> {
        let ItemKind::Fn(f) = &item.kind else {
            return None;
        };
        self.resolve_fn_def(f).map(|def| (def, f.as_ref()))
    }

    pub(crate) fn check_function(&mut self, f: &'ast ast::Fn) {
        self.resolve_fn_def(f)
            .and_then(|def| self.check_fn_body(def, f))
            .into_iter()
            .for_each(|scc| self.generalize_group(&scc));
    }

    pub(crate) fn check_module(&mut self, ident: &Ident, kind: &'ast ModKind) {
        let name = ident.symbol;
        self.with_mod_scope(name, |this, _def| match kind {
            ModKind::Loaded(items) => {
                this.check_items(items.iter().map(Box::as_ref));
            }
            ModKind::Unloaded => unimplemented!(),
        });
    }

    fn record_edge(
        &mut self,
        from: DefId,
        to: DefId,
        check: impl FnOnce(&mut Self) -> Option<Vec<DefId>>,
    ) -> Option<Vec<DefId>> {
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

    fn check_nested_functions(&mut self, items: impl IntoIterator<Item = &'ast Item>, from: DefId) {
        items
            .into_iter()
            .filter_map(|item| self.resolve_fn(item))
            .collect::<Vec<_>>()
            .into_iter()
            .for_each(|(def, f)| {
                let scc = self.record_edge(from, def, |this| this.check_fn_body(def, f));
                if let Some(scc) = scc {
                    self.generalize_group(&scc);
                }
            });
    }

    fn check_referenced_fn(&mut self, def: DefId) {
        let Some(&item) = self.items_by_def.get(&def) else {
            return;
        };
        let ItemKind::Fn(f) = &item.kind else {
            return;
        };

        let scc = match self.current_fn {
            Some(from) => {
                self.graph.call(from, def);
                self.record_edge(from, def, |this| this.check_fn_body(def, f))
            }
            None if !self.sccc.is_visited(def) => self.check_fn_body(def, f),
            None => None,
        };

        scc.into_iter().for_each(|scc| self.generalize_group(&scc));
    }

    fn check_fn_body(&mut self, def: DefId, f: &'ast ast::Fn) -> Option<Vec<DefId>> {
        if self.sccc.is_visited(def) {
            return None;
        }

        let scope = self.fn_def_scope(def)?;
        let body = f.body.as_ref()?;

        let def_ty = self.def(def).ty();
        let (input_tys, output_ty) = self.resolved_fn_parts(def_ty)?;

        self.sccc.enter(def);
        let parent_fn = self.current_fn;
        self.current_fn = Some(def);
        self.checking_stack.push(def);

        self.with_scope(scope, |this| {
            f.sig
                .inputs
                .iter()
                .zip(&input_tys)
                .for_each(|(param, input_ty)| {
                    this.check_pat(&param.pat, *input_ty, PatDeclKind::Param);
                });

            let output_span = match &f.sig.output {
                FnRetTy::Default(span) => *span,
                FnRetTy::Ty(ty) => ty.span,
            };
            this.check_block_expecting(body, Some(output_ty), Some(output_span));

            this.check_nested_functions(nested_items(body), def);
        });

        self.checking_stack.pop();
        self.current_fn = parent_fn;
        self.sccc.exit(def)
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
    items.into_iter().for_each(|item| match &item.kind {
        ItemKind::Fn(f) => {
            let name = f.ident.symbol;
            cx.with_fn_scope(name, |cx, def| {
                cx.items_by_def.insert(def, item);

                let Some(body) = f.body.as_ref() else {
                    return;
                };

                collect_fn_mod_items(cx, nested_items(body));
            });
        }
        ItemKind::Mod(ident, kind) => {
            let name = ident.symbol;
            cx.with_mod_scope(name, |cx, def| {
                cx.items_by_def.insert(def, item);

                match kind {
                    ModKind::Loaded(items) => {
                        collect_fn_mod_items(cx, items.iter().map(Box::as_ref));
                    }
                    ModKind::Unloaded => unimplemented!(),
                }
            });
        }
        _ => {}
    });
}
