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

use ast::visit::{Visitor, Walkable};
use ast::{
    Fn, FnRetTy, FnTy, GenericParam, Ident, Item, ItemKind, ModKind, Path, Span, Trait, Ty,
    TyAlias, TyKind as AstTyKind, UseTree, UseTreeKind, VariantData,
};
use slotmap::SlotMap;

use crate::inference::{InferenceTable, TyId, VarId};
use crate::types::TyKind;

mod call_graph;
mod check;
mod checked_program;
mod errors;
mod generic_names;
mod inference;
mod polymorphism;
mod position_index;
mod types;

use check::collect_fn_mod_items;
use errors::Diagnostics;
use generic_names::GenericNames;
use position_index::PositionIndex;

pub use checked_program::CheckedProgram;
pub use diagnostics::{Diagnostic, Level};
pub use errors::Locale;

use crate::call_graph::{CallGraph, SCCCollector};
use crate::errors::{
    AlreadyDefined, AnnotationsNeeded, InvalidGlobTarget, UnresolvedImport, UnresolvedType,
};

slotmap::new_key_type! {
    /// A handle to a scope stored in the scope arena within
    /// the [`TypeCheckContext`]
    pub struct ScopeId;

    /// A handle to a symbol stored in the symbol arena within
    /// the [`TypeCheckContext`]
    pub struct SymbolId;

    /// A handle to a generic parameter stored in the generic
    /// arena within the [`TypeCheckContext`].
    pub struct GenericId;
}

/// A handle to a name. Use the NameInterner to transition
/// between a NameId, and the actual string which it is
/// associated with.
#[derive(Debug, Copy, Clone, Eq, PartialEq, Hash)]
struct NameId(usize);

/// A kind of namespace within each scope.
///
/// A scope has two separate namespaces for symbols. One only
/// contains symbols which represent types within the scope,
/// while the other only contains symbols which represent
/// values within the scope.
#[derive(Clone, Copy)]
enum Namespace {
    /// The namespace of Types within a scope.
    Type,

    /// The namespace of Values within a scope.
    Value,
}

/// A scope. Represents a context where symbols can be defined.
///
/// Scopes are created for things such as function bodies,
/// blocks, etc.
#[derive(Debug)]
struct Scope {
    /// A handle to the enclosing scope.
    parent: Option<ScopeId>,

    /// The [Namespace::Type] namespace. Maps the name of each
    /// type defined in this scope to its symbol's handle.
    types: HashMap<NameId, SymbolId>,

    /// The [Namespace::Value] namespace. Maps the name of
    /// each value defined in this scope to its symbol's
    /// handle.
    values: HashMap<NameId, SymbolId>,
}

/// A symbol within a symbol table.
#[derive(Debug)]
struct Symbol {
    /// An interned string which is the name of the symbol.
    name: NameId,

    /// The specific kind of symbol that it is.
    kind: SymbolKind,

    /// The span within the source code that resulted in
    /// the introduction of this symbol.
    declared_at: Span,
}

impl Symbol {
    /// The ty representing the type associated with this
    /// symbol.
    ///
    /// Panics if this symbol's kind can never have a ty (e.g.
    /// [`SymbolKind::Mod`]). Only call this where the kind is
    /// already known by construction.
    fn ty(&self) -> TyId {
        self.kind
            .ty()
            .expect("symbol kind does not have a ty")
    }

    /// The generic parameters associated with this symbol.
    /// Empty for kinds that can never have generics.
    fn generics(&self) -> &[GenericId] {
        self.kind.generics().unwrap_or(&[])
    }
}

/// Extra information about a function symbol.
#[derive(Debug)]
struct FnSymbol {
    /// A handle to the scope of the function body.
    scope: ScopeId,

    /// The span of each of the parameters of the function
    /// within the source code.
    param_spans: Vec<Option<Span>>,

    /// The name of each of the parameters as they appear
    /// in the source code.
    param_names: Vec<String>,

    /// The ty representing the type of this function.
    ty: TyId,

    /// The generic parameters associated with this function.
    generics: Vec<GenericId>,
}

/// Extra information about a type alias symbol.
#[derive(Debug)]
struct TyAliasSymbol {
    /// A handle to the scope in which the alias's generic
    /// parameters live.
    scope: ScopeId,

    /// The ty representing the aliased type.
    ty: TyId,

    /// The generic parameters associated with this alias.
    generics: Vec<GenericId>,
}

/// The specific kind of [`Symbol`].
#[derive(Debug)]
enum SymbolKind {
    Struct,
    Enum,
    Variant,
    Trait,
    /// A type alias. Type aliases need their own scope
    /// because they can have generic type parameters which
    /// should only exist during the evaluation of the
    /// type on the right hand side of the alias.
    TyAlias(TyAliasSymbol),
    /// A module. Here, the [`ScopeId`] is a handle to the
    /// scope of the module body.
    Mod(ScopeId),
    Fn(FnSymbol),
    Local(TyId),
    Param(TyId),
    GenericParam(TyId),
}

