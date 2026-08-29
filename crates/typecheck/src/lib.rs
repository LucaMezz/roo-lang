//! Contains the type checking stage of the interpreter.
//!
//! The type checking stage depends on the AST that was constructed
//! by the `parser` during the previous stage. Given some AST, the
//! type checker does several passes of the AST.
//!
//! ```text
//!     1.
//! ```

use std::collections::HashMap;

use ast::visit::Visitor;
use ast::{FnRetTy, FnTy, GenericParam, Item, ItemKind, Path, Span, Ty, TyKind as AstTyKind};
use intern::{Interner, Symbol};

use crate::inference::{InferenceTable, TyId, VarId};
use crate::types::TyKind;

mod adt;
mod call_graph;
mod check;
mod checked_program;
mod defs;
mod errors;
mod generics;
mod inference;
mod lower_signatures;
mod polymorphism;
mod position_index;
mod recursion;
mod resolve;
mod scope;
mod types;

use check::collect_fn_mod_items;
use defs::{Def, DefKind, Defs, EnumDef, VariantDef};
use errors::Diagnostics;
use generics::{GenericId, GenericRegistry};
use lower_signatures::SignatureLowerer;
use position_index::PositionIndex;
use recursion::RecursionTracker;
use resolve::Resolver;
use scope::{Namespace, ScopeId, ScopeTree};

pub use checked_program::CheckedProgram;
pub use diagnostics::{Diagnostic, Level};
pub use errors::Locale;

use crate::errors::{AlreadyDefined, AnnotationsNeeded, UnresolvedType};

slotmap::new_key_type! {
    /// A handle to a def stored in the def arena within
    /// the [`TypeCheckContext`]
    pub struct DefId;
}

#[derive(Debug)]
struct ImplInfo {
    scope: ScopeId,
    of_trait: Option<DefId>,
    generics: Vec<GenericId>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum ImplTarget {
    Adt(DefId),
    Int,
    Float,
    Bool,
    Str,
    Array,
    Tuple(usize),
}

fn impl_target_of(kind: &TyKind) -> Option<ImplTarget> {
    match kind {
        TyKind::Struct(def, _) | TyKind::Enum(def, _) => Some(ImplTarget::Adt(*def)),
        TyKind::Int => Some(ImplTarget::Int),
        TyKind::Float => Some(ImplTarget::Float),
        TyKind::Bool => Some(ImplTarget::Bool),
        TyKind::Str => Some(ImplTarget::Str),
        TyKind::Array(_) => Some(ImplTarget::Array),
        TyKind::Tuple(elems) => Some(ImplTarget::Tuple(elems.len())),
        _ => None,
    }
}

/// Differentiates between a variable introduced to the scope
/// of a function via being a parameter, and one introduced
/// by a let def.
#[derive(Clone, Copy)]
enum PatDeclKind {
    Param,
    Let,
}

impl PatDeclKind {
    fn def_kind(self, ty: TyId) -> DefKind {
        match self {
            PatDeclKind::Param => DefKind::Param(ty),
            PatDeclKind::Let => DefKind::Local(ty),
        }
    }
}

/// Holds all state required to perform type checking on an
/// entire program. Also provides methods used to complete
/// the type checking and type inference.
struct TypeCheckContext<'ast> {
    /// Keeps track of what every inference variable
    /// throughout the entire program is bound to. Facilitates
    /// O(1) unification of two tys, performing the required
    /// substitutions to ensure two tys are equal. Also
    /// stores a [`Span`] with the reason two tys were
    /// unified.
    inf: InferenceTable,

    /// Maps all symbols used throughout the type checking
    /// process to unique symbol IDs to improve performance.
    /// They are cheap to copy and hash, etc.
    symbols: Interner,

    /// The def table. See [`Defs`].
    defs: Defs,

    /// Contains all scopes within the program, linked into a
    /// tree by parent pointers. See [`ScopeTree`].
    scopes: ScopeTree,

    /// The registry of every generic type parameter in the
    /// program. See [`GenericRegistry`].
    generics: GenericRegistry,

    /// A handle to the scope that is currently being checked.
    current_scope: ScopeId,

    /// All the diagnostics that have been accumulated so far.
    diagnostics: Diagnostics,

