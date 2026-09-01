use std::collections::HashMap;

use ast::{
    AssocItemKind, Block, Closure, Expr, ExprKind, FnRetTy, Ident, Impl, Item, ItemKind, Lit,
    LitKind, Local, LocalKind, MethodCall, ModKind, Pat, PatKind, Path, QSelf, SELF_PARAM, Span,
    Stmt, StmtKind, StructExpr, Trait,
};
use diagnostics::Related;
use intern::Symbol;

use crate::defs::{GenericParamDef, StructDef};
use crate::errors::*;

use crate::inference::UnifyError;
use crate::types::{TyKind, Type};
use crate::{
    CxExt, DefId, DefIdOf, FnDef, ImplTarget, Namespace, PatDeclKind, ScopeId, TyId,
    TypeCheckContext, display_path, impl_target_of,
};

#[derive(Default)]
pub(crate) struct TypeMismatchExtras {
    expected_due_to: Option<Related>,
    generic_on_expected: Option<String>,
    generic_on_found: Option<String>,
}

impl TypeMismatchExtras {
    pub(crate) fn expected_due_to(mut self, related: Option<Related>) -> Self {
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
        self.inf
            .ty(resolved)
            .map(|kind| match kind {
                TyKind::Generic(id) => Some(self.defs.generic_param_ref(*id).name),
                _ => None,
            })
            .flatten()
            .map(|name| self.symbols.resolve(name).to_owned())
    }