impl SymbolKind {
    /// A human-readable description of this kind of symbol,
    /// e.g. for use in diagnostics like "expected a module,
    /// found a function".
    fn describe(&self) -> &'static str {
        match self {
            SymbolKind::Struct => "a struct",
            SymbolKind::Enum => "an enum",
            SymbolKind::Variant => "an enum variant",
            SymbolKind::Trait => "a trait",
            SymbolKind::TyAlias(_) => "a type alias",
            SymbolKind::Mod(_) => "a module",
            SymbolKind::Fn(_) => "a function",
            SymbolKind::Local(_) => "a local variable",
            SymbolKind::Param(_) => "a parameter",
            SymbolKind::GenericParam(_) => "a generic parameter",
        }
    }

    /// The ty representing the type of this symbol, if this
    /// kind of symbol can have one at all.
    fn ty(&self) -> Option<TyId> {
        match self {
            SymbolKind::Fn(fn_data) => Some(fn_data.ty),
            SymbolKind::TyAlias(alias_data) => Some(alias_data.ty),
            SymbolKind::Local(ty) | SymbolKind::Param(ty) | SymbolKind::GenericParam(ty) => {
                Some(*ty)
            }
            SymbolKind::Struct
            | SymbolKind::Enum
            | SymbolKind::Variant
            | SymbolKind::Trait
            | SymbolKind::Mod(_) => None,
        }
    }

    /// The generic parameters of this symbol, if this kind of
    /// symbol can have any at all.
    fn generics(&self) -> Option<&[GenericId]> {
        match self {
            SymbolKind::Fn(fn_data) => Some(&fn_data.generics),
            SymbolKind::TyAlias(alias_data) => Some(&alias_data.generics),
            _ => None,
        }
    }
}

/// Differentiates between a variable introduced to the scope
/// of a function via being a parameter, and one introduced
/// by a let binding.
#[derive(Clone, Copy)]
enum PatDeclKind {
    Param,
    Let,
}

impl PatDeclKind {
    fn symbol_kind(self, ty: TyId) -> SymbolKind {
        match self {
            PatDeclKind::Param => SymbolKind::Param(ty),
            PatDeclKind::Let => SymbolKind::Local(ty),
        }
    }
}

impl SymbolKind {
    /// Which [`Namespace`] a symbol of this kind belongs to
    /// within a [`Scope`].
    fn namespace(&self) -> Namespace {
        match self {
            SymbolKind::Struct
            | SymbolKind::Enum
            | SymbolKind::Trait
            | SymbolKind::TyAlias(_)
            | SymbolKind::GenericParam(_)
            | SymbolKind::Mod(_) => Namespace::Type,
            SymbolKind::Variant
            | SymbolKind::Fn(_)
            | SymbolKind::Local(_)
            | SymbolKind::Param(_) => Namespace::Value,
        }
    }
}

/// Maps names to unique integer [`NameId`]s.
struct NameInterner {
    strings: Vec<String>,
    ids: HashMap<String, NameId>,
}

impl NameInterner {
    /// Create a new empty [`NameInterner`].
    pub fn new() -> Self {
        Self {
            strings: vec![],
            ids: HashMap::new(),
        }
    }

    /// Get the [`NameId`] associated with the given name.
    pub fn id(&mut self, string: &str) -> NameId {
        if let Some(id) = self.ids.get(string) {
            return *id;
        }

        let id = NameId(self.strings.len());
        self.strings.push(string.to_owned());
        self.ids.insert(string.to_owned(), id);
        id
    }