    /// An index which allows you to query for defs and types
    /// and other things, based on spans within the source code.
    positions: PositionIndex,

    /// Tracks which function is currently being checked, the
    /// call graph, and SCC discovery. See [`RecursionTracker`].
    recursion: RecursionTracker,

    items_by_def: HashMap<DefId, &'ast Item>,

    impls_by_target: HashMap<ImplTarget, Vec<ImplInfo>>,

    inference_vars: Vec<(VarId, Span)>,
}

impl<'ast> TypeCheckContext<'ast> {
    /// Create a new blank [`TypeCheckContext`].
    fn new(symbols: Interner) -> Self {
        let (scopes, root) = ScopeTree::new();

        Self {
            inf: InferenceTable::new(),

            symbols,
            scopes,
            generics: GenericRegistry::new(),
            inference_vars: Vec::new(),
            defs: Defs::new(),
            current_scope: root,
            diagnostics: Diagnostics::default(),
            positions: PositionIndex::default(),
            recursion: RecursionTracker::new(),
            items_by_def: HashMap::new(),
            impls_by_target: HashMap::new(),
        }
    }

    fn def(&self, def: DefId) -> &Def {
        self.defs.get(def)
    }

    #[cfg(test)]
    pub(crate) fn diagnostics(&self) -> Vec<Diagnostic> {
        let catalog = errors::catalog(errors::Locale::EnUs);
        self.diagnostics
            .as_slice()
            .iter()
            .map(|d| d.render(catalog))
            .collect()
    }

    /// Updates the [`PositionIndex`] to include a newly found path.
    fn record_path_reference(&mut self, path: &Path, def: DefId) {
        if let Some(segment) = path.segments.last() {
            self.positions.record_def(segment.ident.span, def);
        }
    }

    #[cfg(test)]
    pub(crate) fn def_at(&self, offset: usize) -> Option<DefId> {
        self.positions.def_at(offset)
    }

