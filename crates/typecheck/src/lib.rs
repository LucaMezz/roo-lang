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
use ast::{Fn, FnRetTy, FnTy, Item, ItemKind, ModKind, Path, Span, Ty, TyKind, VariantData};
use slotmap::SlotMap;
use unify::{TermId, UnificationContext, VarId, term};

mod call_graph;
mod check;
mod checked_program;
mod errors;
mod generic_names;
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
use crate::errors::AnnotationsNeeded;

/// The enum containing all the possible constructors which can
/// appear within terms. Specifically, a `Term` represents a type
/// which may contain unknowns, i.e. unbound inference variables
/// like `?a`. A `Term` is defined recursively, as
///
/// ```text
///     t ::= a  |  f(t_1, t_2, ..., t_n)
/// ```
///
/// for some natural n >= 0, where `f` is a constructor, and
/// t_1,...,t_n are themselves terms. Call t_1,...,t_n the
/// `arguments`.
///
/// For example, the `Term` which represents the `Int` type would
/// be the [`TyCon::Int`] constructor, applied to zero arguments.
#[derive(Debug, Clone, PartialEq)]
enum TyCon {
    // ===============
    // Primitive Types
    // ===============
    // Always take 0 arguments.
    //
    /// The `<error>` type. Used to indicate there has been some
    /// kind of type error.
    Err,

    /// The `any` type. Used to opt out of static type checking
    /// in favour of runtime type checking.
    Any,

    /// The Never `!` type. Indicates that no value is ever produced.
    /// Directly equivalent to the Never `!` type from Rust.
    Never,

    /// The `int` type. An integer value.
    Int,

    /// The `float` type. A floating point value.
    Float,

    /// The `bool` type. Either `true` or `false`.
    Bool,

    /// The `char` type. A character.
    Char,

    /// The `String` type. A string of characters of arbitrary
    /// length.
    Str,

    /// The `Fn` type. Always takes exactly two arguments.
    /// The first is a Tuple term containing terms representing types
    /// of all the parameters of the function, and the second being
    /// a term representing the return type of the function.
    Fn,

    /// The Array `[T]` type. Always takes exactly one argument,
    /// which is a term representing the type of the elements
    /// held by the array.
    Array,

    /// The Tuple `(T, U, ...)` type. Takes any finite number of
    /// arguments, where the ith argument is a term representing
    /// the type of the ith position in the tuple.
    Tuple,

    /// A Struct. Always takes zero arguments. Note that this
    /// constructor always referes to some specific named struct.
    /// This ensures nominal typing. Two structs are only ever
    /// equal if they are actually the same exact named struct, not
    /// just if they have the same shape.
    Struct(SymbolId),

    /// An Enum. Always takes zero arguments. Note that this
    /// constructor always referes to some specific named enum.
    /// This ensures nominal typing. Two enums are only ever
    /// equal if they are actually the same exact named enum, not
    /// just if they have the same shape.
    Enum(SymbolId),

    /// A generic type parameter. Always takes zero arguments.
    /// Two generics are only ever equal if they refer to the
    /// exact same named generic.
    Generic(GenericId),
}

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
#[derive(Copy, Clone, Eq, PartialEq, Hash)]
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
struct Symbol {
    /// An interned string which is the name of the symbol.
    name: NameId,

    /// The specific kind of symbol that it is.
    kind: SymbolKind,

    /// The term representing the type associated with this
    /// symbol.
    ty: TermId,

    /// The generic parameters associated with this symbol.
    generics: Vec<GenericId>,

    /// The span within the source code that resulted in
    /// the introduction of this symbol.
    declared_at: Span,
}

/// Extra information about a function symbol.
struct FnSymbol {
    /// A handle to the scope of the function body.
    scope: ScopeId,

    /// The span of each of the parameters of the function
    /// within the source code.
    param_spans: Vec<Option<Span>>,

    /// The name of each of the parameters as they appear
    /// in the source code.
    param_names: Vec<String>,
}