    /// Get the name string associated with a given [`NameId`].
    pub fn name(&self, id: NameId) -> Option<&String> {
        self.strings.get(id.0)
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

    /// Maps all names used throughout the type checking
    /// process to unique name IDs to improve performance.
    /// They are cheap to copy and hash, etc.
    names: NameInterner,

    /// The symbol table. Contains all symbols within the
    /// program. It is a generational arena where a
    /// [`SymbolId`] is a unique handle to a [`Symbol`].
    symbols: SlotMap<SymbolId, Symbol>,

    /// Contains all scopes within the program. It is a
    /// generational arena where a [`ScopeId`] is a unique
    /// handle to a certain [`Scope`].
    scopes: SlotMap<ScopeId, Scope>,

    /// Identifies a unique generic type parameter which appears
    /// somewhere in the program.
    ///
    /// TODO No need for a SlotMap here. GenericIds are never
    /// deleted from the arena, so need for a generational arena.
    generic_ids: SlotMap<GenericId, ()>,

    /// Store and synthesise names for generics. See
    /// [`GenericNames`] for more info.
    generic_names: GenericNames,

    /// A handle to the scope that is currently being checked.
    current_scope: ScopeId,

    /// All the diagnostics that have been accumulated so far.
    diagnostics: Diagnostics,

    /// An index which allows you to query for symbols and types
    /// and other things, based on spans within the source code.
    positions: PositionIndex,

    graph: CallGraph,

    sccc: SCCCollector,

    items_by_symbol: HashMap<SymbolId, &'ast Item>,

    current_fn: Option<SymbolId>,

    checking_stack: Vec<SymbolId>,

    inference_vars: Vec<(VarId, Span)>,
}

impl<'ast> Default for TypeCheckContext<'ast> {
    fn default() -> Self {
        Self::new()
    }
}

impl<'ast> TypeCheckContext<'ast> {
    /// Create a new blank [`TypeCheckContext`].
    fn new() -> Self {
        let mut scopes = SlotMap::with_key();
        let root = scopes.insert(Scope {
            parent: None,
            types: HashMap::new(),
            values: HashMap::new(),
        });

        Self {
            inf: InferenceTable::new(),

            names: NameInterner::new(),
            scopes,
            generic_ids: SlotMap::with_key(),
            generic_names: GenericNames::new(),
            inference_vars: Vec::new(),
            symbols: SlotMap::with_key(),
            current_scope: root,
            diagnostics: Diagnostics::default(),
            positions: PositionIndex::default(),
            graph: CallGraph::new(),
            sccc: SCCCollector::new(),
            items_by_symbol: HashMap::new(),
            current_fn: None,
            checking_stack: Vec::new(),
        }
    }

    fn with_scope<T>(&mut self, scope: ScopeId, f: impl FnOnce(&mut Self) -> T) -> T {
        let parent = self.current_scope;
        self.current_scope = scope;
        let result = f(self);
        self.current_scope = parent;
        result
    }