    #[cfg(test)]
    pub(crate) fn type_symbol_at(&self, offset: usize) -> Option<&'static str> {
        self.positions.type_name_at(offset)
    }

    fn fresh_var_at(&mut self, span: Option<Span>) -> TyId {
        let var = self.inf.fresh_var();
        let ty = self.ty_var(var);
        if let Some(span) = span {
            self.inference_vars.push((var, span));
        }
        ty
    }

    fn fresh_var(&mut self) -> TyId {
        self.fresh_var_at(None)
    }

    fn or_fresh_var(&mut self, ty: Option<TyId>) -> TyId {
        ty.unwrap_or_else(|| self.fresh_var())
    }

    fn ty(&mut self, kind: TyKind) -> TyId {
        self.inf.insert_ty(kind)
    }

    fn ty_var(&mut self, id: VarId) -> TyId {
        self.ty(TyKind::Var(id))
    }

    /// Performs the resolution stage of the type checking.
    ///
    /// This is the first stage of the type checking process.
    /// It walks the AST, and creates a new def in the def
    /// table for each item.
    ///
    /// For example, say one of the items is a `function`
    /// ```ignore
    /// fn add_int<T>(a: T, b: int) {
    ///     a + b
    /// }
    /// ```
    /// Then a new [`Def`] is created within the def table
    /// for it, with [`DefKind::Fn`] kind. Note that in this
    /// stage *does* create a [`Scope`] for the function body,
    /// and declares defs for the generic type parameters,
    /// in this case just `T`. However, this stage does *not*
    /// try to determine the type of the new def for the
    /// function yet. Instead, it just assigns a fresh inference
    /// variable to represent its type.
    fn resolve(&mut self, items: &[Box<Item>]) {
        let mut resolver = Resolver { cx: self };
        items.iter().for_each(|item| resolver.visit_item(item));
    }

    /// Performs lowering of signatures. It does this for
    /// functions and also type aliases.
    ///
    /// This is the second stage of the type checking process.
    /// In the previous `resolve` stage, it created new defs
    /// for all of the items within the AST. However, it only
    /// assigns a fresh inference variable as the type of each
    /// def.
    ///
    /// This stage uses information in the AST about types of
    /// the defs and lowers those types into tys. It does
    /// this for all applicable kinds of items.
    ///
    /// For example, say one of the items is a function
    /// ```ignore
    /// fn add_int<T>(a: T, b: int) {
    ///     a + b
    /// }
    /// ```
    /// In the previous resolution stage, a new [`Def`] of
    /// kind [`DefKind::Fn`] would have been created inside
    /// the def table. Now, a new `ty` is created for the
    /// type of this `add_int` function, based on the type
    /// annotations in the signature only. So the [`Def`]
    /// will now have type `Fn<T>(T, int) -> ?a`. The return
    /// type is an inference variable because there is no
    /// return type annotation, and no type checking or
    /// inference of function bodies has been performed yet.
    fn lower_signatures(&mut self, items: &[Box<Item>]) {
        let mut lowerer = SignatureLowerer { cx: self };
        items.iter().for_each(|item| lowerer.visit_item(item));
    }

    /// Performs type checking of function bodies.
    ///
    /// This is the third and final stage of the type checking
    /// process. So far after the `resolve` and
    /// `lower_signatures` stages have compelted, we have a
    /// def table where functions all have types matching
    /// the types annotated in their signatures (including
    /// inference variables `?a` where type annotations were
    /// left out).
    ///
    /// This stage performs type checking and inference on
    /// the body of all functions. It also unifies the ty
    /// representing the type of the function that was created
    /// in the previous `lower_signatures` stage.
    ///
    /// For example, say there is a function
    /// ```ignore
    /// fn apply(f, x: int) {
    ///     f(x)
    /// }
    /// ```
    /// The def for `add_int` would have had its type
    /// recorded as `Fn(?a, x: int) -> ?b` in the previous
    /// step.
    ///
    /// Now we look at the body of the function. Since the
    /// parameter `f` is called with argument `x`, it would
    /// introduce the constraint that `f` must have type
    /// `Fn(int) -> ?c`, since `f` must be a function that
    /// can be called for that call to be valid, and it must
    /// support a parameter of type `int` since `x : int`.
    ///
    /// Additionally, since the call is also the last
    /// expression in the function and has no semicolon,
    /// we also require that the type of the expression
    /// matches the return type of the function `apply`.
    ///
    /// So we `unify(?c, ?b)` which means that both these
    /// inference variables must bind to the same ty.
    /// Suppose the representative is ?b. Then the inferred
    /// type of the function at this point is
    /// `Fn( Fn(int) -> ?b ) -> ?b`.
    ///
    /// At this point [`Self::generalize_group`] introduces
    /// a new type parameter `T` giving final inferred type
    /// `Fn<T>( Fn(int) -> T ) -> T`.
    fn check(&mut self, items: &'ast [Box<Item>]) {
        collect_fn_mod_items(self, items.iter().map(Box::as_ref));

        self.check_items(items.iter().map(Box::as_ref));

        // self.free_variables();
    }

    fn check_items(&mut self, items: impl IntoIterator<Item = &'ast Item>) {
        items.into_iter().for_each(|item| match &item.kind {
            ItemKind::Fn(f) => self.check_function(f),
            ItemKind::Mod(ident, kind) => self.check_module(ident, kind),
            _ => {}
        });
    }

    /// Iterates the recorded list of inference variables and the
    /// span within the source code of what the variables were
    /// created to represent the type of. Finds any inference
    /// variables that are still unbound and raises an error
    /// that type annotations are needed.
    ///
    /// NOTE Not sure if we actually need this function. If the
    /// type of something cannot be inferred, we don't really care
    /// at the moment. Types just ensure there are no type
    /// mismatches at compile time, so if something cant have a
    /// type inferred, it isn't really breaking anything.
    /// Its probably likely that it is dead / unused code anway.
    #[allow(unused)]
    fn free_variables(&mut self) {
        self.inference_vars
            .iter()
            .filter(|(v, _)| self.inf.binding(*v).is_none())
            .for_each(|(_, span)| {
                self.diagnostics.push(AnnotationsNeeded::new(*span));
            });
    }

    /// Returns a handle to the [`Def`] which the given path
    /// references, if it exists in the given Namespace.
    ///
    /// Searches for the def represented by the first
    /// segment in the path recursively up the chain of
    /// enclosed scopes. If it finds it, it then continues
    /// recursively resolving the shortened path until
    /// it potentially arrives at a def.
    fn resolve_path(&mut self, path: &Path, namespace: Namespace) -> Option<DefId> {
        let last = path.segments.len() - 1;

        let mut segments = path.segments.iter().enumerate();
        let (i, first) = segments.next()?;
        let symbol = first.ident.symbol;
        let first_def = self.scopes.lookup_up_chain(
            self.current_scope,
            symbol,
            segment_namespace(i, last, namespace),
        )?;

        segments.try_fold(first_def, |def, (i, segment)| {
            let scope = self.mod_def_scope(def).or(self.enum_def_scope(def))?;
            let symbol = segment.ident.symbol;
            self.scopes
                .lookup(scope, symbol, segment_namespace(i, last, namespace))
        })
    }

    fn resolve_path_to_type(&mut self, path: &Path) -> Option<DefId> {
        self.resolve_path(path, Namespace::Type)
    }

    fn resolve_path_to_value(&mut self, path: &Path) -> Option<DefId> {
        self.resolve_path(path, Namespace::Value)
    }

    fn resolve_path_to_struct(&mut self, path: &Path) -> Option<(DefId, &VariantDef)> {
        let id = self.resolve_path_to_type(path)?;
        self.def(id).kind.as_struct().map(|variant| (id, variant))
    }

    fn resolve_path_to_enum(&mut self, path: &Path) -> Option<DefId> {
        self.resolve_path_to_type(path)
            .filter(|def| matches!(self.def(*def).kind, DefKind::Enum(_)))
    }

    fn resolve_path_to_variant(&mut self, path: &Path) -> Option<(DefId, &VariantDef)> {
        let id = self.resolve_path_to_value(path)?;
        self.def(id)
            .kind
            .as_variant()
            .filter(|variant| variant.ctor_ty.is_none())
            .map(|variant| (id, variant))
    }

    fn resolve_path_from(
        &mut self,
        root: DefId,
        path: &Path,
        namespace: Namespace,
    ) -> Option<DefId> {
        let last = path.segments.len() - 1;
        path.segments
            .iter()
            .enumerate()
            .try_fold(root, |def, (i, segment)| {
                let scope = self.mod_def_scope(def)?;
                let symbol = segment.ident.symbol;
                self.scopes
                    .lookup(scope, symbol, segment_namespace(i, last, namespace))
            })
    }

    fn with_def_in_scope<T>(
        &mut self,
        symbol: Symbol,
        namespace: Namespace,
        f: impl FnOnce(&mut Self, DefId) -> T,
    ) -> Option<T> {
        self.scopes
            .lookup(self.current_scope, symbol, namespace)
            .map(|def| f(self, def))
    }

    fn with_type_def<T>(
        &mut self,
        symbol: Symbol,
        f: impl FnOnce(&mut Self, DefId) -> T,
    ) -> Option<T> {
        self.with_def_in_scope(symbol, Namespace::Type, f)
    }

    fn with_value_def<T>(
        &mut self,
        symbol: Symbol,
        f: impl FnOnce(&mut Self, DefId) -> T,
    ) -> Option<T> {
        self.with_def_in_scope(symbol, Namespace::Value, f)
    }

    fn fn_def_scope(&self, def: DefId) -> Option<ScopeId> {
        let DefKind::Fn(fn_data) = &self.def(def).kind else {
            return None;
        };
        Some(fn_data.scope)
    }

    fn struct_def_scope(&self, def: DefId) -> Option<ScopeId> {
        let DefKind::Struct(def) = &self.def(def).kind else {
            return None;
        };
        Some(def.scope)
    }

    fn mod_def_scope(&self, def: DefId) -> Option<ScopeId> {
        let DefKind::Mod(scope) = self.def(def).kind else {
            return None;
        };
        Some(scope)
    }

    fn enum_def_scope(&self, def: DefId) -> Option<ScopeId> {
        let DefKind::Enum(EnumDef { scope, .. }) = self.def(def).kind else {
            return None;
        };
        Some(scope)
    }

    fn ty_alias_def_scope(&self, def: DefId) -> Option<ScopeId> {
        let DefKind::TyAlias(alias_data) = &self.def(def).kind else {
            return None;
        };
        Some(alias_data.scope)
    }

    fn register_impl_for(
        &mut self,
        target: ImplTarget,
        scope: ScopeId,
        of_trait: Option<DefId>,
        generics: Vec<GenericId>,
    ) {
        self.impls_by_target
            .entry(target)
            .or_default()
            .push(ImplInfo {
                scope,
                of_trait,
                generics,
            });
    }

    /// Declares a new [`Def`] in a scope of a certain kind.
    ///
    /// Any ty this kind of def needs must already be embedded
    /// in `kind` by the caller (see e.g. [`PatDeclKind::def_kind`],
    /// which requires a fresh inference variable `?a` be created
    /// up front to represent the type of the def until something
    /// else later constrains it).
    fn declare(&mut self, symbol: Symbol, span: Span, kind: DefKind) -> DefId {
        let namespace = kind.namespace();

        // `let` defs (and, transitively through them, their
        // shadowing sub-patterns) are always allowed to shadow
        // whatever previously held the same symbol in this scope.
        // Everything else -- functions, modules, structs, enums,
        // traits, type aliases, and parameters -- must be uniquely
        // symbold within a scope.
        if !matches!(kind, DefKind::Local(_)) {
            self.check_redeclaration(namespace, symbol, span);
        }

        let def = self.defs.insert(Def {
            symbol,
            kind,
            declared_at: span,
        });
        self.insert_in_scope(symbol, def, namespace);
        self.positions.record_def(span, def);
        def
    }

    /// If a def already exists with the given symbol in the given
    /// Namespace of the current scope, emits an [`AlreadyDefined`]
    /// diagnostic pointing back at its original declaration.
    fn check_redeclaration(&mut self, namespace: Namespace, symbol: Symbol, span: Span) {
        self.scopes
            .lookup(self.current_scope, symbol, namespace)
            .into_iter()
            .for_each(|existing| {
                let original = self.def(existing).declared_at;
                let symbol = self.symbols.resolve(symbol).to_owned();
                self.diagnostics
                    .push(AlreadyDefined::new(span, symbol, original));
            });
    }

    /// Declares a new generic parameter within the current
    /// scope. For example, say there is a function
    /// ```ignore
    /// fn identity<T>(x: T) -> T {
    ///     x
    /// }
    /// ```
    /// then when resolving this function, the scope of the
    /// body is entered, and a new [`DefKind::GenericParam`]
    /// is created for `T` inside that scope so `T` becomes
    /// a valid type that can be used within the function body.
    fn declare_generic_param(&mut self, symbol: Symbol, span: Span) -> (DefId, GenericId) {
        let id = self
            .generics
            .declare_new(self.symbols.resolve(symbol).to_owned());
        let ty = self.ty(TyKind::Generic(id));
        let def = self.defs.insert(Def {
            symbol,
            kind: DefKind::GenericParam(ty),
            declared_at: span,
        });
        self.insert_type_in_scope(symbol, def);
        self.positions.record_def(span, def);
        (def, id)
    }

    fn declare_generic_params(&mut self, params: &[GenericParam]) -> Vec<GenericId> {
        params
            .iter()
            .map(|param| {
                self.declare_generic_param(param.ident.symbol, param.ident.span)
                    .1
            })
            .collect()
    }

    /// Inserts a def into the current scope.
    fn insert_in_scope(&mut self, symbol: Symbol, def: DefId, namespace: Namespace) {
        self.scopes
            .insert(self.current_scope, symbol, def, namespace);
    }

    fn insert_type_in_scope(&mut self, symbol: Symbol, def: DefId) {
        self.insert_in_scope(symbol, def, Namespace::Type);
    }

    fn insert_value_in_scope(&mut self, symbol: Symbol, def: DefId) {
        self.insert_in_scope(symbol, def, Namespace::Value);
    }

    /// Convert a [`Ty`] AST node into a ty which represents
    /// that type and which can actually be used by the type
    /// checker to perform checking and inference.
    fn lower_ty(&mut self, ty: &Ty) -> TyId {
        match &ty.kind {
            AstTyKind::Never => self.ty(TyKind::Never),
            AstTyKind::Paren(inner) => self.lower_ty(inner),
            AstTyKind::Array(inner) => self.lower_array_ty(inner),
            AstTyKind::Tup(inner) => self.lower_tup_ty(inner),
            AstTyKind::Fn(fn_ty) => self.lower_fn_ty(fn_ty, ty.span),
            AstTyKind::Path(path) => self.lower_path_ty(path),
            AstTyKind::ImplicitSelf => self.lower_implicit_self_ty(),
            // When `_` is used as a type annotation, it means
            // the type should be inferred. Hence, introduce
            // a fresh inference variable `?a` to represent
            // this type.
            AstTyKind::Infer => self.fresh_var_at(Some(ty.span)),
            AstTyKind::Err => self.ty(TyKind::Err),
        }
    }

    fn lower_array_ty(&mut self, inner: &Ty) -> TyId {
        let elem = self.lower_ty(inner);
        self.ty(TyKind::Array(elem))
    }

    fn lower_tup_ty(&mut self, inner: &[Box<Ty>]) -> TyId {
        let args = inner.iter().map(|x| self.lower_ty(x)).collect();
        self.ty(TyKind::Tuple(args))
    }

    fn lower_fn_ty(&mut self, fn_ty: &FnTy, span: Span) -> TyId {
        let FnTy { inputs, output } = fn_ty;
        let input_args = inputs.iter().map(|x| self.lower_ty(x)).collect();
        let output_ty = self.lower_ret_ty(output, Some(span));
        self.ty(TyKind::Fn(input_args, output_ty))
    }

    fn lower_ret_ty(&mut self, output: &FnRetTy, default_span: Option<Span>) -> TyId {
        match output {
            FnRetTy::Default(_) => self.fresh_var_at(default_span),
            FnRetTy::Ty(ty) => self.lower_ty(ty),
        }
    }

    fn lower_implicit_self_ty(&mut self) -> TyId {
        unimplemented!()
    }

    fn lower_path_ty(&mut self, path: &Path) -> TyId {
        if let [segment] = path.segments.as_slice() {
            let found = PRIMITIVE_TYPES
                .iter()
                .find(|(symbol, _)| self.symbols.get(symbol) == Some(segment.ident.symbol));
            if let Some((symbol, con)) = found {
                self.positions.record_primitive(segment.ident.span, symbol);
                return self.ty(con.clone());
            }
        }

        self.resolve_path_to_type(path)
            .map(|def| {
                self.record_path_reference(path, def);
                match &self.def(def).kind {
                    DefKind::Struct(_) => {
                        let generics = self.def(def).generics().to_vec();
                        let args = self.instantiate_adt_args(&generics, path);
                        self.ty(TyKind::Struct(def, args))
                    }
                    DefKind::Enum(_) => {
                        let generics = self.def(def).generics().to_vec();
                        let args = self.instantiate_adt_args(&generics, path);
                        self.ty(TyKind::Enum(def, args))
                    }
                    _ => self.instantiate_path(def, path),
                }
            })
            .unwrap_or_else(|| {
                self.diagnostics.push(UnresolvedType::new(
                    path.span,
                    display_path(path, &self.symbols),
                ));
                self.ty(TyKind::Err)
            })
    }
}