    pub(crate) fn generic_name(&self, id: DefIdOf<GenericParamDef>) -> String {
        let symbol = self.defs.generic_param_ref(id).name;
        self.symbols.resolve(symbol).to_owned()
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
            if self.coerce(actual, expected) {
                return expected;
            }
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
            ExprKind::Cast(expr, ty) => {
                self.check_expr(expr, None);
                self.lower_ty(ty)
            }
            ExprKind::Array(exprs) => self.check_array_expr(exprs, expected),
            ExprKind::Assign(lhs, rhs, _) => self.check_assign_expr(lhs, rhs),
            ExprKind::MethodCall(call) => self.check_method_call_expr(call),
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

    fn check_method_call_expr(&mut self, call: &MethodCall) -> TyId {
        enum MethodReceiver {
            Target(ImplTarget),
            Generic(DefIdOf<GenericParamDef>),
        }

        let receiver_ty = self.check_expr(call.receiver.as_ref(), None);

        let resolved_receiver = self.inf.resolve(receiver_ty);
        let receiver_kind = self.inf.ty(resolved_receiver).cloned();

        let method_receiver = match &receiver_kind {
            Some(TyKind::Generic(id)) if !self.defs.generic_param_ref(*id).bounds.is_empty() => {
                Some(MethodReceiver::Generic(*id))
            }
            Some(kind) => impl_target_of(kind).map(MethodReceiver::Target),
            None => None,
        };

        let Some(method_receiver) = method_receiver else {
            let found = self.resolved(receiver_ty);
            self.diagnostics
                .push(InvalidMethodReceiver::new(call.receiver.span, found));
            return self.check_method_args_untyped(call);
        };

        let symbol = call.seg.ident.symbol;
        let resolution = match method_receiver {
            MethodReceiver::Target(target) => self
                .resolve_in_impls_for_target(target, symbol, Namespace::Value)
                .map(|def| (def, HashMap::new())),
            MethodReceiver::Generic(id) => {
                self.resolve_method_in_generic_bounds(id, symbol, Namespace::Value)
            }
        };

        let Some((def, mut trait_subst)) = resolution else {
            let found = self.resolved(receiver_ty);
            let name = self.symbols.resolve(symbol).to_owned();
            self.diagnostics
                .push(UnresolvedMethod::new(call.seg.ident.span, name, found));
            return self.check_method_args_untyped(call);
        };
        self.positions.record_def(call.seg.ident.span, def);

        let (fn_def, _) = self
            .fn_def_scope(def)
            .expect("a value found in an impl or trait scope is always a fn def");
        let fn_data = self.defs.fn_ref(fn_def);
        let params = fn_data.params.clone();
        let generics = fn_data.generics.clone();
        let fn_ty = fn_data.ty;

        let has_self = params.first().is_some_and(|p| p.symbol == SELF_PARAM);
        if !has_self {
            let found = self.resolved(receiver_ty);
            let name = self.symbols.resolve(symbol).to_owned();
            self.diagnostics
                .push(NotAMethod::new(call.seg.ident.span, name, found));
            return self.check_method_args_untyped(call);
        }

        let mut subst = self.subst_for_seg(&generics, &call.seg);
        subst.extend(trait_subst.drain());
        let instantiated = self.instantiate_ty(fn_ty, &mut subst);
        let (mut input_tys, output_ty) = self
            .resolved_fn_parts(instantiated)
            .expect("a fn def's ty always resolves to TyKind::Fn");

        let self_ty = input_tys.remove(0);
        let _ = self.unify_reporting_mismatch(
            self_ty,
            receiver_ty,
            call.receiver.span,
            call.receiver.span,
            TypeMismatchExtras::default(),
        );

        let expected = input_tys.len();
        let actual = call.args.len();
        if expected != actual {
            let span = if actual < expected {
                let end = call
                    .args
                    .last()
                    .map(|arg| arg.span.end)
                    .unwrap_or(call.seg.ident.span.end);
                Span {
                    start: call.seg.ident.span.start,
                    end,
                }
            } else {
                Span {
                    start: call.args[expected].span.start,
                    end: call
                        .args
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

        let param_spans = &params[1..];
        call.args.iter().enumerate().for_each(|(i, arg)| {
            let expected_ty = input_tys.get(i).copied();
            let expected_span = param_spans.get(i).and_then(|p| p.span);
            self.check_expr_expecting(arg, expected_ty, expected_span);
        });

        output_ty
    }

    fn check_method_args_untyped(&mut self, call: &MethodCall) -> TyId {
        call.args.iter().for_each(|arg| {
            self.check_expr(arg, None);
        });
        self.ty(TyKind::Err)
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
        def: DefIdOf<StructDef>,
        generics: Vec<TyId>,
        ident: &Ident,
    ) -> TyId {
        let struct_ref = self.defs.struct_ref(def);
        let variant = &struct_ref.variant;
        let field_ty = variant.field(ident.symbol).map(|field| field.ty);
        let struct_name = variant.name;

        let Some(field_ty) = field_ty else {
            let name = self.symbols.resolve(ident.symbol).to_owned();
            let struct_name = self.symbols.resolve(struct_name).to_owned();
            self.diagnostics
                .push(UnknownField::new(ident.span, name, struct_name));
            return self.ty(TyKind::Err);
        };

        let mut subst: HashMap<DefIdOf<GenericParamDef>, TyId> =
            variant.generics.iter().copied().zip(generics).collect();
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
        let resolved = if let Some((id, def)) = self.resolve_path_to_struct_variant(&expr.path) {
            Some((id.id(), def))
        } else {
            self.resolve_path_to_variant(&expr.path)
                .map(|(id, def)| (id.id(), def))
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
            None => self.ty(TyKind::Struct(DefIdOf::new_unchecked(def), args)),
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
        let expected = self.current_return_ty();
        if let Some(expr) = expr {
            self.check_expr(expr, expected);
        }
        self.ty(TyKind::Never)
    }

    fn check_path_expr(&mut self, qself: &Option<Box<QSelf>>, path: &Path) -> TyId {
        let Some(qself) = qself else {
            return self
                .resolve_path_to_value(path)
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
                });
        };

        let Some(first) = path.segments.first() else {
            self.diagnostics.push(UnresolvedValue::new(
                path.span,
                display_path(path, &self.symbols),
            ));
            return self.ty(TyKind::Err);
        };
        let symbol = first.ident.symbol;

        let Some((def, mut trait_subst)) = self.resolve_qself_item(qself, symbol, Namespace::Value)
        else {
            self.diagnostics.push(UnresolvedValue::new(
                path.span,
                display_path(path, &self.symbols),
            ));
            return self.ty(TyKind::Err);
        };

        self.record_path_reference(path, def);
        self.check_referenced_fn(def);

        let ty = self.def(def).ty();
        let generics = self.def(def).generics().to_vec();
        let mut subst = self.subst_for_seg(&generics, first);
        subst.extend(trait_subst.drain());
        self.instantiate_ty(ty, &mut subst)
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

    fn coerce(&mut self, actual: TyId, expected: TyId) -> bool {
        let resolved_expected = self.inf.resolve(expected);
        let Some(TyKind::TraitObject(trait_def, trait_args)) = self.inf.ty(resolved_expected)
        else {
            return false;
        };
        let trait_args = trait_args.clone();

        self.target_implements(actual, *trait_def, &trait_args)
    }

    #[allow(unused)]
    fn unify_or_coerce(&mut self, actual: TyId, expected: TyId) -> Result<(), UnifyError> {
        if self.coerce(actual, expected) {
            return Ok(());
        }
        self.inf.unify(actual, expected)
    }

    pub(crate) fn unify_reporting_mismatch(
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
                .and_then(|def| self.def(def).as_fn())
                .map(|fn_data| fn_data.params.iter().map(|p| p.span).collect())
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
    fn resolve_fn_def(&mut self, f: &ast::Fn) -> Option<(DefIdOf<FnDef>, ScopeId)> {
        let name = f.ident.symbol;
        self.with_fn_def(name, |_, def, scope| (def, scope))
    }

    fn resolve_fn_def_at(&self, f: &ast::Fn) -> Option<(DefIdOf<FnDef>, ScopeId)> {
        let def = self.positions.def_at_span(f.ident.span)?;
        self.fn_def_scope(def)
    }

    fn resolve_fn<'b>(&mut self, item: &'b Item) -> Option<(DefIdOf<FnDef>, ScopeId, &'b ast::Fn)> {
        let ItemKind::Fn(f) = &item.kind else {
            return None;
        };
        self.resolve_fn_def(f)
            .map(|(def, scope)| (def, scope, f.as_ref()))
    }

    pub(crate) fn check_function(&mut self, f: &'ast ast::Fn) {
        let scc = self
            .resolve_fn_def(f)
            .and_then(|(def, scope)| self.check_fn_body(def, scope, f));
        self.finish_scc(scc);
    }

    fn finish_scc(&mut self, scc: Option<Vec<DefId>>) {
        let Some(scc) = scc else {
            return;
        };
        let scc: Vec<DefIdOf<FnDef>> = scc
            .into_iter()
            .map(|def| {
                self.fn_def_scope(def)
                    .map(|(def, _)| def)
                    .expect("SCC members are always function defs")
            })
            .collect();
        self.generalize_group(&scc);
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

    pub(crate) fn check_trait(&mut self, t: &'ast Trait) {
        self.with_trait_scope(t.ident.symbol, |this, _def| {
            t.items.iter().for_each(|item| match &item.kind {
                AssocItemKind::Fn(f) => this.check_function(f),
                AssocItemKind::Type(_) => {}
            });
        });
    }

    pub(crate) fn check_impl(&mut self, imp: &'ast Impl) {
        imp.items.iter().for_each(|item| match &item.kind {
            AssocItemKind::Fn(f) => {
                let scc = self
                    .resolve_fn_def_at(f)
                    .and_then(|(def, scope)| self.check_fn_body(def, scope, f));
                self.finish_scc(scc);
            }
            AssocItemKind::Type(_) => {}
        });
    }

    fn record_edge(
        &mut self,
        from: DefId,
        to: DefId,
        check: impl FnOnce(&mut Self) -> Option<Vec<DefId>>,
    ) -> Option<Vec<DefId>> {
        if !self.recursion.is_visited(to) {
            let completed = check(self);
            if self.recursion.is_visited(to) {
                self.recursion.pull_lowlink(from, to);
            }
            completed
        } else {
            self.recursion.note_back_edge(from, to);
            None
        }
    }

    fn check_nested_functions(
        &mut self,
        items: impl IntoIterator<Item = &'ast Item>,
        from: DefIdOf<FnDef>,
    ) {
        items
            .into_iter()
            .filter_map(|item| self.resolve_fn(item))
            .collect::<Vec<_>>()
            .into_iter()
            .for_each(|(def, scope, f)| {
                let scc = self.record_edge(from.id(), def.id(), |this| {
                    this.check_fn_body(def, scope, f)
                });
                self.finish_scc(scc);
            });
    }

    fn check_referenced_fn(&mut self, def: DefId) {
        let Some(&item) = self.items_by_def.get(&def) else {
            return;
        };
        let ItemKind::Fn(f) = &item.kind else {
            return;
        };
        let Some((def, scope)) = self.fn_def_scope(def) else {
            return;
        };

        let scc = match self.recursion.current() {
            Some(from) => {
                self.recursion.record_call(from, def.id());
                self.record_edge(from, def.id(), |this| this.check_fn_body(def, scope, f))
            }
            None if !self.recursion.is_visited(def.id()) => self.check_fn_body(def, scope, f),
            None => None,
        };

        self.finish_scc(scc);
    }

    fn check_fn_body(
        &mut self,
        def: DefIdOf<FnDef>,
        scope: ScopeId,
        f: &'ast ast::Fn,
    ) -> Option<Vec<DefId>> {
        if self.recursion.is_visited(def.id()) {
            return None;
        }

        let body = f.body.as_ref()?;

        let def_ty = self.defs.fn_ref(def).ty;
        let (input_tys, output_ty) = self.resolved_fn_parts(def_ty)?;

        let (_, scc) = self.checking(def.id(), |this| {
            this.with_scope(scope, |this| {
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
                this.with_return_ty(output_ty, |this| {
                    this.check_block_expecting(body, Some(output_ty), Some(output_span));
                });

                this.check_nested_functions(nested_items(body), def);
            });
        });

        scc
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
                cx.items_by_def.insert(def.id(), item);

                let Some(body) = f.body.as_ref() else {
                    return;
                };

                collect_fn_mod_items(cx, nested_items(body));
            });
        }
        ItemKind::Mod(ident, kind) => {
            let name = ident.symbol;
            cx.with_mod_scope(name, |cx, def| {
                cx.items_by_def.insert(def.id(), item);

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

#[cfg(test)]
mod tests {
    use ast::{Expr, ExprKind};

    use crate::tests::*;

    #[test]
    fn check_expr_bool_literal() {
        let mut cx = TypeCheckContext::new(Interner::new());
        let expr_val = expr(&mut cx.symbols, "true");
        let t = cx.check_expr(&expr_val, None);
        assert_eq!(resolved_kind(&mut cx, t), Some(TyKind::Bool));
        let expr_val = expr(&mut cx.symbols, "false");
        let t = cx.check_expr(&expr_val, None);
        assert_eq!(resolved_kind(&mut cx, t), Some(TyKind::Bool));
    }

    #[test]
    fn check_expr_int_literal() {
        let mut cx = TypeCheckContext::new(Interner::new());
        let expr_val = expr(&mut cx.symbols, "5");
        let t = cx.check_expr(&expr_val, None);
        assert_eq!(resolved_kind(&mut cx, t), Some(TyKind::Int));
    }

    #[test]
    fn check_expr_float_literal() {
        let mut cx = TypeCheckContext::new(Interner::new());
        let expr_val = expr(&mut cx.symbols, "5.0");
        let t = cx.check_expr(&expr_val, None);
        assert_eq!(resolved_kind(&mut cx, t), Some(TyKind::Float));
    }

    #[test]
    fn check_expr_str_literal() {
        let mut cx = TypeCheckContext::new(Interner::new());
        let expr_val = expr(&mut cx.symbols, "\"hi\"");
        let t = cx.check_expr(&expr_val, None);
        assert_eq!(resolved_kind(&mut cx, t), Some(TyKind::Str));
    }

    #[test]
    fn check_expr_paren_has_the_inner_exprs_type() {
        let mut cx = TypeCheckContext::new(Interner::new());
        let expr_val = expr(&mut cx.symbols, "(5)");
        let t = cx.check_expr(&expr_val, None);
        assert_eq!(resolved_kind(&mut cx, t), Some(TyKind::Int));
    }

    #[test]
    fn check_expr_err_is_a_wildcard() {
        let mut cx = TypeCheckContext::new(Interner::new());
        let err_expr = Expr {
            annotations: Vec::new(),
            kind: ExprKind::Err,
            span: ast::Span { start: 0, end: 0 },
        };
        let bool_ty = cx.ty(TyKind::Bool);
        let t = cx.check_expr(&err_expr, Some(bool_ty));
        assert_eq!(resolved_kind(&mut cx, t), Some(TyKind::Err));
    }

    #[test]
    fn check_expr_unifies_the_result_against_the_expected_type() {
        let mut cx = resolve("fn foo() {}");
        let target = path(&mut cx.symbols, &["foo"]);
        let def = cx
            .resolve_path_to_value(&target)
            .expect("foo should resolve");
        let def_ty = cx.def(def).ty();

        let never_ty = cx.ty(TyKind::Never);
        let expr_val = expr(&mut cx.symbols, "foo");
        cx.check_expr(&expr_val, Some(never_ty));

        assert_eq!(resolved_kind(&mut cx, def_ty), Some(TyKind::Never));
    }

    #[test]
    fn check_expr_tup_elements_keep_independent_types() {
        let mut cx = TypeCheckContext::new(Interner::new());
        let expr_val = expr(&mut cx.symbols, "(1, \"hi\")");
        let t = cx.check_expr(&expr_val, None);
        let Some(TyKind::Tuple(args)) = resolved_kind(&mut cx, t) else {
            panic!("should be a Tuple ty");
        };
        assert_eq!(resolved_kind(&mut cx, args[0]), Some(TyKind::Int));
        assert_eq!(resolved_kind(&mut cx, args[1]), Some(TyKind::Str));
    }

    #[test]
    fn check_expr_array_elements_are_unified_with_each_other() {
        let mut cx = TypeCheckContext::new(Interner::new());
        let expr_val = expr(&mut cx.symbols, "[1, 2, 3]");
        let t = cx.check_expr(&expr_val, None);
        let Some(TyKind::Array(elem)) = resolved_kind(&mut cx, t) else {
            panic!("should be an Array ty");
        };
        assert_eq!(resolved_kind(&mut cx, elem), Some(TyKind::Int));
    }

    #[test]
    fn check_expr_empty_array_uses_the_expected_element_type() {
        let mut cx = TypeCheckContext::new(Interner::new());
        let never_ty = cx.ty(TyKind::Never);
        let array_of_never = cx.ty(TyKind::Array(never_ty));

        let expr_val = expr(&mut cx.symbols, "[]");
        let t = cx.check_expr(&expr_val, Some(array_of_never));
        let Some(TyKind::Array(elem)) = resolved_kind(&mut cx, t) else {
            panic!("should be an Array ty");
        };
        assert_eq!(resolved_kind(&mut cx, elem), Some(TyKind::Never));
    }

    #[test]
    fn check_expr_path_resolves_to_the_defs_type() {
        let mut cx = resolve("fn foo() {}");
        let target = path(&mut cx.symbols, &["foo"]);
        let def = cx
            .resolve_path_to_value(&target)
            .expect("foo should resolve");
        let def_ty = cx.def(def).ty();

        let expr_val = expr(&mut cx.symbols, "foo");
        let t = cx.check_expr(&expr_val, None);
        assert_eq!(t, def_ty);
    }

    #[test]
    fn check_expr_path_to_an_undeclared_symbol_is_err() {
        let mut cx = TypeCheckContext::new(Interner::new());
        let expr_val = expr(&mut cx.symbols, "doesNotExist");
        let t = cx.check_expr(&expr_val, None);
        assert_eq!(resolved_kind(&mut cx, t), Some(TyKind::Err));
    }

    #[test]
    fn check_expr_cast_lowers_the_target_type() {
        let mut cx = TypeCheckContext::new(Interner::new());
        let expr_val = expr(&mut cx.symbols, "5 as float");
        let t = cx.check_expr(&expr_val, None);
        assert_eq!(resolved_kind(&mut cx, t), Some(TyKind::Float));
    }

    #[test]
    fn check_expr_call_pins_the_callees_type_to_a_fn_shape() {
        let mut cx = resolve("fn foo() {}");
        let target = path(&mut cx.symbols, &["foo"]);
        let def = cx
            .resolve_path_to_value(&target)
            .expect("foo should resolve");
        let def_ty = cx.def(def).ty();

        let expr_val = expr(&mut cx.symbols, "foo()");
        cx.check_expr(&expr_val, None);

        assert!(matches!(
            resolved_kind(&mut cx, def_ty),
            Some(TyKind::Fn(..))
        ));
    }

    #[test]
    fn check_expr_call_checks_arguments_against_the_signature() {
        let mut cx = resolve("fn foo() {}");
        let expr_val = expr(&mut cx.symbols, "foo(5)");
        cx.check_expr(&expr_val, None);

        let target = path(&mut cx.symbols, &["foo"]);
        let def = cx
            .resolve_path_to_value(&target)
            .expect("foo should resolve");
        let def_ty = cx.def(def).ty();

        let Some(TyKind::Fn(input_args, _)) = resolved_kind(&mut cx, def_ty) else {
            panic!("should be a Fn ty");
        };
        assert_eq!(resolved_kind(&mut cx, input_args[0]), Some(TyKind::Int));
    }

    #[test]
    fn check_expr_call_result_is_an_unbound_var_when_nothing_constrains_it() {
        let mut cx = resolve("fn foo() {}");
        let expr_val = expr(&mut cx.symbols, "foo()");
        let t = cx.check_expr(&expr_val, None);
        let resolved = cx.inf.resolve(t);
        assert!(matches!(cx.inf.ty(resolved), Some(TyKind::Var(_))));
    }

    #[test]
    fn check_all_calling_an_unannotated_parameter_infers_its_fn_shape_with_no_error() {
        let source = indoc! {r#"
            fn apply(f, x) {
                f(x)
            }
        "#};
        let mut cx = check_all(source);
        assert!(cx.diagnostics.is_empty());

        let target = path(&mut cx.symbols, &["apply"]);
        let apply = cx
            .resolve_path_to_value(&target)
            .expect("apply should resolve");
        assert_eq!(
            cx.def(apply).generics().len(),
            2,
            "<T, U> Fn(Fn(T) -> U, T) -> U"
        );
    }

    #[test]
    fn check_expr_ret_with_no_value_is_never() {
        let mut cx = TypeCheckContext::new(Interner::new());
        let expr_val = expr(&mut cx.symbols, "return");
        let t = cx.check_expr(&expr_val, None);
        assert_eq!(resolved_kind(&mut cx, t), Some(TyKind::Never));
    }

    #[test]
    fn check_expr_ret_with_a_value_is_still_never_not_the_values_type() {
        let mut cx = TypeCheckContext::new(Interner::new());
        let expr_val = expr(&mut cx.symbols, "return 5");
        let t = cx.check_expr(&expr_val, None);
        assert_eq!(resolved_kind(&mut cx, t), Some(TyKind::Never));
    }

    #[test]
    fn never_is_a_wildcard_that_unifies_with_anything() {
        let mut cx = TypeCheckContext::new(Interner::new());
        let never_ty = cx.ty(TyKind::Never);
        let int_ty = cx.ty(TyKind::Int);
        assert!(cx.inf.unify(never_ty, int_ty).is_ok());
    }

    #[test]
    fn if_with_no_else_and_a_unit_then_branch_is_unit_typed() {
        let mut cx = TypeCheckContext::new(Interner::new());
        let expr_val = expr(&mut cx.symbols, "if true { }");
        let t = cx.check_expr(&expr_val, None);
        assert_eq!(resolved_kind(&mut cx, t), Some(TyKind::Tuple(Vec::new())));
    }

    #[test]
    fn if_branches_are_unified_together() {
        let mut cx = TypeCheckContext::new(Interner::new());
        let expr_val = expr(&mut cx.symbols, "if true { 1 } else { 2 }");
        let t = cx.check_expr(&expr_val, None);
        assert_eq!(resolved_kind(&mut cx, t), Some(TyKind::Int));
    }

    #[test]
    fn if_prefers_the_else_branchs_type_when_the_then_branch_diverges() {
        let mut cx = TypeCheckContext::new(Interner::new());
        let expr_val = expr(&mut cx.symbols, "if true { return } else { 5 }");
        let t = cx.check_expr(&expr_val, None);
        assert_eq!(resolved_kind(&mut cx, t), Some(TyKind::Int));
    }

    #[test]
    fn if_prefers_the_then_branchs_type_when_the_else_branch_diverges() {
        let mut cx = TypeCheckContext::new(Interner::new());
        let expr_val = expr(&mut cx.symbols, "if true { 5 } else { return }");
        let t = cx.check_expr(&expr_val, None);
        assert_eq!(resolved_kind(&mut cx, t), Some(TyKind::Int));
    }

    #[test]
    fn if_is_never_when_both_branches_diverge() {
        let mut cx = TypeCheckContext::new(Interner::new());
        let expr_val = expr(&mut cx.symbols, "if true { return } else { return }");
        let t = cx.check_expr(&expr_val, None);
        assert_eq!(resolved_kind(&mut cx, t), Some(TyKind::Never));
    }

    #[test]
    fn if_prefers_the_then_branchs_type_when_the_else_branch_diverges_via_a_semicolon() {
        let mut cx = TypeCheckContext::new(Interner::new());
        let expr_val = expr(&mut cx.symbols, "if true { 5 } else { return 0; }");
        let t = cx.check_expr(&expr_val, None);
        assert_eq!(resolved_kind(&mut cx, t), Some(TyKind::Int));
    }

    #[test]
    fn check_block_empty_is_unit() {
        let mut cx = TypeCheckContext::new(Interner::new());
        let block_val = block(&mut cx.symbols, "{}");
        let t = cx.check_block(&block_val, None);
        assert_eq!(resolved_kind(&mut cx, t), Some(TyKind::Tuple(Vec::new())));
    }

    #[test]
    fn check_block_trailing_expr_with_no_semicolon_is_its_type() {
        let mut cx = TypeCheckContext::new(Interner::new());
        let block_val = block(&mut cx.symbols, "{ 5 }");
        let t = cx.check_block(&block_val, None);
        assert_eq!(resolved_kind(&mut cx, t), Some(TyKind::Int));
    }

    #[test]
    fn check_block_trailing_expr_with_a_semicolon_does_not_count() {
        let mut cx = TypeCheckContext::new(Interner::new());
        let block_val = block(&mut cx.symbols, "{ 5; }");
        let t = cx.check_block(&block_val, None);
        assert_eq!(resolved_kind(&mut cx, t), Some(TyKind::Tuple(Vec::new())));
    }

    #[test]
    fn check_block_a_semicolon_tyinated_return_makes_the_block_never() {
        let mut cx = TypeCheckContext::new(Interner::new());
        let block_val = block(&mut cx.symbols, "{ return 0; }");
        let t = cx.check_block(&block_val, None);
        assert_eq!(resolved_kind(&mut cx, t), Some(TyKind::Never));
    }

    #[test]
    fn check_block_a_non_trailing_let_declares_a_def_visible_to_later_statements() {
        let mut cx = TypeCheckContext::new(Interner::new());
        let block_val = block(&mut cx.symbols, "{ let x = 5; x }");
        let t = cx.check_block(&block_val, None);
        assert_eq!(resolved_kind(&mut cx, t), Some(TyKind::Int));
    }

    #[test]
    fn check_block_a_non_trailing_lets_ascription_propagates_to_a_later_reference() {
        let mut cx = TypeCheckContext::new(Interner::new());
        let block_val = block(&mut cx.symbols, "{ let x: float; let y = x; y }");
        let t = cx.check_block(&block_val, None);
        assert_eq!(resolved_kind(&mut cx, t), Some(TyKind::Float));
    }

    #[test]
    fn check_pat_ident_declares_a_local_def() {
        let mut cx = TypeCheckContext::new(Interner::new());
        let never_ty = cx.ty(TyKind::Never);
        let pat_val = pat(&mut cx.symbols, "x");
        cx.check_pat(&pat_val, never_ty, PatDeclKind::Let);

        assert!(lookup(&cx, cx.current_scope, Namespace::Value, "x"));
    }

    #[test]
    fn check_pat_ident_binds_the_locals_type_to_expected() {
        let mut cx = TypeCheckContext::new(Interner::new());
        let never_ty = cx.ty(TyKind::Never);
        let pat_val = pat(&mut cx.symbols, "x");
        cx.check_pat(&pat_val, never_ty, PatDeclKind::Let);

        let def = declared_def(&cx, cx.current_scope, Namespace::Value, "x")
            .expect("x should be declared");
        let def_ty = cx.def(def).ty();
        assert_eq!(resolved_kind(&mut cx, def_ty), Some(TyKind::Never));
    }

    #[test]
    fn check_pat_wild_matches_anything_and_binds_nothing() {
        let mut cx = TypeCheckContext::new(Interner::new());
        let never_ty = cx.ty(TyKind::Never);
        let pat_val = pat(&mut cx.symbols, "_");
        let t = cx.check_pat(&pat_val, never_ty, PatDeclKind::Let);
        assert_eq!(t, never_ty);
        assert!(cx.defs.is_empty());
    }

    #[test]
    fn check_pat_tuple_declares_one_local_per_position() {
        let mut cx = TypeCheckContext::new(Interner::new());
        let never_ty = cx.ty(TyKind::Never);
        let int_ty = cx.ty(TyKind::Int);
        let expected = cx.ty(TyKind::Tuple(vec![never_ty, int_ty]));

        let pat_val = pat(&mut cx.symbols, "(a, b)");
        cx.check_pat(&pat_val, expected, PatDeclKind::Let);

        let a = declared_def(&cx, cx.current_scope, Namespace::Value, "a")
            .expect("a should be declared");
        let b = declared_def(&cx, cx.current_scope, Namespace::Value, "b")
            .expect("b should be declared");
        let a_ty = cx.def(a).ty();
        let b_ty = cx.def(b).ty();
        assert_eq!(resolved_kind(&mut cx, a_ty), Some(TyKind::Never));
        assert_eq!(resolved_kind(&mut cx, b_ty), Some(TyKind::Int));
    }

    #[test]
    fn check_pat_tuple_with_no_matching_expected_shape_uses_fresh_vars_per_position() {
        let mut cx = TypeCheckContext::new(Interner::new());
        let int_ty = cx.ty(TyKind::Int);
        let pat_val = pat(&mut cx.symbols, "(a, b)");
        let t = cx.check_pat(&pat_val, int_ty, PatDeclKind::Let);
        let Some(TyKind::Tuple(args)) = resolved_kind(&mut cx, t) else {
            panic!("should be a Tuple ty");
        };
        assert_eq!(args.len(), 2);
    }

    #[test]
    fn check_local_declares_the_pattern_with_the_initializers_type() {
        let mut cx = TypeCheckContext::new(Interner::new());
        let local_val = local(&mut cx.symbols, "let x = 5;");
        cx.check_local(&local_val);

        let def = declared_def(&cx, cx.current_scope, Namespace::Value, "x")
            .expect("x should be declared");
        let def_ty = cx.def(def).ty();
        assert_eq!(resolved_kind(&mut cx, def_ty), Some(TyKind::Int));
    }

    #[test]
    fn check_local_with_no_initializer_uses_the_ascription() {
        let mut cx = TypeCheckContext::new(Interner::new());
        let local_val = local(&mut cx.symbols, "let x: !;");
        cx.check_local(&local_val);

        let def = declared_def(&cx, cx.current_scope, Namespace::Value, "x")
            .expect("x should be declared");
        let def_ty = cx.def(def).ty();
        assert_eq!(resolved_kind(&mut cx, def_ty), Some(TyKind::Never));
    }

    #[test]
    fn check_local_ascription_constrains_the_initializer() {
        let mut cx = resolve("fn foo() {}");
        let target = path(&mut cx.symbols, &["foo"]);
        let def = cx
            .resolve_path_to_value(&target)
            .expect("foo should resolve");

        let local_val = local(&mut cx.symbols, "let x: ! = foo();");
        cx.check_local(&local_val);

        let def_ty = cx.def(def).ty();
        let Some(TyKind::Fn(_, ret)) = resolved_kind(&mut cx, def_ty) else {
            panic!("should be a Fn ty");
        };
        assert_eq!(resolved_kind(&mut cx, ret), Some(TyKind::Never));
    }

    #[test]
    fn check_all_infers_an_untyped_params_type_from_the_bodys_declared_return_type() {
        let mut cx = check_all("fn identity(x) -> int { x }");
        let target = path(&mut cx.symbols, &["identity"]);
        let fn_def = cx
            .resolve_path_to_value(&target)
            .expect("identity should resolve");
        let body_scope = fn_body_scope(&cx, fn_def);

        let x_def = declared_def(&cx, body_scope, Namespace::Value, "x")
            .expect("x should be declared as a param");
        let x_ty = cx.def(x_def).ty();
        assert_eq!(resolved_kind(&mut cx, x_ty), Some(TyKind::Int));
    }

    #[test]
    fn check_all_recurses_into_a_nested_fns_body() {
        let mut cx = check_all("fn outer() { fn inner(x) -> int { x } }");
        let target = path(&mut cx.symbols, &["outer"]);
        let outer_def = cx
            .resolve_path_to_value(&target)
            .expect("outer should resolve");
        let outer_scope = fn_body_scope(&cx, outer_def);

        let inner_def = declared_def(&cx, outer_scope, Namespace::Value, "inner")
            .expect("inner should be declared inside outer's body");
        let inner_scope = fn_body_scope(&cx, inner_def);

        let x_def = declared_def(&cx, inner_scope, Namespace::Value, "x")
            .expect("x should be declared as inner's param");
        let x_ty = cx.def(x_def).ty();
        assert_eq!(resolved_kind(&mut cx, x_ty), Some(TyKind::Int));
    }

    #[test]
    fn check_all_nested_fn_body_resolves_a_reference_to_an_outer_params_def() {
        let source = indoc! {r#"
            fn outer(x: int) {
                fn inner() {
                    x;
                }
            }
        "#};
        let cx = check_all(source);

        let param_decl_offset = source.find("x: int").unwrap();
        let param_use_offset = source.rfind('x').unwrap();
        assert_ne!(param_decl_offset, param_use_offset);

        let decl_def = cx
            .def_at(param_decl_offset)
            .expect("should resolve at outer's parameter declaration");
        let use_def = cx
            .def_at(param_use_offset)
            .expect("inner's reference to x should resolve to outer's parameter");
        assert_eq!(decl_def, use_def);
    }
}