    fn symbol(&self, symbol: SymbolId) -> &Symbol {
        &self.symbols[symbol]
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
    fn record_path_reference(&mut self, path: &Path, symbol: SymbolId) {
        if let Some(segment) = path.segments.last() {
            self.positions.record_symbol(segment.ident.span, symbol);
        }
    }

    #[cfg(test)]
    pub(crate) fn symbol_at(&self, offset: usize) -> Option<SymbolId> {
        self.positions.symbol_at(offset)
    }

    #[cfg(test)]
    pub(crate) fn type_name_at(&self, offset: usize) -> Option<&'static str> {
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
    /// It walks the AST, and creates a new symbol in the symbol
    /// table for each item.
    ///
    /// For example, say one of the items is a `function`
    /// ```ignore
    /// fn add_int<T>(a: T, b: int) {
    ///     a + b
    /// }
    /// ```
    /// Then a new [`Symbol`] is created within the symbol table
    /// for it, with [`SymbolKind::Fn`] kind. Note that in this
    /// stage *does* create a [`Scope`] for the function body,
    /// and declares symbols for the generic type parameters,
    /// in this case just `T`. However, this stage does *not*
    /// try to determine the type of the new symbol for the
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
    /// In the previous `resolve` stage, it created new symbols
    /// for all of the items within the AST. However, it only
    /// assigns a fresh inference variable as the type of each
    /// symbol.
    ///
    /// This stage uses information in the AST about types of
    /// the symbols and lowers those types into tys. It does
    /// this for all applicable kinds of items.
    ///
    /// For example, say one of the items is a function
    /// ```ignore
    /// fn add_int<T>(a: T, b: int) {
    ///     a + b
    /// }
    /// ```
    /// In the previous resolution stage, a new [`Symbol`] of
    /// kind [`SymbolKind::Fn`] would have been created inside
    /// the symbol table. Now, a new `ty` is created for the
    /// type of this `add_int` function, based on the type
    /// annotations in the signature only. So the [`Symbol`]
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
    /// symbol table where functions all have types matching
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
    /// The symbol for `add_int` would have had its type
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

    /// Returns a handle to the [`Symbol`] which the given path
    /// references, if it exists in the given namespace.
    ///
    /// Searches for the symbol represented by the first
    /// segment in the path recursively up the chain of
    /// enclosed scopes. If it finds it, it then continues
    /// recursively resolving the shortened path until
    /// it potentially arrives at a symbol.
    fn resolve_path(&mut self, path: &Path, namespace: Namespace) -> Option<SymbolId> {
        let last = path.segments.len() - 1;

        let mut segments = path.segments.iter().enumerate();
        let (i, first) = segments.next()?;
        let name = self.names.id(&first.ident.name);
        let first_symbol = self.lookup_up_scope_chain(
            self.current_scope,
            name,
            segment_namespace(i, last, namespace),
        )?;

        segments.try_fold(first_symbol, |symbol, (i, segment)| {
            let scope = self.mod_symbol_scope(symbol)?;
            let name = self.names.id(&segment.ident.name);
            self.lookup_in_scope(scope, name, segment_namespace(i, last, namespace))
        })
    }

    fn resolve_path_to_type(&mut self, path: &Path) -> Option<SymbolId> {
        self.resolve_path(path, Namespace::Type)
    }

    fn resolve_path_to_value(&mut self, path: &Path) -> Option<SymbolId> {
        self.resolve_path(path, Namespace::Value)
    }

    fn resolve_path_from(
        &mut self,
        root: SymbolId,
        path: &Path,
        namespace: Namespace,
    ) -> Option<SymbolId> {
        let last = path.segments.len() - 1;
        path.segments
            .iter()
            .enumerate()
            .try_fold(root, |symbol, (i, segment)| {
                let scope = self.mod_symbol_scope(symbol)?;
                let name = self.names.id(&segment.ident.name);
                self.lookup_in_scope(scope, name, segment_namespace(i, last, namespace))
            })
    }

    /// Directly checks if the given scope contains a symbol
    /// with a given name, which belongs to a certain namespace.
    fn lookup_in_scope(
        &self,
        scope: ScopeId,
        name: NameId,
        namespace: Namespace,
    ) -> Option<SymbolId> {
        let map = match namespace {
            Namespace::Type => &self.scopes[scope].types,
            Namespace::Value => &self.scopes[scope].values,
        };
        map.get(&name).copied()
    }

    fn with_symbol_in_scope<T>(
        &mut self,
        name: NameId,
        namespace: Namespace,
        f: impl FnOnce(&mut Self, SymbolId) -> T,
    ) -> Option<T> {
        self.lookup_in_scope(self.current_scope, name, namespace)
            .map(|symbol| f(self, symbol))
    }

    fn with_type_symbol<T>(
        &mut self,
        name: NameId,
        f: impl FnOnce(&mut Self, SymbolId) -> T,
    ) -> Option<T> {
        self.with_symbol_in_scope(name, Namespace::Type, f)
    }

    fn with_value_symbol<T>(
        &mut self,
        name: NameId,
        f: impl FnOnce(&mut Self, SymbolId) -> T,
    ) -> Option<T> {
        self.with_symbol_in_scope(name, Namespace::Value, f)
    }

    fn fn_symbol_scope(&self, symbol: SymbolId) -> Option<ScopeId> {
        let SymbolKind::Fn(fn_data) = &self.symbol(symbol).kind else {
            return None;
        };
        Some(fn_data.scope)
    }

    fn mod_symbol_scope(&self, symbol: SymbolId) -> Option<ScopeId> {
        let SymbolKind::Mod(scope) = self.symbol(symbol).kind else {
            return None;
        };
        Some(scope)
    }

    fn ty_alias_symbol_scope(&self, symbol: SymbolId) -> Option<ScopeId> {
        let SymbolKind::TyAlias(alias_data) = &self.symbol(symbol).kind else {
            return None;
        };
        Some(alias_data.scope)
    }

    fn with_fn_symbol<T>(
        &mut self,
        name: NameId,
        f: impl FnOnce(&mut Self, SymbolId, ScopeId) -> T,
    ) -> Option<T> {
        self.with_value_symbol(name, |this, symbol| {
            this.fn_symbol_scope(symbol)
                .map(|scope| f(this, symbol, scope))
        })
        .flatten()
    }

    fn with_fn_scope<T>(
        &mut self,
        name: NameId,
        f: impl FnOnce(&mut Self, SymbolId) -> T,
    ) -> Option<T> {
        self.with_fn_symbol(name, |this, symbol, scope| {
            this.with_scope(scope, |this| f(this, symbol))
        })
    }

    fn with_mod_symbol<T>(
        &mut self,
        name: NameId,
        f: impl FnOnce(&mut Self, SymbolId, ScopeId) -> T,
    ) -> Option<T> {
        self.with_type_symbol(name, |this, symbol| {
            this.mod_symbol_scope(symbol)
                .map(|scope| f(this, symbol, scope))
        })
        .flatten()
    }

    fn with_mod_scope<T>(
        &mut self,
        name: NameId,
        f: impl FnOnce(&mut Self, SymbolId) -> T,
    ) -> Option<T> {
        self.with_mod_symbol(name, |this, symbol, scope| {
            this.with_scope(scope, |this| f(this, symbol))
        })
    }

    fn with_ty_alias_symbol<T>(
        &mut self,
        name: NameId,
        f: impl FnOnce(&mut Self, SymbolId, ScopeId) -> T,
    ) -> Option<T> {
        self.with_type_symbol(name, |this, symbol| {
            this.ty_alias_symbol_scope(symbol)
                .map(|scope| f(this, symbol, scope))
        })
        .flatten()
    }

    #[allow(unused)]
    fn with_ty_alias_scope<T>(
        &mut self,
        name: NameId,
        f: impl FnOnce(&mut Self, SymbolId) -> T,
    ) -> Option<T> {
        self.with_ty_alias_symbol(name, |this, symbol, scope| {
            this.with_scope(scope, |this| f(this, symbol))
        })
    }

    /// Recursively searches the current scope and enclosing
    /// scopes for a [`Symbol`] with a given name and which
    /// belongs to the specified namespace.
    fn lookup_up_scope_chain(
        &self,
        scope: ScopeId,
        name: NameId,
        namespace: Namespace,
    ) -> Option<SymbolId> {
        std::iter::successors(Some(scope), |&scope| self.scopes[scope].parent)
            .find_map(|scope| self.lookup_in_scope(scope, name, namespace))
    }

    /// Declares a new [`Symbol`] in a scope of a certain kind.
    ///
    /// Any ty this kind of symbol needs must already be embedded
    /// in `kind` by the caller (see e.g. [`PatDeclKind::symbol_kind`],
    /// which requires a fresh inference variable `?a` be created
    /// up front to represent the type of the symbol until something
    /// else later constrains it).
    fn declare(&mut self, name: &str, span: Span, kind: SymbolKind) -> SymbolId {
        let namespace = kind.namespace();
        let name = self.names.id(name);

        // `let` bindings (and, transitively through them, their
        // shadowing sub-patterns) are always allowed to shadow
        // whatever previously held the same name in this scope.
        // Everything else -- functions, modules, structs, enums,
        // traits, type aliases, and parameters -- must be uniquely
        // named within a scope.
        if !matches!(kind, SymbolKind::Local(_)) {
            self.check_redeclaration(namespace, name, span);
        }

        let symbol = self.symbols.insert(Symbol {
            name,
            kind,
            declared_at: span,
        });
        self.insert_in_scope(name, symbol, namespace);
        self.positions.record_symbol(span, symbol);
        symbol
    }

    /// If a symbol already exists with the given name in the given
    /// namespace of the current scope, emits an [`AlreadyDefined`]
    /// diagnostic pointing back at its original declaration.
    fn check_redeclaration(&mut self, namespace: Namespace, name: NameId, span: Span) {
        self.lookup_in_scope(self.current_scope, name, namespace)
            .into_iter()
            .for_each(|existing| {
                let original = self.symbol(existing).declared_at;
                let name = self
                    .names
                    .name(name)
                    .cloned()
                    .unwrap_or_else(|| "_".to_owned());
                self.diagnostics
                    .push(AlreadyDefined::new(span, name, original));
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
    /// body is entered, and a new [`SymbolKind::GenericParam`]
    /// is created for `T` inside that scope so `T` becomes
    /// a valid type that can be used within the function body.
    fn declare_generic_param(&mut self, param_name: &str, span: Span) -> (SymbolId, GenericId) {
        let name = self.names.id(param_name);
        let id = self.generic_ids.insert(());
        self.generic_names.declare(id, param_name.to_owned());
        let ty = self.ty(TyKind::Generic(id));
        let symbol = self.symbols.insert(Symbol {
            name,
            kind: SymbolKind::GenericParam(ty),
            declared_at: span,
        });
        self.insert_type_in_scope(name, symbol);
        self.positions.record_symbol(span, symbol);
        (symbol, id)
    }

    fn declare_generic_params(&mut self, params: &[GenericParam]) -> Vec<GenericId> {
        params
            .iter()
            .map(|param| {
                self.declare_generic_param(&param.ident.name, param.ident.span)
                    .1
            })
            .collect()
    }

    /// Inserts a symbol into the current [`Scope`] via its
    /// handle.
    fn insert_in_scope(&mut self, name: NameId, symbol: SymbolId, namespace: Namespace) {
        let scope = &mut self.scopes[self.current_scope];
        match namespace {
            Namespace::Type => scope.types.insert(name, symbol),
            Namespace::Value => scope.values.insert(name, symbol),
        };
    }

    fn insert_type_in_scope(&mut self, name: NameId, symbol: SymbolId) {
        self.insert_in_scope(name, symbol, Namespace::Type);
    }

    fn insert_value_in_scope(&mut self, name: NameId, symbol: SymbolId) {
        self.insert_in_scope(name, symbol, Namespace::Value);
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
                .find(|(name, _)| segment.ident.name == *name);
            if let Some((name, con)) = found {
                self.positions.record_primitive(segment.ident.span, name);
                return self.ty(con.clone());
            }
        }

        self.resolve_path_to_type(path)
            .map(|symbol| {
                self.record_path_reference(path, symbol);
                match &self.symbol(symbol).kind {
                    SymbolKind::Struct => self.ty(TyKind::Struct(symbol)),
                    SymbolKind::Enum => self.ty(TyKind::Enum(symbol)),
                    _ => self.instantiate_path(symbol, path),
                }
            })
            .unwrap_or_else(|| {
                self.diagnostics
                    .push(UnresolvedType::new(path.span, display_path(path)));
                self.ty(TyKind::Err)
            })
    }
}

const PRIMITIVE_TYPES: &[(&str, TyKind)] = &[
    ("bool", TyKind::Bool),
    ("int", TyKind::Int),
    ("float", TyKind::Float),
    ("char", TyKind::Char),
    ("String", TyKind::Str),
    ("any", TyKind::Any),
];

/// The AST Visitor that performs the Resolution stage of the
/// type checking. Walks the AST, creating new symbols in the
/// symbol table for each item it finds.
struct Resolver<'a, 'ast> {
    /// Mutable reference to the underlying TypeCheckContext.
    cx: &'a mut TypeCheckContext<'ast>,
}

impl<'ast> Resolver<'_, 'ast> {
    /// Creates a new [`Scope`] in the underling
    /// [`TypeCheckContext`], and returns a handle to it.
    fn new_scope(&mut self) -> ScopeId {
        let parent = self.cx.current_scope;
        self.cx.scopes.insert(Scope {
            parent: Some(parent),
            types: HashMap::new(),
            values: HashMap::new(),
        })
    }

    /// Enters the given scope, performs some function while
    /// inside that scope, and then exits the scope once the
    /// function is complete.
    ///
    /// This ensures that even an early exit will still
    /// ensure that the current_scope is updated back to
    /// the parent scope.
    fn with_scope(&mut self, scope: ScopeId, f: impl FnOnce(&mut Self)) {
        let parent = self.cx.current_scope;
        self.cx.current_scope = scope;
        f(self);
        self.cx.current_scope = parent;
    }
}

impl Visitor for Resolver<'_, '_> {
    fn visit_item(&mut self, item: &Item) {
        match &item.kind {
            ItemKind::Fn(f) => self.resolve_fn_item(item, f),
            ItemKind::TyAlias(alias) => self.resolve_ty_alias_item(alias),
            ItemKind::Enum(ident, _generics, _def) => self.resolve_enum_item(ident),
            ItemKind::Struct(ident, _generics, data) => self.resolve_struct_item(ident, data),
            ItemKind::Trait(t) => self.resolve_trait_item(t),
            ItemKind::Mod(ident, ModKind::Unloaded) => self.resolve_mod_unloaded_item(ident),
            ItemKind::Mod(ident, ModKind::Loaded(_)) => self.resolve_mod_loaded_item(ident, item),
            ItemKind::Use(_) => self.resolve_use_item(),
            ItemKind::Impl(_) => self.resolve_impl_item(),
        }
    }
}

impl Resolver<'_, '_> {
    fn resolve_fn_item(&mut self, item: &Item, f: &Fn) {
        let scope = self.new_scope();
        let ty = self.cx.fresh_var_at(Some(f.ident.span));
        let fn_symbol = self.cx.declare(
            &f.ident.name,
            f.ident.span,
            SymbolKind::Fn(FnSymbol {
                scope,
                param_spans: Vec::new(),
                param_names: Vec::new(),
                ty,
                generics: Vec::new(),
            }),
        );

        let mut generics = Vec::new();
        self.with_scope(scope, |this| {
            generics = this.cx.declare_generic_params(&f.generics.params);
            item.walk(this);
        });
        if let SymbolKind::Fn(fn_data) = &mut self.cx.symbols[fn_symbol].kind {
            fn_data.generics = generics;
        }
    }

    fn resolve_ty_alias_item(&mut self, alias: &TyAlias) {
        let scope = self.new_scope();
        let ty = self.cx.fresh_var_at(Some(alias.ident.span));
        let alias_symbol = self.cx.declare(
            &alias.ident.name,
            alias.ident.span,
            SymbolKind::TyAlias(TyAliasSymbol {
                scope,
                ty,
                generics: Vec::new(),
            }),
        );

        let mut generics = Vec::new();
        self.with_scope(scope, |this| {
            generics = this.cx.declare_generic_params(&alias.generics.params);
        });
        if let SymbolKind::TyAlias(alias_data) = &mut self.cx.symbols[alias_symbol].kind {
            alias_data.generics = generics;
        }
    }

    fn resolve_enum_item(&mut self, ident: &Ident) {
        self.cx.declare(&ident.name, ident.span, SymbolKind::Enum);
    }

    fn resolve_struct_item(&mut self, ident: &Ident, data: &VariantData) {
        let symbol = self.cx.declare(&ident.name, ident.span, SymbolKind::Struct);
        if !matches!(data, VariantData::Struct(_)) {
            let name = self.cx.names.id(&ident.name);
            self.cx
                .check_redeclaration(Namespace::Value, name, ident.span);
            self.cx.insert_value_in_scope(name, symbol);
        }
    }

    fn resolve_trait_item(&mut self, t: &Trait) {
        self.cx
            .declare(&t.ident.name, t.ident.span, SymbolKind::Trait);
    }

    fn resolve_mod_unloaded_item(&mut self, ident: &Ident) {
        let scope = self.new_scope();
        self.cx
            .declare(&ident.name, ident.span, SymbolKind::Mod(scope));
    }

    fn resolve_mod_loaded_item(&mut self, ident: &Ident, item: &Item) {
        let scope = self.new_scope();
        self.cx
            .declare(&ident.name, ident.span, SymbolKind::Mod(scope));
        self.with_scope(scope, |this| item.walk(this));
    }

    fn resolve_use_item(&mut self) {}

    fn resolve_impl_item(&mut self) {}
}

/// Performs the signature lowering stage of the type checking.
/// Fills in the types of the symbols created by the [`Resolver`]
/// where possible, for example for functions and type aliases.
struct SignatureLowerer<'a, 'ast> {
    /// A mutable reference to the underlying TypeCheckContext.
    cx: &'a mut TypeCheckContext<'ast>,
}

impl SignatureLowerer<'_, '_> {
    /// Enters the given scope, performs some function while
    /// inside that scope, and then exits the scope once the
    /// function is complete.
    ///
    /// This ensures that even an early exit will still
    /// ensure that the current_scope is updated back to
    /// the parent scope.
    fn with_scope(&mut self, scope: ScopeId, f: impl FnOnce(&mut Self)) {
        let parent = self.cx.current_scope;
        self.cx.current_scope = scope;
        f(self);
        self.cx.current_scope = parent;
    }

    /// Creates a ty representing the type of a function
    /// based on the explicit type annotations within its
    /// signature.
    fn lower_fn_sig(&mut self, f: &Fn) -> TyId {
        let inputs = f
            .sig
            .inputs
            .iter()
            .map(|param| match &param.ty {
                Some(ty) => self.cx.lower_ty(ty),
                None => self.cx.fresh_var(),
            })
            .collect();
        let output_ty = self.cx.lower_ret_ty(&f.sig.output, None);
        self.cx.ty(TyKind::Fn(inputs, output_ty))
    }
}

impl SignatureLowerer<'_, '_> {
    fn lower_fn_item(&mut self, item: &Item, f: &Fn) {
        let name = self.cx.names.id(&f.ident.name);
        let Some((symbol, scope)) = self
            .cx
            .with_fn_symbol(name, |_, symbol, scope| (symbol, scope))
        else {
            return;
        };

        self.with_scope(scope, |this| {
            let fn_ty = this.lower_fn_sig(f);
            let symbol_ty = this.cx.symbol(symbol).ty();
            // Unifies the fresh placeholder inference variable which
            // was created during the previous Resolution stage with the
            // ty created by lowering the function signature.
            let _ = this.cx.inf.unify(symbol_ty, fn_ty);

            // Collect information about parameter names and spans.
            if let SymbolKind::Fn(fn_data) = &mut this.cx.symbols[symbol].kind {
                fn_data.param_spans = f
                    .sig
                    .inputs
                    .iter()
                    .map(|p| p.ty.as_ref().map(|ty| ty.span))
                    .collect();
                fn_data.param_names = f
                    .sig
                    .inputs
                    .iter()
                    .map(|p| types::pat_display_name(&p.pat))
                    .collect();
            }
            item.walk(this);
        });
    }

    /// Resolves a `use` path, optionally rooted at an already
    /// resolved parent module (for paths nested inside a
    /// `use foo::{ ... }` group).
    fn resolve_use_path(
        &mut self,
        prefix: Option<SymbolId>,
        path: &Path,
        namespace: Namespace,
    ) -> Option<SymbolId> {
        match prefix {
            Some(pid) => self.cx.resolve_path_from(pid, path, namespace),
            None => self.cx.resolve_path(path, namespace),
        }
    }

    fn resolve_use_path_to_type(
        &mut self,
        prefix: Option<SymbolId>,
        path: &Path,
    ) -> Option<SymbolId> {
        self.resolve_use_path(prefix, path, Namespace::Type)
    }

    fn resolve_use_path_to_value(
        &mut self,
        prefix: Option<SymbolId>,
        path: &Path,
    ) -> Option<SymbolId> {
        self.resolve_use_path(prefix, path, Namespace::Value)
    }

    fn lower_use_tree(&mut self, tree: &UseTree, prefix: Option<SymbolId>) {
        let mut sid = self.resolve_use_path_to_type(prefix, &tree.prefix);
        if sid.is_none() && matches!(tree.kind, UseTreeKind::Simple(_)) {
            sid = self.resolve_use_path_to_value(prefix, &tree.prefix);
        }
        let Some(sid) = sid else {
            self.cx.diagnostics.push(UnresolvedImport::new(
                tree.prefix.span,
                display_path(&tree.prefix),
            ));
            return;
        };

        self.cx.record_path_reference(&tree.prefix, sid);

        match &tree.kind {
            UseTreeKind::Simple(ident) => self.lower_use_tree_simple(tree, sid, ident),
            UseTreeKind::Glob(span) => self.lower_use_tree_glob(tree, sid, *span),
            UseTreeKind::Nested { items, .. } => self.lower_use_tree_nested(items, sid),
        }
    }

    fn lower_use_tree_simple(&mut self, tree: &UseTree, sid: SymbolId, ident: &Option<Ident>) {
        let Some(ident) = ident
            .as_ref()
            .or(tree.prefix.segments.last().map(|seg| &seg.ident))
        else {
            unreachable!("A path should always have a valid name");
        };
        let name = self.cx.names.id(&ident.name);
        let namespace = self.cx.symbols[sid].kind.namespace();
        self.cx.insert_in_scope(name, sid, namespace);
    }

    fn lower_use_tree_glob(&mut self, tree: &UseTree, sid: SymbolId, span: Span) {
        let Some(scope) = self.cx.mod_symbol_scope(sid) else {
            self.cx.diagnostics.push(InvalidGlobTarget::new(
                span,
                display_path(&tree.prefix),
                self.cx.symbols[sid].kind.describe().to_string(),
            ));
            return;
        };
        self.cx.scopes[scope]
            .types
            .clone()
            .into_iter()
            .for_each(|(name, sid)| self.cx.insert_type_in_scope(name, sid));
        self.cx.scopes[scope]
            .values
            .clone()
            .into_iter()
            .for_each(|(name, sid)| self.cx.insert_value_in_scope(name, sid));
    }

    fn lower_use_tree_nested(&mut self, items: &[UseTree], sid: SymbolId) {
        items
            .iter()
            .for_each(|item| self.lower_use_tree(item, Some(sid)));
    }

    fn lower_ty_alias_item(&mut self, alias: &TyAlias) {
        let name = self.cx.names.id(&alias.ident.name);
        let resolved = self
            .cx
            .with_ty_alias_symbol(name, |_, symbol, scope| (symbol, scope));

        if let (Some((symbol, scope)), Some(ty)) = (resolved, alias.ty.as_ref()) {
            self.with_scope(scope, |this| {
                let aliased = this.cx.lower_ty(ty);
                let symbol_ty = this.cx.symbol(symbol).ty();
                // Unifies the fresh placeholder inference variable which
                // was created during the previous Resolution stage with the
                // ty created by lowering the type of the expression being
                // aliased. A type alias can never refer to itself, directly
                // or indirectly (e.g. `type Foo = (Foo, int);`), since that
                // would make it an infinitely-sized type.
                this.cx.unify_or_report_cycle(symbol_ty, aliased, ty.span);
            });
        }
    }

    fn lower_mod_item(&mut self, name: &Ident, _kind: &ModKind, item: &Item) {
        let name = self.cx.names.id(&name.name);
        let scope = self.cx.with_mod_symbol(name, |_, _, scope| scope);
        scope
            .into_iter()
            .for_each(|scope| self.with_scope(scope, |this| item.walk(this)));
    }
}

fn display_path(path: &Path) -> String {
    path.segments
        .iter()
        .map(|segment| segment.ident.name.as_str())
        .collect::<Vec<_>>()
        .join("::")
}

impl Visitor for SignatureLowerer<'_, '_> {
    fn visit_item(&mut self, item: &Item) {
        match &item.kind {
            ItemKind::Fn(f) => self.lower_fn_item(item, f),
            ItemKind::TyAlias(alias) => self.lower_ty_alias_item(alias),
            ItemKind::Mod(ident, kind) => self.lower_mod_item(ident, kind, item),
            ItemKind::Use(tree) => self.lower_use_tree(tree, None),
            _ => {}
        }
    }
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