pub(crate) trait CxExt<'ast> {
    fn cx(&mut self) -> &mut TypeCheckContext<'ast>;

    /// Enters the given scope, performs some function while
    /// inside that scope, and then exits the scope once the
    /// function is complete.
    ///
    /// This ensures that even an early exit will still
    /// ensure that the current_scope is updated back to
    /// the parent scope.
    fn with_scope<T>(&mut self, scope: ScopeId, f: impl FnOnce(&mut Self) -> T) -> T {
        let parent = self.cx().current_scope;
        self.cx().current_scope = scope;
        let result = f(self);
        self.cx().current_scope = parent;
        result
    }

    fn with_fn_def<T>(
        &mut self,
        symbol: Symbol,
        f: impl FnOnce(&mut Self, DefId, ScopeId) -> T,
    ) -> Option<T> {
        let (def, scope) = self
            .cx()
            .with_value_def(symbol, |cx, def| {
                cx.fn_def_scope(def).map(|scope| (def, scope))
            })
            .flatten()?;
        Some(f(self, def, scope))
    }

    fn with_fn_scope<T>(
        &mut self,
        symbol: Symbol,
        f: impl FnOnce(&mut Self, DefId) -> T,
    ) -> Option<T> {
        self.with_fn_def(symbol, |this, def, scope| {
            this.with_scope(scope, |this| f(this, def))
        })
    }

    fn with_struct_def<T>(
        &mut self,
        symbol: Symbol,
        f: impl FnOnce(&mut Self, DefId, ScopeId) -> T,
    ) -> Option<T> {
        let (def, scope) = self
            .cx()
            .with_type_def(symbol, |cx, def| {
                cx.struct_def_scope(def).map(|scope| (def, scope))
            })
            .flatten()?;
        Some(f(self, def, scope))
    }

    fn with_struct_scope<T>(
        &mut self,
        symbol: Symbol,
        f: impl FnOnce(&mut Self, DefId) -> T,
    ) -> Option<T> {
        self.with_struct_def(symbol, |this, def, scope| {
            this.with_scope(scope, |this| f(this, def))
        })
    }

    fn with_enum_def<T>(
        &mut self,
        symbol: Symbol,
        f: impl FnOnce(&mut Self, DefId, ScopeId) -> T,
    ) -> Option<T> {
        let (def, scope) = self
            .cx()
            .with_type_def(symbol, |cx, def| {
                cx.enum_def_scope(def).map(|scope| (def, scope))
            })
            .flatten()?;
        Some(f(self, def, scope))
    }

    fn with_enum_scope<T>(
        &mut self,
        symbol: Symbol,
        f: impl FnOnce(&mut Self, DefId) -> T,
    ) -> Option<T> {
        self.with_enum_def(symbol, |this, def, scope| {
            this.with_scope(scope, |this| f(this, def))
        })
    }

    fn with_mod_def<T>(
        &mut self,
        symbol: Symbol,
        f: impl FnOnce(&mut Self, DefId, ScopeId) -> T,
    ) -> Option<T> {
        let (def, scope) = self
            .cx()
            .with_type_def(symbol, |cx, def| {
                cx.mod_def_scope(def).map(|scope| (def, scope))
            })
            .flatten()?;
        Some(f(self, def, scope))
    }

    fn with_mod_scope<T>(
        &mut self,
        symbol: Symbol,
        f: impl FnOnce(&mut Self, DefId) -> T,
    ) -> Option<T> {
        self.with_mod_def(symbol, |this, def, scope| {
            this.with_scope(scope, |this| f(this, def))
        })
    }

    fn with_ty_alias_def<T>(
        &mut self,
        symbol: Symbol,
        f: impl FnOnce(&mut Self, DefId, ScopeId) -> T,
    ) -> Option<T> {
        let (def, scope) = self
            .cx()
            .with_type_def(symbol, |cx, def| {
                cx.ty_alias_def_scope(def).map(|scope| (def, scope))
            })
            .flatten()?;
        Some(f(self, def, scope))
    }

    fn with_ty_alias_scope<T>(
        &mut self,
        symbol: Symbol,
        f: impl FnOnce(&mut Self, DefId) -> T,
    ) -> Option<T> {
        self.with_ty_alias_def(symbol, |this, def, scope| {
            this.with_scope(scope, |this| f(this, def))
        })
    }
}

impl<'ast> CxExt<'ast> for TypeCheckContext<'ast> {
    fn cx(&mut self) -> &mut TypeCheckContext<'ast> {
        self
    }
}

const PRIMITIVE_TYPES: &[(&str, TyKind)] = &[
    ("bool", TyKind::Bool),
    ("int", TyKind::Int),
    ("float", TyKind::Float),
    ("String", TyKind::Str),
];

fn display_path(path: &Path, symbols: &Interner) -> String {
    path.segments
        .iter()
        .map(|segment| symbols.resolve(segment.ident.symbol))
        .collect::<Vec<_>>()
        .join("::")
}

fn segment_namespace(i: usize, last: usize, namespace: Namespace) -> Namespace {
    if i == last {
        namespace
    } else {
        Namespace::Type
    }
}

#[cfg(test)]
mod tests;