/// The specific kind of [`Symbol`].
enum SymbolKind {
    Struct,
    Enum,
    Variant,
    Trait,
    /// A type alias. Type aliases need their own scope
    /// because they can have generic type parameters which
    /// should only exist during the evaluation of the
    /// type on the right hand side of the alias.
    TyAlias(ScopeId),
    /// A module. Here, the [`ScopeId`] is a handle to the
    /// scope of the module body.
    Mod(ScopeId),
    Fn(FnSymbol),
    Local,
    Param,
    GenericParam,
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
    fn symbol_kind(self) -> SymbolKind {
        match self {
            PatDeclKind::Param => SymbolKind::Param,
            PatDeclKind::Let => SymbolKind::Local,
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
            | SymbolKind::GenericParam
            | SymbolKind::Mod(_) => Namespace::Type,
            SymbolKind::Variant | SymbolKind::Fn(_) | SymbolKind::Local | SymbolKind::Param => {
                Namespace::Value
            }
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
    /// O(1) unification of two terms, performing the required
    /// substitutions to ensure two terms are equal. Also
    /// stores a [`Span`] with the reason two terms were
    /// unified.
    uni_cx: UnificationContext<TyCon, Span>,

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
            uni_cx: UnificationContext::with_wildcards(vec![TyCon::Any, TyCon::Err, TyCon::Never]),

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

    fn with_scope(&mut self, scope: ScopeId, f: impl FnOnce(&mut Self)) {
        let parent = self.current_scope;
        self.current_scope = scope;
        f(self);
        self.current_scope = parent;
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

    fn fresh_var(&mut self, span: Span) -> TermId {
        let var = self.uni_cx.fresh_var();
        let term = self.term_var(var);
        self.inference_vars.push((var, span));
        term
    }

    fn term(&mut self, con: TyCon) -> TermId {
        term!(self.uni_cx, con)
    }

    fn term_app(&mut self, con: TyCon, args: Vec<TermId>) -> TermId {
        term!(self.uni_cx, con => args)
    }

    fn term_var(&mut self, id: VarId) -> TermId {
        term!(self.uni_cx, var id)
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
    /// the symbols and lowers those types into Terms. It does
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
    /// the symbol table. Now, a new `Term` is created for the
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
    /// the body of all functions. It also unifies the term
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
    /// inference variables must bind to the same term.
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
        for item in items {
            match &item.kind {
                ItemKind::Fn(_) => self.check_function(item),
                ItemKind::Mod(_, _) => self.check_module(item),
                _ => {}
            }
        }
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
            .filter(|(v, _)| self.uni_cx.binding(*v).is_none())
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
        let mut symbol = self.lookup_up_scope_chain(
            self.current_scope,
            name,
            segment_namespace(i, last, namespace),
        )?;

        for (i, segment) in segments {
            let SymbolKind::Mod(scope) = self.symbols[symbol].kind else {
                return None;
            };
            let name = self.names.id(&segment.ident.name);
            symbol = self.lookup_in_scope(scope, name, segment_namespace(i, last, namespace))?;
        }

        Some(symbol)
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

    /// Recursively searches the current scope and enclosing
    /// scopes for a [`Symbol`] with a given name and which
    /// belongs to the specified namespace.
    fn lookup_up_scope_chain(
        &self,
        mut scope: ScopeId,
        name: NameId,
        namespace: Namespace,
    ) -> Option<SymbolId> {
        loop {
            if let Some(symbol) = self.lookup_in_scope(scope, name, namespace) {
                return Some(symbol);
            }
            scope = self.scopes[scope].parent?;
        }
    }

    /// Declares a new [`Symbol`] in a scope of a certain kind.
    ///
    /// Note that a fresh inference variable `?a` is created to
    /// represent the type of this symbol until something else
    /// later constrains it.
    fn declare(&mut self, name: &str, span: Span, kind: SymbolKind) -> SymbolId {
        let namespace = kind.namespace();
        let name = self.names.id(name);
        let ty = self.fresh_var(span);
        let symbol = self.symbols.insert(Symbol {
            name,
            kind,
            ty,
            generics: Vec::new(),
            declared_at: span,
        });
        self.insert_in_scope(name, symbol, namespace);
        self.positions.record_symbol(span, symbol);
        symbol
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
        let ty = self.term(TyCon::Generic(id));
        let symbol = self.symbols.insert(Symbol {
            name,
            kind: SymbolKind::GenericParam,
            ty,
            generics: Vec::new(),
            declared_at: span,
        });
        self.insert_in_scope(name, symbol, Namespace::Type);
        self.positions.record_symbol(span, symbol);
        (symbol, id)
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

    /// Convert a [`Ty`] AST node into a Term which represents
    /// that type and which can actually be used by the type
    /// checker to perform checking and inference.
    fn lower_ty(&mut self, ty: &Ty) -> TermId {
        match &ty.kind {
            TyKind::Never => self.term(TyCon::Never),
            TyKind::Paren(inner) => self.lower_ty(inner),
            TyKind::Array(inner) => {
                let elem = self.lower_ty(inner);
                self.term_app(TyCon::Array, vec![elem])
            }
            TyKind::Tup(inner) => {
                let args = inner.iter().map(|x| self.lower_ty(x)).collect();
                self.term_app(TyCon::Tuple, args)
            }
            TyKind::Fn(fn_ty) => {
                let FnTy { inputs, output } = fn_ty.as_ref();
                let input_args = inputs.iter().map(|x| self.lower_ty(x)).collect();
                let inputs_term = self.term_app(TyCon::Tuple, input_args);
                let output_term = match output {
                    // When a function doesn't annotate a return type,
                    // inroduce a fresh inference variable `?a` to
                    // represent its type.
                    FnRetTy::Default(_) => self.fresh_var(ty.span),
                    FnRetTy::Ty(ty) => self.lower_ty(ty),
                };
                self.term_app(TyCon::Fn, vec![inputs_term, output_term])
            }
            TyKind::Path(path) => self.lower_path_ty(path),
            TyKind::ImplicitSelf => unimplemented!(),
            // When `_` is used as a type annotation, it means
            // the type should be inferred. Hence, introduce
            // a fresh inference variable `?a` to represent
            // this type.
            TyKind::Infer => self.fresh_var(ty.span),
            TyKind::Err => self.term(TyCon::Err),
        }
    }

    fn lower_path_ty(&mut self, path: &Path) -> TermId {
        if let [segment] = path.segments.as_slice() {
            let found = PRIMITIVE_TYPES
                .iter()
                .find(|(name, _)| segment.ident.name == *name);
            if let Some((name, con)) = found {
                self.positions.record_primitive(segment.ident.span, name);
                return self.term(con.clone());
            }
        }

        match self.resolve_path(path, Namespace::Type) {
            Some(symbol) => {
                self.record_path_reference(path, symbol);
                match &self.symbols[symbol].kind {
                    SymbolKind::Struct => self.term(TyCon::Struct(symbol)),
                    SymbolKind::Enum => self.term(TyCon::Enum(symbol)),
                    _ => self.instantiate_path(symbol, path),
                }
            }
            None => self.term(TyCon::Err),
        }
    }
}

const PRIMITIVE_TYPES: &[(&str, TyCon)] = &[
    ("bool", TyCon::Bool),
    ("int", TyCon::Int),
    ("float", TyCon::Float),
    ("char", TyCon::Char),
    ("String", TyCon::Str),
    ("any", TyCon::Any),
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
            ItemKind::Fn(f) => {
                let scope = self.new_scope();
                let fn_symbol = self.cx.declare(
                    &f.ident.name,
                    f.ident.span,
                    SymbolKind::Fn(FnSymbol {
                        scope,
                        param_spans: Vec::new(),
                        param_names: Vec::new(),
                    }),
                );

                let mut generics = Vec::new();
                self.with_scope(scope, |this| {
                    for param in &f.generics.params {
                        let (_, id) = this
                            .cx
                            .declare_generic_param(&param.ident.name, param.ident.span);
                        generics.push(id);
                    }
                    item.walk(this);
                });
                self.cx.symbols[fn_symbol].generics = generics;
            }
            ItemKind::TyAlias(alias) => {
                let scope = self.new_scope();
                let alias_symbol = self.cx.declare(
                    &alias.ident.name,
                    alias.ident.span,
                    SymbolKind::TyAlias(scope),
                );

                let mut generics = Vec::new();
                self.with_scope(scope, |this| {
                    for param in &alias.generics.params {
                        let (_, id) = this
                            .cx
                            .declare_generic_param(&param.ident.name, param.ident.span);
                        generics.push(id);
                    }
                });
                self.cx.symbols[alias_symbol].generics = generics;
            }
            ItemKind::Enum(ident, _generics, _def) => {
                self.cx.declare(&ident.name, ident.span, SymbolKind::Enum);
            }
            ItemKind::Struct(ident, _generics, data) => {
                let symbol = self.cx.declare(&ident.name, ident.span, SymbolKind::Struct);
                if !matches!(data, VariantData::Struct(_)) {
                    let name = self.cx.names.id(&ident.name);
                    self.cx.insert_in_scope(name, symbol, Namespace::Value);
                }
            }
            ItemKind::Trait(t) => {
                self.cx
                    .declare(&t.ident.name, t.ident.span, SymbolKind::Trait);
            }
            ItemKind::Mod(ident, ModKind::Unloaded) => {
                let scope = self.new_scope();
                self.cx
                    .declare(&ident.name, ident.span, SymbolKind::Mod(scope));
            }
            ItemKind::Mod(ident, ModKind::Loaded(_)) => {
                let scope = self.new_scope();
                self.cx
                    .declare(&ident.name, ident.span, SymbolKind::Mod(scope));
                self.with_scope(scope, |this| item.walk(this));
            }
            ItemKind::Use(_) | ItemKind::Impl(_) => {}
        }
    }
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

    /// Creates a term representing the type of a function
    /// based on the explicit type annotations within its
    /// signature.
    fn lower_fn_sig(&mut self, f: &Fn) -> TermId {
        let inputs = f
            .sig
            .inputs
            .iter()
            .map(|param| match &param.ty {
                Some(ty) => self.cx.lower_ty(ty),
                None => {
                    let var = self.cx.uni_cx.fresh_var();
                    self.cx.term_var(var)
                }
            })
            .collect();
        let inputs_term = self.cx.term_app(TyCon::Tuple, inputs);
        let output_term = match &f.sig.output {
            FnRetTy::Default(_) => {
                let var = self.cx.uni_cx.fresh_var();
                self.cx.term_var(var)
            }
            FnRetTy::Ty(ty) => self.cx.lower_ty(ty),
        };
        self.cx.term_app(TyCon::Fn, vec![inputs_term, output_term])
    }
}

impl SignatureLowerer<'_, '_> {
    fn lower_fn_item(&mut self, item: &Item, f: &Fn) {
        let name = self.cx.names.id(&f.ident.name);
        let Some(symbol) = self
            .cx
            .lookup_in_scope(self.cx.current_scope, name, Namespace::Value)
        else {
            return;
        };
        let SymbolKind::Fn(fn_data) = &self.cx.symbols[symbol].kind else {
            return;
        };
        let scope = fn_data.scope;

        self.with_scope(scope, |this| {
            let fn_term = this.lower_fn_sig(f);
            let symbol_ty = this.cx.symbols[symbol].ty;
            // Unifies the fresh placeholder inference variable which
            // was created during the previous Resolution stage with the
            // term created by lowering the function signature.
            let _ = this.cx.uni_cx.unify(symbol_ty, fn_term);

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
}

impl Visitor for SignatureLowerer<'_, '_> {
    fn visit_item(&mut self, item: &Item) {
        match &item.kind {
            ItemKind::Fn(f) => self.lower_fn_item(item, f),
            ItemKind::TyAlias(alias) => {
                let name = self.cx.names.id(&alias.ident.name);
                let symbol = self
                    .cx
                    .lookup_in_scope(self.cx.current_scope, name, Namespace::Type);

                let scope = symbol.and_then(|symbol| match &self.cx.symbols[symbol].kind {
                    SymbolKind::TyAlias(scope) => Some(*scope),
                    _ => None,
                });
                if let (Some(symbol), Some(scope), Some(ty)) = (symbol, scope, alias.ty.as_ref()) {
                    self.with_scope(scope, |this| {
                        let aliased = this.cx.lower_ty(ty);
                        let symbol_ty = this.cx.symbols[symbol].ty;
                        // Unifies the fresh placeholder inference variable which
                        // was created during the previous Resolution stage with the
                        // term created by lowering the type of the expression being
                        // aliased.
                        let _ = this.cx.uni_cx.unify(symbol_ty, aliased);
                    });
                }
            }
            ItemKind::Mod(ident, ModKind::Loaded(_)) => {
                let name = self.cx.names.id(&ident.name);
                let scope = self
                    .cx
                    .lookup_in_scope(self.cx.current_scope, name, Namespace::Type)
                    .and_then(|symbol| match &self.cx.symbols[symbol].kind {
                        SymbolKind::Mod(scope) => Some(*scope),
                        _ => None,
                    });
                if let Some(scope) = scope {
                    self.with_scope(scope, |this| item.walk(this));
                }
            }
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
