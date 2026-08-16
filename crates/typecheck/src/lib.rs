//! `typecheck` is roo-lang's type checker.
//!
//! Combines two things: the concrete type representation (`TyCon`, the
//! `C` that plugs into [`unify::Term`]) and the actual checking logic
//! that uses it -- lowering `ast::Ty`/`ast::ExprKind` into terms,
//! generating `t1 ≟ t2` constraints, and driving `unify::unify` to solve
//! them. Deliberately one crate, not split into a separate "just the
//! enum" crate and a logic crate -- `unify` is the reusable, roo-agnostic
//! piece; `TyCon` is roo-specific by definition, so it lives with the
//! rest of roo's type checking rather than off on its own. (rustc did
//! the same thing for most of its history: `rustc_typeck` held both,
//! only splitting into `rustc_hir_analysis`/`rustc_hir_typeck` once it
//! grew large enough for that to be worth it.)
//!
//! Not yet implemented -- nothing here yet.

use std::collections::{HashMap, HashSet};

use ast::visit::{Visitor, Walkable};
use ast::{
    Block, Expr, ExprKind, Fn, FnRetTy, FnTy, GenericArg, Item, ItemKind, LitKind, Local,
    LocalKind, ModKind, Pat, PatKind, Path, Span, Stmt, StmtKind, Ty, TyKind, VariantData,
};
use slotmap::SlotMap;
use unify::{Term, TermId, UnificationContext, UnifyError, VarId, term};

/// The severity of a [`Diagnostic`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Level {
    /// Checking failed outright at this point.
    Error,
    /// Checking succeeded, but something is likely a mistake.
    Warning,
    /// Additional context attached to another diagnostic.
    Note,
    /// A suggestion for how to fix another diagnostic.
    Help,
}

/// A single diagnostic produced while type checking -- an error,
/// warning, or note, with the span it applies to. Checking never
/// aborts on one of these; they're collected in
/// [`TypeCheckContext::diagnostics`] and returned all together once
/// checking finishes.
#[derive(Debug, Clone)]
pub struct Diagnostic {
    primary_span: Span,
    level: Level,
    message: String,
    related: Vec<(Span, String)>,
    notes: Vec<String>,
}

impl Diagnostic {
    fn error(span: Span, message: impl Into<String>) -> Self {
        Self::new(span, Level::Error, message)
    }

    fn warning(span: Span, message: impl Into<String>) -> Self {
        Self::new(span, Level::Warning, message)
    }

    fn note(span: Span, message: impl Into<String>) -> Self {
        Self::new(span, Level::Note, message)
    }

    fn help(span: Span, message: impl Into<String>) -> Self {
        Self::new(span, Level::Help, message)
    }

    fn new(span: Span, level: Level, message: impl Into<String>) -> Self {
        Self {
            primary_span: span,
            level,
            message: message.into(),
            related: Vec::new(),
            notes: Vec::new(),
        }
    }

    fn with_related(mut self, span: Span, message: impl Into<String>) -> Self {
        self.related.push((span, message.into()));
        self
    }

    fn with_note(mut self, message: impl Into<String>) -> Self {
        self.notes.push(message.into());
        self
    }

    fn cyclic_type(span: Span, expected: &str, actual: &str) -> Self {
        Self::error(span, "cyclic type of infinite size")
            .with_note(format!("expected type `{expected}`"))
            .with_note(format!("found type `{actual}`"))
    }

    /// This diagnostic's severity.
    pub fn level(&self) -> Level {
        self.level
    }

    /// The primary message describing this diagnostic.
    pub fn message(&self) -> &str {
        &self.message
    }

    /// The primary span this diagnostic applies to.
    pub fn span(&self) -> Span {
        self.primary_span
    }

    /// Secondary spans related to this diagnostic, each with a message
    /// explaining its relevance.
    pub fn related(&self) -> &[(Span, String)] {
        &self.related
    }

    /// Additional notes attached to this diagnostic.
    pub fn notes(&self) -> &[String] {
        &self.notes
    }
}

/// Type constructor enum. This enum's variants are all of the possible
/// types.
#[derive(Debug, Clone, PartialEq)]
enum TyCon {
    Any,

    Never,

    Int,
    Float,
    Bool,
    Char,
    Str,

    Fn,
    Array,
    Tuple,

    // We need the symbol id as part of the type to enforce the
    // nominal typing system, where two different structs or
    // enums which have the same exact structure should not
    // be considered the same type.
    Struct(SymbolId),
    Enum(SymbolId),

    // A generic type parameter. Not a wildcard: it
    // only unifies with another `Generic` carrying the same id, and is
    // rejected against every other constructor. A polymorphic symbol's
    // stored type is a template built from these; using it (e.g. calling
    // a generic function) substitutes each one for a fresh, ordinary
    // unification variable, one fresh copy per use.
    Generic(GenericId),

    Err,
}

slotmap::new_key_type! {
    /// Generational id for the scope arena.
    pub struct ScopeId;

    /// Generational id for the symbol arena.
    pub struct SymbolId;

    /// Identifies one generic parameter binding site (e.g. the `T` in
    /// `fn identity<T>(x: T) -> T`, or one discovered by generalizing an
    /// inferred function). Two [`TyCon::Generic`] terms are the same type
    /// only if they carry the same id. This is what gives a generic
    /// parameter rigid/skolem-like behavior (unifies with itself, fails
    /// against any concrete type) using nothing but `unify`'s ordinary
    /// constructor-equality check, with no changes to the `unify` crate
    /// itself.
    pub struct GenericId;
}

/// A unique identifier for a name. The [`NameInterner`] maps
/// strings to a unique [`NameId`] so name strings do not have
/// to be passed and copied around everywhere. For example in
/// hash maps, it's much cheaper to hash a NameId than a name
/// string directly.
#[derive(Copy, Clone, Eq, PartialEq, Hash)]
struct NameId(usize);

/// Identifies the namespace that a symbol belongs to. Type and
/// value symbols get their own namespaces so that typa and
/// value names are allowed to overlap without conflicting.
#[derive(Clone, Copy)]
pub enum Namespace {
    /// Contains types including structs, enums, traits, type
    /// aliases, and modules.
    Type,

    /// Contains values including variants, functions, and locals.
    Value,
}

/// Represents a scope.
struct Scope {
    /// The parent scope of this scope.
    parent: Option<ScopeId>,

    /// All symbols in the types namespace.
    types: HashMap<NameId, SymbolId>,

    /// All symbols in the values namespace.
    values: HashMap<NameId, SymbolId>,
}

/// Represents a symbol. A symbol represents something that has
/// been declared within a scope.
struct Symbol {
    /// The name of the symbol.
    name: NameId,

    /// The kind of the symbol.
    kind: SymbolKind,

    /// The type of the symbol.
    ty: TermId,

    /// The generic parameters quantified over in `ty`, if this symbol is
    /// polymorphic (e.g. a generic `fn`) . This list will be empty for
    /// every monomorphic symbol, which is the common case and keeps `ty`
    /// usable as-is with no special handling. A non-empty list means `ty`
    /// is a *template*: each `TyCon::Generic` id listed in the `ty` must
    /// be replaced with a fresh unification variable per use
    /// (instantiation) rather than read off directly.
    generics: Vec<GenericId>,

    /// The span of each parameter's own type annotation, in
    /// declaration order -- `None` for a parameter left unannotated.
    /// Used to point a mismatched-argument diagnostic's "expected due
    /// to this" note at the actual text that pinned the expected type
    /// down. Deliberately `None`, not a fallback span (e.g. the
    /// parameter's own pattern), when there's no annotation: pointing
    /// at *something* that doesn't actually explain the expected type
    /// is worse than not pointing at anything.
    param_spans: Vec<Option<Span>>,
}

/// The different kinds of symbols.
enum SymbolKind {
    Struct,
    Enum,
    Variant,
    Trait,
    TyAlias(ScopeId),
    Mod(ScopeId),
    Fn(ScopeId),
    Local,
    GenericParam,
}

impl SymbolKind {
    /// Determines whether this kind of symbol belongs in the type
    /// or value namespace of a scope.
    fn namespace(&self) -> Namespace {
        match self {
            SymbolKind::Struct
            | SymbolKind::Enum
            | SymbolKind::Trait
            | SymbolKind::TyAlias(_)
            | SymbolKind::GenericParam
            | SymbolKind::Mod(_) => Namespace::Type,
            SymbolKind::Variant | SymbolKind::Fn(_) | SymbolKind::Local => Namespace::Value,
        }
    }
}

/// Maps name strings to unique ids. Eliminates the need to pass raw
/// name strings around everywhere and hash them etc, instead just
/// using the id associated with the name string.
struct NameInterner {
    strings: Vec<String>,
    ids: HashMap<String, NameId>,
}

impl NameInterner {
    /// Creates a new empty [`NameInterner`].
    pub fn new() -> Self {
        Self {
            strings: vec![],
            ids: HashMap::new(),
        }
    }

    /// Returns the id associated with a name string.
    pub fn id(&mut self, string: &str) -> NameId {
        if let Some(id) = self.ids.get(string) {
            return *id;
        }

        let id = NameId(self.strings.len());
        self.strings.push(string.to_owned());
        self.ids.insert(string.to_owned(), id);
        id
    }

    /// Returns the name string associated with an id
    pub fn name(&self, id: NameId) -> Option<&String> {
        self.strings.get(id.0)
    }
}

/// Contains all data and all methods required for type checking.
pub struct TypeCheckContext {
    /// Used to track type inference variables and their equivalence
    /// classes, and facilitates the unification of different type
    /// terms.
    uni_cx: UnificationContext<TyCon>,

    /// Makes the mapping between ids and name strings for all type
    /// checking in this context.
    names: NameInterner,

    /// An arena which owns all symbols. Identifies each
    /// [`Symbol`] with a [`SymbolId`].
    symbols: SlotMap<SymbolId, Symbol>,

    /// An arena which owns all scopes. Identifies each
    /// [`Scopes`] with a [`ScopeId`].
    scopes: SlotMap<ScopeId, Scope>,

    generic_ids: SlotMap<GenericId, ()>,

    generic_names: HashMap<GenericId, String>,

    synthetic_generic_names: u32,

    /// The current scope being checked.
    current_scope: ScopeId,

    /// Symbols whose bodies are currently being checked, outermost
    /// first -- one entry per still-open [`Checker::check_fn_body`]
    /// call, so a nested function's own generalization can tell which
    /// enclosing, not-yet-finalized signatures it must not steal a
    /// free variable from. See [`Self::enclosing_free_vars`].
    checking_stack: Vec<SymbolId>,

    /// Diagnostics collected while type checking. Errors are pushed
    /// here rather than aborting, so checking always runs to completion
    /// over the whole file.
    diagnostics: Vec<Diagnostic>,
}

impl Default for TypeCheckContext {
    fn default() -> Self {
        Self::new()
    }
}

impl TypeCheckContext {
    /// Creates a new empty [`TypeCheckContext`].
    pub fn new() -> Self {
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
            generic_names: HashMap::new(),
            synthetic_generic_names: 0,
            symbols: SlotMap::with_key(),
            current_scope: root,
            checking_stack: Vec::new(),
            diagnostics: Vec::new(),
        }
    }

    /// Records a new diagnostic
    fn diagnostic(&mut self, diagnostic: Diagnostic) {
        self.diagnostics.push(diagnostic);
    }

    /// The diagnostics collected so far
    pub fn diagnostics(&self) -> &[Diagnostic] {
        &self.diagnostics
    }

    /// Does the first pass of the AST which creates all scopes
    /// and recursively declares symbols for all of the items in
    /// the AST.
    pub fn resolve(&mut self, items: &[Box<Item>]) {
        let mut resolver = Resolver { cx: self };
        for item in items {
            resolver.visit_item(item);
        }
    }

    /// Second pass, run after `resolve`. Lowers every `Fn`/`TyAlias`
    /// item's declared signature and unifies it into the symbol
    /// `resolve` already created a placeholder for.
    pub fn lower_signatures(&mut self, items: &[Box<Item>]) {
        let mut lowerer = SignatureLowerer { cx: self };
        for item in items {
            lowerer.visit_item(item);
        }
    }

    /// Third pass, run after `lower_signatures`: checks every item's body
    /// (currently just `Fn` bodies) against its already-lowered signature.
    /// Sibling `Fn`s are generalized together by strongly-connected
    /// call-graph component -- see [`Checker::check_items`].
    pub fn check(&mut self, items: &[Box<Item>]) {
        let mut checker = Checker { cx: self };
        let items: Vec<&Item> = items.iter().map(Box::as_ref).collect();
        checker.check_items(&items);
    }

    /// Resolves a path relative to the current scope, for a given
    /// namespace. Recursively searches parent scopes of the current
    /// scope until the first segment of the path is found, and then
    /// iterates over all segments, following the module specified
    /// by each segment until it reaches the symbol of the last
    /// segment in the path.
    pub fn resolve_path(&mut self, path: &Path, namespace: Namespace) -> Option<SymbolId> {
        let mut segments = path.segments.iter().peekable();

        let first = segments.next()?;
        let name = self.names.id(&first.ident.name);

        // If there is more than one segment in the path, then the
        // first segment must correspond to a module. Modules always
        // belong to the [`Namespace::Type`] namespace. Hence, the
        // namespace that should be searched for this first module
        // symbol is the [`Namespace::Type`] namespace.
        let ns = if segments.peek().is_some() {
            Namespace::Type
        } else {
            namespace
        };

        // Finds the symbol for the first segment in the path, first
        // checking the current scope and then recursively checking
        // parent scope until its found.
        let mut symbol = self.lookup_up_scope_chain(self.current_scope, name, ns)?;

        while let Some(segment) = segments.next() {
            let scope = match &self.symbols[symbol].kind {
                SymbolKind::Mod(scope) => *scope,
                _ => return None,
            };
            let name = self.names.id(&segment.ident.name);

            // If there is more than one segment in the path, then the
            // first segment must correspond to a module. Modules always
            // belong to the [`Namespace::Type`] namespace. Hence, the
            // namespace that should be searched for this first module
            // symbol is the [`Namespace::Type`] namespace.
            let ns = if segments.peek().is_some() {
                Namespace::Type
            } else {
                namespace
            };
            symbol = self.lookup_in_scope(scope, name, ns)?;
        }

        Some(symbol)
    }

    /// Checks if the scope has a symbol with the given name in the
    /// given namespace.
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

    /// Recursively searches parent scopes for a symbol with the given
    /// name, which belongs to the specified namespace. Continues until
    /// it finds such a symbol, or until it reaches the root scope which
    /// has no parent scope.
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

    /// Declares a new symbol within the current scope. Creates a new
    /// inference variable for the type of the symbol.
    fn declare(&mut self, name: &str, kind: SymbolKind) -> SymbolId {
        let namespace = kind.namespace();
        let name = self.names.id(name);
        let var = self.uni_cx.fresh_var();
        let ty = term!(self.uni_cx, var var);
        let symbol = self.symbols.insert(Symbol {
            name,
            kind,
            ty,
            generics: Vec::new(),
            param_spans: Vec::new(),
        });
        self.insert_in_scope(name, symbol, namespace);
        symbol
    }

    /// Declares a new generic parameter within the current scope.
    /// Used to make generic type parameters available within the
    /// scope of a function, or within the expression of a type
    /// alias.
    fn declare_generic_param(&mut self, param_name: &str) -> (SymbolId, GenericId) {
        let name = self.names.id(param_name);
        let id = self.generic_ids.insert(());
        self.generic_names.insert(id, param_name.to_owned());
        let ty = term!(self.uni_cx, TyCon::Generic(id));
        let symbol = self.symbols.insert(Symbol {
            name,
            kind: SymbolKind::GenericParam,
            ty,
            generics: Vec::new(),
            param_spans: Vec::new(),
        });
        self.insert_in_scope(name, symbol, Namespace::Type);
        (symbol, id)
    }

    /// Collects all the free inference variables within a type.
    /// Used to generalise functions which still have free type
    /// inference variables after being type-checked.
    fn free_vars(&mut self, term: TermId, out: &mut Vec<VarId>) {
        let resolved = self.uni_cx.resolve(term);
        match self.uni_cx.term(resolved).cloned() {
            Some(Term::Var(v)) => {
                let root = self.uni_cx.find(v);
                if !out.contains(&root) {
                    out.push(root);
                }
            }
            Some(Term::App { args, .. }) => {
                for arg in args {
                    self.free_vars(arg, out);
                }
            }
            None => {}
        }
    }

    /// The free type variables reachable from any symbol currently
    /// being checked -- every entry on [`Self::checking_stack`], i.e.
    /// every ancestor `fn` whose own body is still mid-check because
    /// checking *this* symbol is nested inside checking theirs.
    ///
    /// [`Self::generalize_group`] must never generalize one of these:
    /// doing so would quantify a variable that's also still free in
    /// an enclosing, not-yet-finalized signature -- exactly the
    /// classic Hindley-Milner "never generalize what's free in the
    /// environment" rule. Left untouched (still an ordinary
    /// `Term::Var`, not bound to a fresh `TyCon::Generic`), such a
    /// variable is correctly picked up later by whichever enclosing
    /// scope's own `generalize_group` call actually owns it -- the
    /// same sharing `generalize_group`'s own doc comment already
    /// describes for siblings within one call, just spanning nesting
    /// levels instead of one flat group.
    fn enclosing_free_vars(&mut self) -> HashSet<VarId> {
        let mut out = Vec::new();
        for i in 0..self.checking_stack.len() {
            let ty = self.symbols[self.checking_stack[i]].ty;
            self.free_vars(ty, &mut out);
        }
        out.into_iter().collect()
    }

    /// Creates a new synthetic name for a generic type parameter
    /// which has been inferred / introduced by generalisation of
    /// a function containing free inference variables after being
    /// typechecked.
    fn synthetic_generic_name(&mut self) -> String {
        const LETTERS: [char; 7] = ['T', 'U', 'V', 'W', 'X', 'Y', 'Z'];
        let n = self.synthetic_generic_names;
        self.synthetic_generic_names += 1;
        let letter = LETTERS[(n % LETTERS.len() as u32) as usize];
        let suffix = n / LETTERS.len() as u32;
        if suffix == 0 {
            letter.to_string()
        } else {
            format!("{letter}{}", suffix + 1)
        }
    }

    /// Like [`Self::synthetic_generic_name`], but keeps drawing names
    /// until it finds one not already in `taken`, adding it to `taken`
    /// before returning it. Without this, a symbol that already has
    /// explicitly-written generic parameters (e.g. `fn f<T>(...)`)
    /// could have a *newly* generalized free variable assigned that
    /// same name `T` again -- the counter `synthetic_generic_name`
    /// draws from has no idea `T` is already spoken for, since it only
    /// tracks synthetic names it has handed out itself. The two `T`s
    /// would still be perfectly sound (distinct `GenericId`s under the
    /// hood, instantiated independently), but the rendered signature
    /// would show two different parameters under one indistinguishable
    /// name, which is exactly the confusing-not-unsound failure mode
    /// this exists to rule out.
    fn fresh_synthetic_generic_name(&mut self, taken: &mut HashSet<String>) -> String {
        loop {
            let name = self.synthetic_generic_name();
            if taken.insert(name.clone()) {
                return name;
            }
        }
    }

    /// Generalizes every member of one strongly-connected call-graph
    /// component together, over the free type variables reachable
    /// from *any* member's type -- not each member's type in
    /// isolation. A non-recursive, non-mutually-recursive function is
    /// just the singleton-component case of the same operation.
    ///
    /// This has to be one combined pass rather than one call per
    /// member: if two members share an underlying unification
    /// variable (because, say, one's body used the other's result),
    /// generalizing the first member alone would bind that shared
    /// variable to a fresh `TyCon::Generic`, and the second member's
    /// own free-variable walk would then see an already-resolved
    /// `TyCon::Generic` there instead of a `Term::Var` -- silently
    /// leaving that generic parameter out of the second member's own
    /// `generics` list even though its type still structurally
    /// mentions it. Computing every member's free variables up front,
    /// before any of them are bound, avoids that.
    ///
    /// Two members ending up with the same [`GenericId`] in their
    /// respective `generics` lists (because they share a variable) is
    /// expected, not a bug: [`Self::instantiate_with`] always builds
    /// a fresh substitution per call, so separate call sites for each
    /// member still instantiate completely independently regardless
    /// of which ids their stored types happen to share.
    fn generalize_group(&mut self, members: &[SymbolId]) {
        self.synthetic_generic_names = 0;

        // Variables that are still free in an enclosing, not-yet-
        // finalized signature -- see `enclosing_free_vars`'s own doc
        // comment for why these can never be generalized here, only
        // deferred to whichever ancestor actually owns them.
        let enclosing = self.enclosing_free_vars();

        // Names already claimed by this component's own explicitly-
        // written generic parameters (already present in each member's
        // `generics` before this call runs, from `declare_generic_param`
        // during resolution) -- synthetic names handed out below must
        // avoid these too, not just each other, or the rendered
        // signature could show an explicit `<T>` and a newly-
        // generalized parameter under the same name.
        let mut taken: HashSet<String> = HashSet::new();
        for &symbol in members {
            for &id in &self.symbols[symbol].generics {
                if let Some(name) = self.generic_names.get(&id) {
                    taken.insert(name.clone());
                }
            }
        }

        // Every member's free variables, computed against the
        // original, still-unbound state -- must happen before any
        // binding below, since binding one member's variable would
        // hide it from a later member's own free_vars walk.
        let mut per_member_vars: Vec<(SymbolId, Vec<VarId>)> = Vec::with_capacity(members.len());
        for &symbol in members {
            let ty = self.symbols[symbol].ty;
            let mut vars = Vec::new();
            self.free_vars(ty, &mut vars);
            vars.retain(|v| !enclosing.contains(v));
            per_member_vars.push((symbol, vars));
        }

        // Assign exactly one fresh generic id per distinct free
        // variable across the whole group, in first-seen order, and
        // bind it once.
        let mut assigned: HashMap<VarId, GenericId> = HashMap::new();
        for (_, vars) in &per_member_vars {
            for &var in vars {
                if let std::collections::hash_map::Entry::Vacant(entry) = assigned.entry(var) {
                    let id = self.generic_ids.insert(());
                    let name = self.fresh_synthetic_generic_name(&mut taken);
                    self.generic_names.insert(id, name);
                    let generic_term = term!(self.uni_cx, TyCon::Generic(id));
                    self.uni_cx.bind(var, generic_term);
                    entry.insert(id);
                }
            }
        }

        // Each member only lists the generic ids that actually appear
        // in its own type, in the order it first encountered them --
        // exactly what `instantiate_with`'s positional zip needs.
        for (symbol, vars) in per_member_vars {
            for var in vars {
                let id = assigned[&var];
                self.symbols[symbol].generics.push(id);
            }
        }
    }

    /// Creates a new type with all generic type parameters in
    /// the symbol replaced with new inference variables.
    fn instantiate(&mut self, symbol: SymbolId) -> TermId {
        self.instantiate_with(symbol, &[])
    }

    /// Creates a new type with all generic type parameters in
    /// the symbol substituted with the given explicit types.
    fn instantiate_with(&mut self, symbol: SymbolId, explicit: &[TermId]) -> TermId {
        let ty = self.symbols[symbol].ty;
        if self.symbols[symbol].generics.is_empty() {
            return ty;
        }
        let generics = self.symbols[symbol].generics.clone();
        let mut subst = HashMap::new();
        for (&id, &term) in generics.iter().zip(explicit) {
            subst.insert(id, term);
        }
        self.instantiate_term(ty, &mut subst)
    }

    fn instantiate_term(&mut self, term: TermId, subst: &mut HashMap<GenericId, TermId>) -> TermId {
        let resolved = self.uni_cx.resolve(term);
        match self.uni_cx.term(resolved).cloned() {
            Some(Term::Var(_)) => resolved,
            Some(Term::App {
                constructor: TyCon::Generic(id),
                ..
            }) => *subst.entry(id).or_insert_with(|| {
                let var = self.uni_cx.fresh_var();
                term!(self.uni_cx, var var)
            }),
            Some(Term::App { constructor, args }) => {
                let new_args = args
                    .iter()
                    .map(|&arg| self.instantiate_term(arg, subst))
                    .collect();
                term!(self.uni_cx, constructor => new_args)
            }
            None => resolved,
        }
    }

    fn instantiate_path(&mut self, symbol: SymbolId, path: &Path) -> TermId {
        match path.segments.last().and_then(|seg| seg.args.as_ref()) {
            Some(generic_args) => {
                // Retireve and lower all generic type arguments in the last
                // segment of the path, which should be used to instantiate
                // the symbol.
                let arg_tys: Vec<TermId> = generic_args
                    .args
                    .iter()
                    .filter_map(|arg| match arg {
                        GenericArg::Arg(ty) => Some(self.lower_ty(ty)),
                        GenericArg::Constraint(_) => None,
                    })
                    .collect();

                let max = self.symbols[symbol].generics.len();
                let actual = arg_tys.len();
                if actual != max {
                    let message = format!(
                        "expected {max} generic argument{}, found {actual}",
                        if max == 1 { "" } else { "s" },
                    );
                    self.diagnostic(Diagnostic::error(generic_args.span, message));
                }

                // Substitute the generic type parameters in the symbol's type
                // with the type arguments specified.
                self.instantiate_with(symbol, &arg_tys[..actual.min(max)])
            }
            None => self.instantiate(symbol),
        }
    }

    /// Inserts an existing symbol into the current scope.
    fn insert_in_scope(&mut self, name: NameId, symbol: SymbolId, namespace: Namespace) {
        let scope = &mut self.scopes[self.current_scope];
        match namespace {
            Namespace::Type => scope.types.insert(name, symbol),
            Namespace::Value => scope.values.insert(name, symbol),
        };
    }

    /// Converts a Type node from the AST into a corresponding type term.
    fn lower_ty(&mut self, ty: &Ty) -> TermId {
        match &ty.kind {
            TyKind::Never => term!(self.uni_cx, TyCon::Never),
            // the Paren type is kept simply for pretty printing, but the parenthesis
            // introduce no new information to the type they surround. Hence the type
            // of the node is just the type of the inner node.
            TyKind::Paren(inner) => self.lower_ty(inner),
            TyKind::Array(inner) => term!(self.uni_cx, TyCon::Array => [ self.lower_ty(inner) ]),
            TyKind::Tup(inner) => {
                let args = inner.iter().map(|x| self.lower_ty(x)).collect();
                term!(self.uni_cx, TyCon::Tuple => args)
            }
            TyKind::Fn(fn_ty) => {
                let FnTy { inputs, output } = fn_ty.as_ref();
                let input_args = inputs.iter().map(|x| self.lower_ty(x)).collect();
                let inputs_term = term!(self.uni_cx, TyCon::Tuple => input_args);
                let output_term = match output {
                    FnRetTy::Default(_) => {
                        let var = self.uni_cx.fresh_var();
                        term!(self.uni_cx, var var)
                    }
                    FnRetTy::Ty(ty) => self.lower_ty(ty),
                };
                term!(self.uni_cx, TyCon::Fn => [inputs_term, output_term])
            }
            TyKind::Path(path) => match path.segments.as_slice() {
                // The primitive types in the language are just special path names.
                [segment] if segment.ident.name == "bool" => term!(self.uni_cx, TyCon::Bool),
                [segment] if segment.ident.name == "int" => term!(self.uni_cx, TyCon::Int),
                [segment] if segment.ident.name == "float" => term!(self.uni_cx, TyCon::Float),
                [segment] if segment.ident.name == "char" => term!(self.uni_cx, TyCon::Char),
                [segment] if segment.ident.name == "String" => term!(self.uni_cx, TyCon::Str),
                [segment] if segment.ident.name == "any" => term!(self.uni_cx, TyCon::Any),
                _ => match self.resolve_path(path, Namespace::Type) {
                    Some(symbol) => match &self.symbols[symbol].kind {
                        SymbolKind::Struct => term!(self.uni_cx, TyCon::Struct(symbol)),
                        SymbolKind::Enum => term!(self.uni_cx, TyCon::Enum(symbol)),
                        _ => self.instantiate_path(symbol, path),
                    },
                    None => term!(self.uni_cx, TyCon::Err),
                },
            },
            TyKind::ImplicitSelf => unimplemented!(),

            // An Infer AST node means that the type should be inferred from
            // the surrounding context. That means a new inference variable should
            // be created to represent this type, which can be pinned down during
            // type-checking via unification.
            TyKind::Infer => {
                let var = self.uni_cx.fresh_var();
                term!(self.uni_cx, var var)
            }
            TyKind::Err => term!(self.uni_cx, TyCon::Err),
        }
    }

    /// Gets the span for the return value of a block. If the block has at
    /// least one statement, then we take the span to be the last statement,
    /// since that will produce the return value of the block. Otherwise, just
    /// use the span of the entire block if it is empty.
    fn block_value_span(block: &Block) -> Span {
        match block.stmts.last() {
            Some(Stmt {
                kind: StmtKind::Expr(expr),
                ..
            }) => expr.span,
            _ => block.span,
        }
    }

    /// Renders a term as roo source-like text, for use in diagnostic
    /// messages. Resolves through any bound inference variables first --
    /// an unresolved variable renders as `_`, since it doesn't have a
    /// concrete type to show yet.
    fn render_term(&mut self, term: TermId) -> String {
        let resolved = self.uni_cx.resolve(term);
        let Some(term) = self.uni_cx.term(resolved).cloned() else {
            return "<error>".to_owned();
        };

        let (constructor, args) = match term {
            Term::Var(_) => return "_".to_owned(),
            Term::App { constructor, args } => (constructor, args),
        };

        match constructor {
            TyCon::Any => "any".to_owned(),
            TyCon::Never => "!".to_owned(),
            TyCon::Int => "int".to_owned(),
            TyCon::Float => "float".to_owned(),
            TyCon::Bool => "bool".to_owned(),
            TyCon::Char => "char".to_owned(),
            TyCon::Str => "String".to_owned(),
            TyCon::Err => "<error>".to_owned(),
            TyCon::Array => format!("[{}]", self.render_term(args[0])),
            TyCon::Tuple => {
                let elems: Vec<String> = args.iter().map(|&arg| self.render_term(arg)).collect();
                format!("({})", elems.join(", "))
            }
            TyCon::Fn => {
                let inputs = self.render_term(args[0]);
                let output = self.render_term(args[1]);
                format!("Fn{inputs} -> {output}")
            }
            TyCon::Struct(symbol) | TyCon::Enum(symbol) => {
                let name = self.symbols[symbol].name;
                self.names
                    .name(name)
                    .cloned()
                    .unwrap_or_else(|| "<unknown>".to_owned())
            }
            TyCon::Generic(id) => self
                .generic_names
                .get(&id)
                .cloned()
                .unwrap_or_else(|| "<generic>".to_owned()),
        }
    }

    /// Renders a symbol's type as roo source-like text, the same way
    /// [`Self::render_term`] does, but prefixed with the symbol's own
    /// generic parameters (if any) the way a `fn`'s `<T, U>` list would
    /// read. Meant for tooling (a CLI, an editor integration, ...) that
    /// wants to display "what did the checker figure out for this item"
    /// without reaching into `TermId`/`Term` internals directly.
    pub fn render_symbol_type(&mut self, symbol: SymbolId) -> String {
        let ty = self.symbols[symbol].ty;
        let rendered = self.render_term(ty);
        let generics = self.symbols[symbol].generics.clone();
        if generics.is_empty() {
            return rendered;
        }
        let names: Vec<String> = generics
            .iter()
            .map(|id| {
                self.generic_names
                    .get(id)
                    .cloned()
                    .unwrap_or_else(|| "<generic>".to_owned())
            })
            .collect();
        format!("<{}> {rendered}", names.join(", "))
    }

    /// Determines the type of an Expression node from the AST, possibly given
    /// additional information about what the type is expected to be.
    fn check_expr(&mut self, expr: &Expr, expected: Option<TermId>) -> TermId {
        self.check_expr_expecting(expr, expected, None)
    }

    /// Gets the name of a generic parameter.
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

    fn check_expr_expecting(
        &mut self,
        expr: &Expr,
        expected: Option<TermId>,
        expected_span: Option<Span>,
    ) -> TermId {
        let actual = self.check_expr_kind(&expr.kind, expected);
        if let Some(expected) = expected {
            let generic_on_expected = self.generic_name_of(expected);
            let generic_on_actual = if generic_on_expected.is_none() {
                self.generic_name_of(actual)
            } else {
                None
            };
            if let Err(err) = self.uni_cx.unify(actual, expected) {
                let expected = self.render_term(expected);
                let actual = self.render_term(actual);
                match err {
                    UnifyError::OccursCheck(_) => {
                        self.diagnostic(Diagnostic::cyclic_type(expr.span, &expected, &actual));
                    }
                    _ => {
                        let mut diagnostic = Diagnostic::error(
                            expr.span,
                            format!("expected `{expected}`, found `{actual}`"),
                        );
                        if let Some(name) = generic_on_expected {
                            diagnostic = diagnostic.with_note(format!(
                                "`{name}` is generic here and must work for every type, not just `{actual}`"
                            ));
                        } else if let Some(name) = generic_on_actual {
                            diagnostic = diagnostic.with_note(format!(
                                "`{name}` is generic here and must work for every type, not just `{expected}`"
                            ));
                        }
                        let diagnostic = match expected_span {
                            Some(span) => diagnostic.with_related(span, "expected due to this"),
                            None => diagnostic,
                        };
                        self.diagnostic(diagnostic);
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

                if let Err(err) = self.uni_cx.unify(body_ty, els_ty) {
                    let body_span = Self::block_value_span(body);
                    let els_span = match els.as_deref() {
                        Some(Expr {
                            kind: ExprKind::Block(block, _),
                            ..
                        }) => Self::block_value_span(block),
                        Some(els) => els.span,
                        None => body_span,
                    };

                    let body_rendered = self.render_term(body_ty);
                    let els_rendered = self.render_term(els_ty);
                    match err {
                        UnifyError::OccursCheck(_) => {
                            self.diagnostic(Diagnostic::cyclic_type(
                                els_span,
                                &body_rendered,
                                &els_rendered,
                            ));
                        }
                        _ => {
                            let diagnostic = Diagnostic::error(
                                els_span,
                                format!("expected `{body_rendered}`, found `{els_rendered}`"),
                            );
                            let diagnostic = if els.is_some() {
                                diagnostic.with_related(body_span, "expected because of this")
                            } else {
                                diagnostic
                            };
                            self.diagnostic(diagnostic);
                        }
                    }
                }

                self.prefer_non_never(body_ty, els_ty)
            }
            ExprKind::Block(block, _) => self.check_block(block, expected),
            ExprKind::Tup(exprs) => {
                // Checks if the expected type is also a tuple of the same
                // length as this expression. If so, it gets the expected
                // type of each position in the tuple.
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

                // Checks the type of each expression in each position of the this
                // tuple expression, with the constraint that each one must match
                // the type of the corresponding position in the expected type.
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
                // TODO Check the returned value against the enclosing
                // fn's declared return type once that context exists
                // (same gap as `break` needing to know its enclosing
                // loop's type) -- for now just check it in isolation.
                if let Some(expr) = expr {
                    self.check_expr(expr, None);
                }
                term!(self.uni_cx, TyCon::Never)
            }
            ExprKind::Path(qself, path) => {
                // TODO Implement qualified paths
                if qself.is_some() {
                    unimplemented!();
                }

                match self.resolve_path(path, Namespace::Value) {
                    // `instantiate_path` already does the right thing for a
                    // symbol that's mid-check as part of the strongly-
                    // connected component currently being checked together
                    // (see `Checker::check_items`): such a symbol's
                    // `generics` is still empty at this point --
                    // `generalize_group` only runs after every member of
                    // the component has been checked -- so this call is a
                    // no-op passthrough
                    // that shares the raw, still-unified type directly,
                    // exactly the sharing plain self-recursion always
                    // relied on, just scoped to the whole component.
                    Some(symbol) => self.instantiate_path(symbol, path),
                    None => term!(self.uni_cx, TyCon::Err),
                }
            }
            ExprKind::Call(callee, args) => {
                // Check the type of the expression which is being called
                let callee_ty = self.check_expr(callee, None);

                let callee_param_spans: Vec<Option<Span>> = match &callee.kind {
                    ExprKind::Path(None, path) => self
                        .resolve_path(path, Namespace::Value)
                        .map(|symbol| self.symbols[symbol].param_spans.clone())
                        .unwrap_or_default(),
                    _ => Vec::new(),
                };

                // If the callee's type is already known to be a Fn with
                // the same number of parameters as this call has
                // arguments, pull those parameter types out.
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
                        self.diagnostic(Diagnostic::error(span, message));
                    }

                    // Check every argument we have against its declared
                    // parameter type where one exists, regardless of
                    // whether the arity matched -- still type-checks as
                    // much as possible instead of bailing out entirely
                    // on an arity mismatch.
                    for (i, arg) in args.iter().enumerate() {
                        let expected_ty = input_tys.get(i).copied();
                        let expected_span = callee_param_spans.get(i).copied().flatten();
                        self.check_expr_expecting(arg, expected_ty, expected_span);
                    }

                    output_term
                } else if matches!(
                    self.uni_cx.term(resolved_callee),
                    None | Some(Term::Var(_))
                ) {
                    // Callee's shape isnt known yet -- still an unbound
                    // inference variable, not yet pinned to anything.
                    // Not enough information to individually check types
                    // of arguments to expected types of parameters, so
                    // infer a Fn shape for the callee from how it's
                    // actually called here instead. Binding a fresh
                    // variable can never fail, so this unification is
                    // infallible.
                    let arg_tys = args.iter().map(|arg| self.check_expr(arg, None)).collect();
                    let inputs_term = term!(self.uni_cx, TyCon::Tuple => arg_tys);
                    let ret_var = self.uni_cx.fresh_var();
                    let ret_term = term!(self.uni_cx, var ret_var);
                    let fn_term = term!(self.uni_cx, TyCon::Fn => [inputs_term, ret_term]);
                    let _ = self.uni_cx.unify(callee_ty, fn_term);
                    ret_term
                } else {
                    // Callee's type is already known -- and it's
                    // concretely something other than Fn. Unlike the
                    // branch above, this can't be resolved by unifying
                    // against a synthesized Fn shape (that would just
                    // fail), so it's a real error: this value isn't
                    // callable at all.
                    let found = self.render_term(callee_ty);
                    self.diagnostic(Diagnostic::error(
                        callee.span,
                        format!("expected a function, found `{found}`"),
                    ));

                    // Still check the arguments, best-effort, so
                    // checking continues past this error instead of
                    // skipping them entirely -- there's just no
                    // parameter type to check them against.
                    for arg in args {
                        self.check_expr(arg, None);
                    }

                    // `Err` is a wildcard that unifies with anything, so
                    // this call's bogus result type doesn't cascade into
                    // a second, misleading diagnostic wherever it's used.
                    term!(self.uni_cx, TyCon::Err)
                }
            }
            ExprKind::Cast(_expr, ty) => {
                // TODO Check if expr can actually be cast to ty

                // The type of a cast from some expression to some typ `ty` will
                // always produce something with type `ty`.
                self.lower_ty(ty)
            }
            ExprKind::Array(exprs) => {
                // If the expected type is also an array, it checks what type of
                // elementents the array should be made up of.
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
                    // If there is at least one element in the array, then it
                    // type checks the first element in the array, with the
                    // constraint that its type must match the expected type
                    // of the array elements determined above.
                    Some(first) => {
                        let first_ty = self.check_expr(first, expected_ty);

                        // Type check the rest of the expressions in the array,
                        // with the constraint that their types must match the
                        // type of the first element in the array, which we already
                        // constrained above to match the `expected_ty`.
                        for rest in exprs {
                            self.check_expr(rest, Some(first_ty));
                        }
                        first_ty
                    }
                    // If there are no elements in the array, then a new type
                    // inference variable is created to represent the type that
                    // the array holds, since it cannot be determined with only
                    // this context.
                    None => expected_ty.unwrap_or_else(|| {
                        let var = self.uni_cx.fresh_var();
                        term!(self.uni_cx, var var)
                    }),
                };

                term!(self.uni_cx, TyCon::Array => [elem_ty])
            }
            ExprKind::Assign(lhs, rhs, _) => {
                // Check the type of the `lhs` of the assignment.
                let lhs = self.check_expr(lhs, None);

                // Check the type of the `rhs` of the assignment, with the constraint
                // that the type of the `rhs` must match the type of the `lhs` which
                // was determined above.
                self.check_expr(rhs, Some(lhs));

                term!(self.uni_cx, TyCon::Tuple)
            }
            _ => unimplemented!(),
        }
    }

    /// Checks the type of a Block node from the AST.
    fn check_block(&mut self, block: &Block, expected: Option<TermId>) -> TermId {
        self.check_block_expecting(block, expected, None)
    }

    fn check_block_expecting(
        &mut self,
        block: &Block,
        expected: Option<TermId>,
        expected_span: Option<Span>,
    ) -> TermId {
        let mut ty = term!(self.uni_cx, TyCon::Tuple);
        // Whether any statement so far diverges (has type Never), e.g. a
        // `return 0;` with a trailing semicolon. Everything after such a
        // statement is unreachable, so the block itself never actually
        // produces `ty` -- it diverges too, regardless of what its
        // syntactic tail expression would otherwise type-check to.
        let mut diverges = false;
        for (i, stmt) in block.stmts.iter().enumerate() {
            let is_last = i == block.stmts.len() - 1;
            match &stmt.kind {
                // Type check the local statement. However, the local statement
                // is not related to the return type of the block.
                StmtKind::Let(local) => self.check_local(local),

                // This is the last statement in the block, and it is an expression.
                // Type check the expression, with the constraint that its type
                // much match the expected type of the block.
                StmtKind::Expr(expr) if is_last => {
                    ty = self.check_expr_expecting(expr, expected, expected_span);
                }

                // Type check the statements, however, since they are not the last
                // expression in the block, they are not related to the return type
                // of the block.
                StmtKind::Expr(expr) | StmtKind::Semi(expr) => {
                    let stmt_ty = self.check_expr(expr, None);
                    if self.is_never(stmt_ty) {
                        diverges = true;
                    }
                }

                // Nothing to do. Items are already traversed as part of the resolve
                // stage.
                StmtKind::Item(_) | StmtKind::Empty => {}
            }
        }
        if diverges {
            term!(self.uni_cx, TyCon::Never)
        } else {
            ty
        }
    }

    /// Whether `term` resolves to the Never `!` type.
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

    /// Type checks a `Local` statement.
    fn check_local(&mut self, local: &Local) {
        // The type annotation for this local, which it may not have.
        let ascribed = local.ty.as_ref().map(|ty| self.lower_ty(ty));
        let ascribed_span = local.ty.as_ref().map(|ty| ty.span);

        let expected = match &local.kind {
            // Local declaration.
            // Example: `let x;`
            //
            // In this case, the type of the local is just the annotated
            // type. If there isnt an annotated type, a new inference
            // variable will be introduced for the type.
            LocalKind::Decl => ascribed,

            // Local declaration with an initialiser.
            // Example: `let x = y;`
            //
            // In this case, we type check the expression being used to
            // initialise the local, and expect it to match the annotated
            // type if there is one. If there is no annotated type, then
            // the local should have type which matches the actual type
            // of the expression its being initialised with.
            LocalKind::Init(init) => {
                let actual = self.check_expr_expecting(init, ascribed, ascribed_span);
                Some(ascribed.unwrap_or(actual))
            }

            // Local declaration with an initialiser and an `else` clause.
            // Example: `let Some(x) = y else { return };`
            //
            // Same as the case above, except that we need to also type-check
            // the `else` clause, which should be required to diverge (and
            // so have return type Never).
            LocalKind::InitElse(init, else_block) => {
                let actual = self.check_expr_expecting(init, ascribed, ascribed_span);
                // TODO The else block should be required to diverge
                // (Never). Not enforced yet, no diagnostics to report
                // it with.
                self.check_block(else_block, None);
                Some(ascribed.unwrap_or(actual))
            }
        };

        // The only case where the expected type of the local would be
        // empty is when it is a declaration with no annotated type.
        // Then the expected type of the local cannot be determined by
        // the annotated type *or* the type of an initialisation
        // expression. Hence, the type of the local is completely
        // unconstrained, so a new inference variable is introduced
        // to represent its type.
        let expected = expected.unwrap_or_else(|| {
            let var = self.uni_cx.fresh_var();
            term!(self.uni_cx, var var)
        });

        // Checks that the pattern
        self.check_pat(&local.pat, expected);
    }

    /// Given two types, it will chose the type which is not the Never `!` type.
    /// In the case where neither is never, it will just return the first one.
    fn prefer_non_never(&mut self, a: TermId, b: TermId) -> TermId {
        if self.is_never(a) { b } else { a }
    }

    /// Checks that a pattern's shape correctly fits a value of the
    /// expected type, and also determines what type each name inside
    /// the pattern will end up with.
    fn check_pat(&mut self, pat: &Pat, expected: TermId) -> TermId {
        let actual = self.check_pat_kind(&pat.kind, expected);
        let _ = self.uni_cx.unify(actual, expected);
        actual
    }

    fn check_pat_kind(&mut self, kind: &PatKind, expected: TermId) -> TermId {
        match kind {
            // The Wildcard pattern `_` deliberately matches with anything.
            PatKind::Wild => expected,
            PatKind::Ident(ident, sub) => {
                let symbol = self.declare(&ident.name, SymbolKind::Local);
                let _ = self.uni_cx.unify(self.symbols[symbol].ty, expected);

                if let Some(sub) = sub {
                    self.check_pat(sub, expected);
                }

                expected
            }
            PatKind::Tuple(pats) => {
                let resolved = self.uni_cx.resolve(expected);

                // When the patten is a tuple destructure, it checks that the
                // expected type is a tuple, and also gets the expected type
                // at each position within the tuple.
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
                            // For the ith position in the tuple pattern, it gets
                            // the expected type in the corresponding position.
                            // But if the expected type is not a tuple, then create
                            // new inference variables for each position in the tuple
                            // pattern. This is because although `expected` is not a tuple,
                            // it may still be an unbound inference variable, and so
                            // the final `unify` call from the `check_pat` wrapper would
                            // then actually pin down `expected` to be a tuple. On the
                            // other hand, if it turns out `expected` really was a
                            // Term::App with a non-tuple constructor, then the final
                            // `unify` call from `check_pat` will fail.
                            let expected = expected_args
                                .as_ref()
                                .map(|args| args[i])
                                .unwrap_or_else(|| {
                                    let var = self.uni_cx.fresh_var();
                                    term!(self.uni_cx, var var)
                                });
                            self.check_pat(pat, expected)
                        })
                        .collect();

                term!(self.uni_cx, TyCon::Tuple => args)
            }
            _ => unimplemented!(),
        }
    }
}

/// The first pass of the AST. Walks the AST and declares symbols
/// for all relevant nodes.
struct Resolver<'a> {
    cx: &'a mut TypeCheckContext,
}

impl Resolver<'_> {
    /// Creates a child scope of the current one and returns its id,
    /// without entering it. This lets a symbol that owns a scope
    /// (`Mod`, `Fn`) be declared with that `ScopeId` already in hand,
    /// before `with_scope` descends into it.
    fn new_scope(&mut self) -> ScopeId {
        let parent = self.cx.current_scope;
        self.cx.scopes.insert(Scope {
            parent: Some(parent),
            types: HashMap::new(),
            values: HashMap::new(),
        })
    }

    /// Enters a scope, performs some function on this context
    /// while in the scope, and then leaves the scope again,
    /// going back to the starting scope.
    fn with_scope(&mut self, scope: ScopeId, f: impl FnOnce(&mut Self)) {
        let parent = self.cx.current_scope;
        self.cx.current_scope = scope;
        f(self);
        self.cx.current_scope = parent;
    }
}

impl Visitor for Resolver<'_> {
    fn visit_item(&mut self, item: &Item) {
        match &item.kind {
            ItemKind::Fn(f) => {
                let scope = self.new_scope();
                let fn_symbol = self.cx.declare(&f.ident.name, SymbolKind::Fn(scope));

                let mut generics = Vec::new();
                self.with_scope(scope, |this| {
                    for param in &f.generics.params {
                        let (_, id) = this.cx.declare_generic_param(&param.ident.name);
                        generics.push(id);
                    }
                    item.walk(this);
                });
                self.cx.symbols[fn_symbol].generics = generics;
                return;
            }
            ItemKind::TyAlias(alias) => {
                let scope = self.new_scope();
                let alias_symbol = self
                    .cx
                    .declare(&alias.ident.name, SymbolKind::TyAlias(scope));

                let mut generics = Vec::new();
                self.with_scope(scope, |this| {
                    for param in &alias.generics.params {
                        let (_, id) = this.cx.declare_generic_param(&param.ident.name);
                        generics.push(id);
                    }
                });
                self.cx.symbols[alias_symbol].generics = generics;
            }
            ItemKind::Enum(ident, _generics, _def) => {
                self.cx.declare(&ident.name, SymbolKind::Enum);
            }
            ItemKind::Struct(ident, _generics, data) => {
                let symbol = self.cx.declare(&ident.name, SymbolKind::Struct);
                if !matches!(data, VariantData::Struct(_)) {
                    let name = self.cx.names.id(&ident.name);
                    self.cx.insert_in_scope(name, symbol, Namespace::Value);
                }
            }
            ItemKind::Trait(t) => {
                self.cx.declare(&t.ident.name, SymbolKind::Trait);
            }
            ItemKind::Mod(ident, ModKind::Unloaded) => {
                // Empty for now. A future `ModuleLoader` fetches this
                // mod's contents and resolves them into this same scope.
                let scope = self.new_scope();
                self.cx.declare(&ident.name, SymbolKind::Mod(scope));
            }
            ItemKind::Mod(ident, ModKind::Loaded(_)) => {
                let scope = self.new_scope();
                self.cx.declare(&ident.name, SymbolKind::Mod(scope));
                self.with_scope(scope, |this| item.walk(this));
                return;
            }
            ItemKind::Use(_) | ItemKind::Impl(_) => {}
        }
    }
}

/// Lowers declared `Fn`/`TyAlias` signatures into the placeholders
/// `Resolver` already created for them.
struct SignatureLowerer<'a> {
    cx: &'a mut TypeCheckContext,
}

impl SignatureLowerer<'_> {
    fn with_scope(&mut self, scope: ScopeId, f: impl FnOnce(&mut Self)) {
        let parent = self.cx.current_scope;
        self.cx.current_scope = scope;
        f(self);
        self.cx.current_scope = parent;
    }

    fn lower_fn_sig(&mut self, f: &Fn) -> TermId {
        let inputs = f
            .sig
            .inputs
            .iter()
            .map(|param| match &param.ty {
                Some(ty) => self.cx.lower_ty(ty),
                None => {
                    let var = self.cx.uni_cx.fresh_var();
                    term!(self.cx.uni_cx, var var)
                }
            })
            .collect();
        let inputs_term = term!(self.cx.uni_cx, TyCon::Tuple => inputs);
        let output_term = match &f.sig.output {
            FnRetTy::Default(_) => {
                let var = self.cx.uni_cx.fresh_var();
                term!(self.cx.uni_cx, var var)
            }
            FnRetTy::Ty(ty) => self.cx.lower_ty(ty),
        };
        term!(self.cx.uni_cx, TyCon::Fn => [inputs_term, output_term])
    }
}

impl Visitor for SignatureLowerer<'_> {
    fn visit_item(&mut self, item: &Item) {
        match &item.kind {
            ItemKind::Fn(f) => {
                let name = self.cx.names.id(&f.ident.name);
                let symbol = self
                    .cx
                    .lookup_in_scope(self.cx.current_scope, name, Namespace::Value);

                let scope = symbol.and_then(|symbol| match &self.cx.symbols[symbol].kind {
                    SymbolKind::Fn(scope) => Some(*scope),
                    _ => None,
                });
                if let Some(scope) = scope {
                    self.with_scope(scope, |this| {
                        if let Some(symbol) = symbol {
                            let fn_term = this.lower_fn_sig(f);
                            let symbol_ty = this.cx.symbols[symbol].ty;
                            let _ = this.cx.uni_cx.unify(symbol_ty, fn_term);
                            this.cx.symbols[symbol].param_spans = f
                                .sig
                                .inputs
                                .iter()
                                .map(|p| p.ty.as_ref().map(|ty| ty.span))
                                .collect();
                        }
                        item.walk(this);
                    });
                }
            }
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

/// Computes the strongly-connected components of a directed graph
/// given as `nodes` (every node, in a fixed, deterministic order) and
/// `edges` (each node's outgoing edges, also in a fixed order),
/// returning them in dependency order: a component only ever depends
/// on components that appear *before* it in the result, never one
/// that appears after. Processing the returned list in order
/// therefore guarantees every dependency of a component has already
/// been processed by the time that component is reached.
///
/// Implemented as Tarjan's algorithm, which produces exactly that
/// order as a side effect of a single depth-first traversal: a
/// component is only popped once every node reachable from it has
/// been fully explored, so a callee's component always pops before
/// its caller's. Recursive, so a pathologically deep call graph could
/// overflow the stack -- not a real concern at the scale of a single
/// module's sibling functions.
fn strongly_connected_components(
    nodes: &[SymbolId],
    edges: &HashMap<SymbolId, Vec<SymbolId>>,
) -> Vec<Vec<SymbolId>> {
    struct State {
        index: HashMap<SymbolId, u32>,
        lowlink: HashMap<SymbolId, u32>,
        on_stack: HashSet<SymbolId>,
        stack: Vec<SymbolId>,
        next_index: u32,
        sccs: Vec<Vec<SymbolId>>,
    }

    fn visit(node: SymbolId, edges: &HashMap<SymbolId, Vec<SymbolId>>, state: &mut State) {
        state.index.insert(node, state.next_index);
        state.lowlink.insert(node, state.next_index);
        state.next_index += 1;
        state.stack.push(node);
        state.on_stack.insert(node);

        for &successor in edges.get(&node).map(Vec::as_slice).unwrap_or_default() {
            if !state.index.contains_key(&successor) {
                // Tree edge: recurse, then pull up whatever the
                // successor can reach.
                visit(successor, edges, state);
                let pulled = state.lowlink[&successor];
                let current = state.lowlink[&node];
                state.lowlink.insert(node, current.min(pulled));
            } else if state.on_stack.contains(&successor) {
                // Back/cross edge into a node still on the stack --
                // part of the same not-yet-closed component.
                let successor_index = state.index[&successor];
                let current = state.lowlink[&node];
                state.lowlink.insert(node, current.min(successor_index));
            }
            // An edge into a node that's visited but no longer on the
            // stack points at an already-finished, unrelated
            // component -- nothing to do.
        }

        if state.lowlink[&node] == state.index[&node] {
            let mut component = Vec::new();
            loop {
                let member = state
                    .stack
                    .pop()
                    .expect("node's own frame is still on the stack until its root pops it");
                state.on_stack.remove(&member);
                component.push(member);
                if member == node {
                    break;
                }
            }
            state.sccs.push(component);
        }
    }

    let mut state = State {
        index: HashMap::new(),
        lowlink: HashMap::new(),
        on_stack: HashSet::new(),
        stack: Vec::new(),
        next_index: 0,
        sccs: Vec::new(),
    };
    for &node in nodes {
        if !state.index.contains_key(&node) {
            visit(node, edges, &mut state);
        }
    }
    state.sccs
}

/// Walks one `fn`'s body collecting every other member of its sibling
/// group (including itself) that it references by bare name, for
/// [`Checker::check_items`] to build a call graph from. Deliberately
/// coarse: it matches on the referenced *name* alone, not on real
/// scope-resolved identity, so a local binding that happens to shadow
/// a sibling's name is (harmlessly) still counted as a reference to
/// that sibling. This can only ever over-approximate the real call
/// graph, never under-approximate it -- at worst grouping two
/// functions into one strongly-connected component that didn't
/// strictly need to be grouped, which is exactly the same shape of
/// (safe, sound) conservatism the previous whole-flag approach had
/// everywhere, not a new source of unsoundness.
struct CallGraphCollector<'a> {
    sibling_names: &'a HashMap<&'a str, SymbolId>,
    edges: Vec<SymbolId>,
}

impl Visitor for CallGraphCollector<'_> {
    fn visit_expr(&mut self, expr: &Expr) {
        if let ExprKind::Path(None, path) = &expr.kind
            && let [segment] = path.segments.as_slice()
            && let Some(&symbol) = self.sibling_names.get(segment.ident.name.as_str())
        {
            self.edges.push(symbol);
        }
        expr.walk(self);
    }
}

/// The second pass of the AST. Walks the AST and, for relevant nodes,
/// it generates constraints between types and unifies them to
/// determine types.
struct Checker<'a> {
    cx: &'a mut TypeCheckContext,
}

impl Checker<'_> {
    fn with_scope(&mut self, scope: ScopeId, f: impl FnOnce(&mut Self)) {
        let parent = self.cx.current_scope;
        self.cx.current_scope = scope;
        f(self);
        self.cx.current_scope = parent;
    }

    /// Checks every item in one sibling group -- either the items at
    /// the top level of a module, or the items hoisted into one `fn`
    /// body -- generalizing its `Fn` items together by
    /// strongly-connected call-graph component rather than one
    /// function at a time in declaration order. This is the entry
    /// point both [`TypeCheckContext::check`] and the recursive
    /// hoisted-item case (in [`Self::check_fn_body`]) use.
    fn check_items(&mut self, items: &[&Item]) {
        // Resolve every sibling `Fn` item to its symbol up front, in
        // declaration order -- the order `strongly_connected_components`
        // uses for determinism. Non-`Fn` items don't need any of the
        // grouping/generalization machinery below (the checker has
        // nothing to do for them today), so they're skipped entirely,
        // exactly as they were before.
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
                let mut collector = CallGraphCollector {
                    sibling_names: &sibling_names,
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

    /// Checks one `fn` item's body against its already-lowered
    /// signature. Deliberately does *not* generalize `symbol` itself
    /// -- that happens once for the whole strongly-connected
    /// component `symbol` belongs to, back in [`Self::check_items`],
    /// only after every member of that component (this one included)
    /// has had its body checked.
    fn check_fn_body(&mut self, symbol: SymbolId, item: &Item) {
        let ItemKind::Fn(f) = &item.kind else {
            return;
        };
        let scope = match &self.cx.symbols[symbol].kind {
            SymbolKind::Fn(scope) => *scope,
            _ => return,
        };
        let Some(body) = f.body.as_ref() else {
            return;
        };

        // The signature was already lowered and unified into the symbol's
        // type by `SignatureLowerer`. Pull it back apart here rather
        // than re-lowering it, so the body is checked against the exact
        // same (already-authoritative) input/output types.
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

        // Get the input types for the function.
        let resolved_inputs = self.cx.uni_cx.resolve(inputs_term);
        let input_tys = match self.cx.uni_cx.term(resolved_inputs) {
            Some(Term::App {
                constructor: TyCon::Tuple,
                args,
            }) => args.clone(),
            _ => return,
        };

        // Marks `symbol` as an open ancestor for as long as its body
        // (and anything nested inside it) is being checked, so nested
        // generalization knows not to steal a variable that's still
        // free in this signature -- see `enclosing_free_vars`.
        self.cx.checking_stack.push(symbol);

        // Enter the scope of the function body.
        self.with_scope(scope, |this| {
            // Here, we take the pattern of the parameter, which is the
            // binding for that parameter. check_pat here requires that
            // the pattern's shape fits input_ty, and declares a local
            // symbol for every name the pattern binds.
            for (param, input_ty) in f.sig.inputs.iter().zip(&input_tys) {
                this.cx.check_pat(&param.pat, *input_ty);
            }

            // Type-check the body of the function, with the constraint
            // that the body of the function must return a value of type
            // consistent with the return type of the function.
            let output_span = match &f.sig.output {
                FnRetTy::Default(span) => *span,
                FnRetTy::Ty(ty) => ty.span,
            };
            this.cx
                .check_block_expecting(body, Some(output_term), Some(output_span));

            // Recurse into any items hoisted into this fn's own body, as
            // their own sibling group, so nested fns get their bodies
            // checked (and, where they're mutually recursive with each
            // other, generalized together) too.
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

#[cfg(test)]
mod tests {
    use super::*;
    use chumsky::Parser;

    fn resolve(source: &str) -> TypeCheckContext {
        let tokens = lexer::tokenize_all(source).expect("should lex");
        let items = parser::module()
            .parse(parser::input(tokens))
            .into_result()
            .expect("should parse");

        let mut cx = TypeCheckContext::new();
        cx.resolve(&items);
        cx
    }

    fn resolve_and_lower(source: &str) -> TypeCheckContext {
        let tokens = lexer::tokenize_all(source).expect("should lex");
        let items = parser::module()
            .parse(parser::input(tokens))
            .into_result()
            .expect("should parse");

        let mut cx = TypeCheckContext::new();
        cx.resolve(&items);
        cx.lower_signatures(&items);
        cx
    }

    fn lookup(cx: &TypeCheckContext, scope: ScopeId, namespace: Namespace, name: &str) -> bool {
        let Some(&name) = cx.names.ids.get(name) else {
            return false;
        };
        let map = match namespace {
            Namespace::Type => &cx.scopes[scope].types,
            Namespace::Value => &cx.scopes[scope].values,
        };
        map.contains_key(&name)
    }

    fn path(segments: &[&str]) -> Path {
        let dummy_span = ast::Span { start: 0, end: 0 };
        Path {
            segments: segments
                .iter()
                .map(|name| ast::PathSegment {
                    ident: ast::Ident {
                        name: name.to_string(),
                        span: dummy_span,
                    },
                    args: None,
                })
                .collect(),
            span: dummy_span,
        }
    }

    fn expr(source: &str) -> Expr {
        let tokens = lexer::tokenize_all(source).expect("should lex");
        parser::expr()
            .parse(parser::input(tokens))
            .into_result()
            .expect("should parse")
    }

    fn ty(source: &str) -> Ty {
        let tokens = lexer::tokenize_all(source).expect("should lex");
        parser::ty()
            .parse(parser::input(tokens))
            .into_result()
            .expect("should parse")
    }

    fn pat(source: &str) -> Pat {
        let tokens = lexer::tokenize_all(source).expect("should lex");
        parser::pat(parser::expr())
            .parse(parser::input(tokens))
            .into_result()
            .expect("should parse")
    }

    fn block(source: &str) -> Block {
        let tokens = lexer::tokenize_all(source).expect("should lex");
        parser::block(parser::expr())
            .parse(parser::input(tokens))
            .into_result()
            .expect("should parse")
    }

    /// Parses a single `let` statement -- `parser`'s own `local`/`stmt`
    /// parsers aren't public, so this just wraps `source` in a block and
    /// pulls the one statement back out.
    fn local(source: &str) -> Local {
        let mut blk = block(&format!("{{ {source} }}"));
        let stmt = blk.stmts.remove(0);
        let StmtKind::Let(local) = stmt.kind else {
            panic!("expected a let statement, got {:?}", stmt.kind);
        };
        *local
    }

    /// Resolves `term` and, if it's an application, returns its
    /// constructor and (cloned) argument ids.
    fn resolved_args(cx: &mut TypeCheckContext, term: TermId) -> Option<(TyCon, Vec<TermId>)> {
        let resolved = cx.uni_cx.resolve(term);
        match cx.uni_cx.term(resolved) {
            Some(Term::App { constructor, args }) => Some((constructor.clone(), args.clone())),
            _ => None,
        }
    }

    fn resolved_con(cx: &mut TypeCheckContext, term: TermId) -> Option<TyCon> {
        resolved_args(cx, term).map(|(con, _)| con)
    }

    fn declared_symbol(
        cx: &TypeCheckContext,
        scope: ScopeId,
        namespace: Namespace,
        name: &str,
    ) -> Option<SymbolId> {
        let name = *cx.names.ids.get(name)?;
        let map = match namespace {
            Namespace::Type => &cx.scopes[scope].types,
            Namespace::Value => &cx.scopes[scope].values,
        };
        map.get(&name).copied()
    }

    #[test]
    fn declares_a_free_fn_in_the_value_namespace() {
        let cx = resolve("fn bar() {}");
        assert!(lookup(&cx, cx.current_scope, Namespace::Value, "bar"));
    }

    #[test]
    fn declares_a_named_struct_only_in_the_type_namespace() {
        let cx = resolve("struct Foo { x: int }");
        assert!(lookup(&cx, cx.current_scope, Namespace::Type, "Foo"));
        assert!(!lookup(&cx, cx.current_scope, Namespace::Value, "Foo"));
    }

    #[test]
    fn declares_a_tuple_struct_in_both_namespaces() {
        let cx = resolve("struct Foo(int);");
        assert!(lookup(&cx, cx.current_scope, Namespace::Type, "Foo"));
        assert!(lookup(&cx, cx.current_scope, Namespace::Value, "Foo"));
    }

    #[test]
    fn a_struct_and_a_fn_can_share_a_name() {
        let cx = resolve("struct Foo { x: int } fn Foo() {}");
        assert!(lookup(&cx, cx.current_scope, Namespace::Type, "Foo"));
        assert!(lookup(&cx, cx.current_scope, Namespace::Value, "Foo"));
    }

    #[test]
    fn a_mod_gets_its_own_child_scope() {
        let cx = resolve("mod m { fn baz() {} }");
        assert!(lookup(&cx, cx.current_scope, Namespace::Type, "m"));
        assert!(!lookup(&cx, cx.current_scope, Namespace::Value, "baz"));

        let child_scope = cx
            .scopes
            .iter()
            .find_map(|(id, scope)| (scope.parent == Some(cx.current_scope)).then_some(id))
            .expect("mod should have created a child scope");
        assert!(lookup(&cx, child_scope, Namespace::Value, "baz"));
    }

    #[test]
    fn an_item_nested_inside_a_fn_body_is_hoisted_into_its_own_scope() {
        let cx = resolve("fn outer() { fn inner() {} }");
        assert!(lookup(&cx, cx.current_scope, Namespace::Value, "outer"));
        assert!(!lookup(&cx, cx.current_scope, Namespace::Value, "inner"));

        let body_scope = cx
            .scopes
            .iter()
            .find_map(|(id, scope)| (scope.parent == Some(cx.current_scope)).then_some(id))
            .expect("the fn body should have created a child scope");
        assert!(lookup(&cx, body_scope, Namespace::Value, "inner"));
    }

    #[test]
    fn resolve_path_finds_a_single_segment_name() {
        let mut cx = resolve("struct Foo { x: int }");
        assert!(cx.resolve_path(&path(&["Foo"]), Namespace::Type).is_some());
    }

    #[test]
    fn resolve_path_fails_on_an_undeclared_name() {
        let mut cx = resolve("struct Foo { x: int }");
        assert!(cx.resolve_path(&path(&["Bar"]), Namespace::Type).is_none());
    }

    #[test]
    fn resolve_path_checks_the_requested_namespace() {
        // Foo is type-only (named struct) -- looking it up as a value
        // should fail even though the name itself is declared.
        let mut cx = resolve("struct Foo { x: int }");
        assert!(cx.resolve_path(&path(&["Foo"]), Namespace::Value).is_none());
    }

    #[test]
    fn resolve_path_walks_through_a_module() {
        let mut cx = resolve("mod m { fn baz() {} }");
        let resolved = cx.resolve_path(&path(&["m", "baz"]), Namespace::Value);
        assert!(resolved.is_some());
    }

    #[test]
    fn resolve_path_rejects_walking_through_a_non_module_segment() {
        let mut cx = resolve("struct Foo { x: int } fn bar() {}");
        // Foo isn't a module, so Foo::bar can't mean anything.
        assert!(
            cx.resolve_path(&path(&["Foo", "bar"]), Namespace::Value)
                .is_none()
        );
    }

    #[test]
    fn resolve_path_module_segment_is_looked_up_by_namespace_not_by_name_alone() {
        // "m" the type-namespace mod and a hypothetical value-namespace
        // "m" are different names as far as lookup is concerned --
        // resolving "m::baz" must go through the module, not get
        // confused by namespace.
        let mut cx = resolve("mod m { fn baz() {} } fn m() {}");
        assert!(
            cx.resolve_path(&path(&["m", "baz"]), Namespace::Value)
                .is_some()
        );
    }

    // -- lower_ty ---------------------------------------------------------

    #[test]
    fn lower_ty_never() {
        let mut cx = TypeCheckContext::new();
        let t = cx.lower_ty(&ty("!"));
        assert_eq!(resolved_con(&mut cx, t), Some(TyCon::Never));
    }

    #[test]
    fn lower_ty_paren_unwraps_to_the_inner_type() {
        let mut cx = TypeCheckContext::new();
        let t = cx.lower_ty(&ty("(!)"));
        assert_eq!(resolved_con(&mut cx, t), Some(TyCon::Never));
    }

    #[test]
    fn lower_ty_array_wraps_the_element_type() {
        let mut cx = TypeCheckContext::new();
        let t = cx.lower_ty(&ty("[!]"));
        let (con, args) = resolved_args(&mut cx, t).expect("should be an App term");
        assert_eq!(con, TyCon::Array);
        assert_eq!(args.len(), 1);
        assert_eq!(resolved_con(&mut cx, args[0]), Some(TyCon::Never));
    }

    #[test]
    fn lower_ty_tup_builds_one_arg_per_element() {
        let mut cx = TypeCheckContext::new();
        let t = cx.lower_ty(&ty("(!, !)"));
        let (con, args) = resolved_args(&mut cx, t).expect("should be an App term");
        assert_eq!(con, TyCon::Tuple);
        assert_eq!(args.len(), 2);
    }

    #[test]
    fn lower_ty_unit_is_a_zero_arg_tuple() {
        let mut cx = TypeCheckContext::new();
        let t = cx.lower_ty(&ty("()"));
        let (con, args) = resolved_args(&mut cx, t).expect("should be an App term");
        assert_eq!(con, TyCon::Tuple);
        assert!(args.is_empty());
    }

    #[test]
    fn lower_ty_fn_with_explicit_return_type() {
        let mut cx = TypeCheckContext::new();
        let t = cx.lower_ty(&ty("Fn(!) -> !"));
        let (con, args) = resolved_args(&mut cx, t).expect("should be an App term");
        assert_eq!(con, TyCon::Fn);
        assert_eq!(args.len(), 2);
        assert_eq!(resolved_con(&mut cx, args[1]), Some(TyCon::Never));
    }

    #[test]
    fn lower_ty_fn_with_no_return_type_defaults_to_a_fresh_unbound_var() {
        let mut cx = TypeCheckContext::new();
        let t = cx.lower_ty(&ty("Fn(!)"));
        let (_, args) = resolved_args(&mut cx, t).expect("should be an App term");
        let resolved = cx.uni_cx.resolve(args[1]);
        assert!(matches!(cx.uni_cx.term(resolved), Some(Term::Var(_))));
    }

    #[test]
    fn lower_ty_infer_produces_a_fresh_unbound_var() {
        let mut cx = TypeCheckContext::new();
        let t = cx.lower_ty(&ty("_"));
        let resolved = cx.uni_cx.resolve(t);
        assert!(matches!(cx.uni_cx.term(resolved), Some(Term::Var(_))));
    }

    #[test]
    fn lower_ty_err_is_a_wildcard_that_unifies_with_anything() {
        let mut cx = TypeCheckContext::new();
        let err_ty = Ty {
            kind: TyKind::Err,
            span: ast::Span { start: 0, end: 0 },
        };
        let err_term = cx.lower_ty(&err_ty);
        let int_term = term!(cx.uni_cx, TyCon::Int);
        assert!(cx.uni_cx.unify(err_term, int_term).is_ok());
    }

    #[test]
    fn lower_ty_path_resolves_primitive_names() {
        let cases = [
            ("bool", TyCon::Bool),
            ("int", TyCon::Int),
            ("float", TyCon::Float),
            ("char", TyCon::Char),
            ("String", TyCon::Str),
        ];
        for (src, expected) in cases {
            let mut cx = TypeCheckContext::new();
            let t = cx.lower_ty(&ty(src));
            assert_eq!(resolved_con(&mut cx, t), Some(expected), "input: {src}");
        }
    }

    #[test]
    fn lower_ty_path_resolves_a_declared_struct_by_nominal_identity() {
        let mut cx = resolve("struct Foo { x: int }");
        let symbol = cx
            .resolve_path(&path(&["Foo"]), Namespace::Type)
            .expect("Foo should resolve");

        let t = cx.lower_ty(&ty("Foo"));
        assert_eq!(resolved_con(&mut cx, t), Some(TyCon::Struct(symbol)));
    }

    #[test]
    fn lower_ty_path_resolves_a_declared_enum_by_nominal_identity() {
        let mut cx = resolve("enum Foo { Bar }");
        let symbol = cx
            .resolve_path(&path(&["Foo"]), Namespace::Type)
            .expect("Foo should resolve");

        let t = cx.lower_ty(&ty("Foo"));
        assert_eq!(resolved_con(&mut cx, t), Some(TyCon::Enum(symbol)));
    }

    #[test]
    fn lower_ty_path_to_an_undeclared_name_is_err() {
        let mut cx = TypeCheckContext::new();
        let t = cx.lower_ty(&ty("DoesNotExist"));
        assert_eq!(resolved_con(&mut cx, t), Some(TyCon::Err));
    }

    #[test]
    fn lower_ty_path_walks_through_a_module() {
        let mut cx = resolve("mod m { struct Foo; }");
        let symbol = cx
            .resolve_path(&path(&["m", "Foo"]), Namespace::Type)
            .expect("m::Foo should resolve");

        let t = cx.lower_ty(&ty("m::Foo"));
        assert_eq!(resolved_con(&mut cx, t), Some(TyCon::Struct(symbol)));
    }

    // -- check_expr ---------------------------------------------------------

    #[test]
    fn check_expr_bool_literal() {
        let mut cx = TypeCheckContext::new();
        let t = cx.check_expr(&expr("true"), None);
        assert_eq!(resolved_con(&mut cx, t), Some(TyCon::Bool));
        let t = cx.check_expr(&expr("false"), None);
        assert_eq!(resolved_con(&mut cx, t), Some(TyCon::Bool));
    }

    #[test]
    fn check_expr_int_literal() {
        let mut cx = TypeCheckContext::new();
        let t = cx.check_expr(&expr("5"), None);
        assert_eq!(resolved_con(&mut cx, t), Some(TyCon::Int));
    }

    #[test]
    fn check_expr_float_literal() {
        let mut cx = TypeCheckContext::new();
        let t = cx.check_expr(&expr("5.0"), None);
        assert_eq!(resolved_con(&mut cx, t), Some(TyCon::Float));
    }

    #[test]
    fn check_expr_str_literal() {
        let mut cx = TypeCheckContext::new();
        let t = cx.check_expr(&expr("\"hi\""), None);
        assert_eq!(resolved_con(&mut cx, t), Some(TyCon::Str));
    }

    #[test]
    fn check_expr_char_literal() {
        let mut cx = TypeCheckContext::new();
        let t = cx.check_expr(&expr("'a'"), None);
        assert_eq!(resolved_con(&mut cx, t), Some(TyCon::Char));
    }

    #[test]
    fn check_expr_paren_has_the_inner_exprs_type() {
        let mut cx = TypeCheckContext::new();
        let t = cx.check_expr(&expr("(5)"), None);
        assert_eq!(resolved_con(&mut cx, t), Some(TyCon::Int));
    }

    #[test]
    fn check_expr_err_is_a_wildcard() {
        let mut cx = TypeCheckContext::new();
        let err_expr = Expr {
            annotations: Vec::new(),
            kind: ExprKind::Err,
            span: ast::Span { start: 0, end: 0 },
        };
        let bool_term = term!(cx.uni_cx, TyCon::Bool);
        // Should not panic -- unifying against an unrelated expected
        // type has to succeed silently, not fail.
        let t = cx.check_expr(&err_expr, Some(bool_term));
        assert_eq!(resolved_con(&mut cx, t), Some(TyCon::Err));
    }

    #[test]
    fn check_expr_unifies_the_result_against_the_expected_type() {
        let mut cx = resolve("fn foo() {}");
        let symbol = cx
            .resolve_path(&path(&["foo"]), Namespace::Value)
            .expect("foo should resolve");
        let symbol_ty = cx.symbols[symbol].ty;

        let never_term = term!(cx.uni_cx, TyCon::Never);
        cx.check_expr(&expr("foo"), Some(never_term));

        // foo's symbol type starts out as an unbound fresh var -- if
        // check_expr's final unify against `expected` never ran, this
        // would still be unbound.
        assert_eq!(resolved_con(&mut cx, symbol_ty), Some(TyCon::Never));
    }

    #[test]
    fn check_expr_tup_elements_keep_independent_types() {
        let mut cx = TypeCheckContext::new();
        let t = cx.check_expr(&expr("(1, \"hi\")"), None);
        let (con, args) = resolved_args(&mut cx, t).expect("should be an App term");
        assert_eq!(con, TyCon::Tuple);
        assert_eq!(resolved_con(&mut cx, args[0]), Some(TyCon::Int));
        assert_eq!(resolved_con(&mut cx, args[1]), Some(TyCon::Str));
    }

    #[test]
    fn check_expr_array_elements_are_unified_with_each_other() {
        let mut cx = TypeCheckContext::new();
        let t = cx.check_expr(&expr("[1, 2, 3]"), None);
        let (con, args) = resolved_args(&mut cx, t).expect("should be an App term");
        assert_eq!(con, TyCon::Array);
        assert_eq!(resolved_con(&mut cx, args[0]), Some(TyCon::Int));
    }

    #[test]
    fn check_expr_empty_array_uses_the_expected_element_type() {
        let mut cx = TypeCheckContext::new();
        let never_term = term!(cx.uni_cx, TyCon::Never);
        let array_of_never = term!(cx.uni_cx, TyCon::Array => [never_term]);

        let t = cx.check_expr(&expr("[]"), Some(array_of_never));
        let (_, args) = resolved_args(&mut cx, t).expect("should be an App term");
        assert_eq!(resolved_con(&mut cx, args[0]), Some(TyCon::Never));
    }

    #[test]
    fn check_expr_path_resolves_to_the_symbols_type() {
        let mut cx = resolve("fn foo() {}");
        let symbol = cx
            .resolve_path(&path(&["foo"]), Namespace::Value)
            .expect("foo should resolve");
        let symbol_ty = cx.symbols[symbol].ty;

        let t = cx.check_expr(&expr("foo"), None);
        assert_eq!(t, symbol_ty);
    }

    #[test]
    fn check_expr_path_to_an_undeclared_name_is_err() {
        let mut cx = TypeCheckContext::new();
        let t = cx.check_expr(&expr("doesNotExist"), None);
        assert_eq!(resolved_con(&mut cx, t), Some(TyCon::Err));
    }

    #[test]
    fn check_expr_cast_lowers_the_target_type() {
        let mut cx = TypeCheckContext::new();
        let t = cx.check_expr(&expr("5 as float"), None);
        assert_eq!(resolved_con(&mut cx, t), Some(TyCon::Float));
    }

    #[test]
    fn check_expr_call_pins_the_callees_type_to_a_fn_shape() {
        let mut cx = resolve("fn foo() {}");
        let symbol = cx
            .resolve_path(&path(&["foo"]), Namespace::Value)
            .expect("foo should resolve");
        let symbol_ty = cx.symbols[symbol].ty;

        cx.check_expr(&expr("foo()"), None);

        // foo's symbol type started as an unbound var -- calling it is
        // what has to pin it down to a concrete Fn shape.
        let (con, _) = resolved_args(&mut cx, symbol_ty).expect("should be an App term");
        assert_eq!(con, TyCon::Fn);
    }

    #[test]
    fn check_expr_call_checks_arguments_against_the_signature() {
        let mut cx = resolve("fn foo() {}");
        cx.check_expr(&expr("foo(5)"), None);

        let symbol = cx
            .resolve_path(&path(&["foo"]), Namespace::Value)
            .expect("foo should resolve");
        let symbol_ty = cx.symbols[symbol].ty;

        let (_, fn_args) = resolved_args(&mut cx, symbol_ty).expect("should be a Fn term");
        let (_, input_args) = resolved_args(&mut cx, fn_args[0]).expect("should be a Tuple term");
        assert_eq!(resolved_con(&mut cx, input_args[0]), Some(TyCon::Int));
    }

    #[test]
    fn check_all_calling_an_annotated_non_fn_parameter_is_an_error() {
        let source = r#"
fn use_it(g: int) {
    g(1);
}
"#;
        let cx = check_all(source);
        let diagnostics = cx.diagnostics();
        assert_eq!(diagnostics.len(), 1, "{diagnostics:#?}");

        let d = &diagnostics[0];
        assert_eq!(d.message, "expected a function, found `int`");
        // Points at the callee (`g`), not the whole call expression.
        assert_eq!(&source[d.primary_span.start..d.primary_span.end], "g");
    }

    #[test]
    fn check_all_calling_a_locally_inferred_non_fn_value_is_an_error() {
        // Same underlying bug as the annotated-parameter case, but
        // reached the other way `check_expr_kind`'s `Call` handling can
        // find a callee whose type is already concretely known: through
        // ordinary local inference rather than an explicit annotation.
        let source = r#"
fn use_it() {
    let g = 5;
    g(1);
}
"#;
        let cx = check_all(source);
        let diagnostics = cx.diagnostics();
        assert_eq!(diagnostics.len(), 1, "{diagnostics:#?}");
        assert_eq!(diagnostics[0].message, "expected a function, found `int`");
    }

    #[test]
    fn check_all_calling_an_unannotated_parameter_infers_its_fn_shape_with_no_error() {
        // The positive case the two tests above are contrasted with:
        // when the callee's type genuinely isn't known yet (`f` here has
        // no annotation), calling it is what pins it down to a Fn shape
        // -- not an error. This is the same mechanism `compose` in
        // crates/cli/examples/generics.roo relies on for `f`/`g`.
        let source = r#"
fn apply(f, x) {
    f(x)
}
"#;
        let mut cx = check_all(source);
        assert!(cx.diagnostics().is_empty(), "{:#?}", cx.diagnostics());

        let apply = cx
            .resolve_path(&path(&["apply"]), Namespace::Value)
            .expect("apply should resolve");
        assert_eq!(cx.symbols[apply].generics.len(), 2, "<T, U> Fn(Fn(T) -> U, T) -> U");
    }

    #[test]
    fn check_expr_call_result_is_an_unbound_var_when_nothing_constrains_it() {
        let mut cx = resolve("fn foo() {}");
        let t = cx.check_expr(&expr("foo()"), None);
        let resolved = cx.uni_cx.resolve(t);
        assert!(matches!(cx.uni_cx.term(resolved), Some(Term::Var(_))));
    }

    #[test]
    fn check_expr_ret_with_no_value_is_never() {
        let mut cx = TypeCheckContext::new();
        let t = cx.check_expr(&expr("return"), None);
        assert_eq!(resolved_con(&mut cx, t), Some(TyCon::Never));
    }

    #[test]
    fn check_expr_ret_with_a_value_is_still_never_not_the_values_type() {
        // `return`'s own type is Never regardless of what it returns --
        // control never continues past it, so nothing downstream can
        // ever actually see the returned value's type.
        let mut cx = TypeCheckContext::new();
        let t = cx.check_expr(&expr("return 5"), None);
        assert_eq!(resolved_con(&mut cx, t), Some(TyCon::Never));
    }

    #[test]
    fn never_is_a_wildcard_that_unifies_with_anything() {
        let mut cx = TypeCheckContext::new();
        let never_term = term!(cx.uni_cx, TyCon::Never);
        let int_term = term!(cx.uni_cx, TyCon::Int);
        assert!(cx.uni_cx.unify(never_term, int_term).is_ok());
    }

    #[test]
    fn if_with_no_else_and_a_unit_then_branch_is_unit_typed() {
        let mut cx = TypeCheckContext::new();
        let t = cx.check_expr(&expr("if true { }"), None);
        assert_eq!(resolved_con(&mut cx, t), Some(TyCon::Tuple));
    }

    #[test]
    fn if_branches_are_unified_together() {
        let mut cx = TypeCheckContext::new();
        let t = cx.check_expr(&expr("if true { 1 } else { 2 }"), None);
        assert_eq!(resolved_con(&mut cx, t), Some(TyCon::Int));
    }

    #[test]
    fn if_prefers_the_else_branchs_type_when_the_then_branch_diverges() {
        let mut cx = TypeCheckContext::new();
        let t = cx.check_expr(&expr("if true { return } else { 5 }"), None);
        assert_eq!(resolved_con(&mut cx, t), Some(TyCon::Int));
    }

    #[test]
    fn if_prefers_the_then_branchs_type_when_the_else_branch_diverges() {
        let mut cx = TypeCheckContext::new();
        let t = cx.check_expr(&expr("if true { 5 } else { return }"), None);
        assert_eq!(resolved_con(&mut cx, t), Some(TyCon::Int));
    }

    #[test]
    fn if_is_never_when_both_branches_diverge() {
        let mut cx = TypeCheckContext::new();
        let t = cx.check_expr(&expr("if true { return } else { return }"), None);
        assert_eq!(resolved_con(&mut cx, t), Some(TyCon::Never));
    }

    #[test]
    fn if_prefers_the_then_branchs_type_when_the_else_branch_diverges_via_a_semicolon() {
        // Regression test: `return 0;` (with a trailing semicolon) is a
        // Semi statement, not a bare tail Expr, so check_block used to
        // fall through to its default Tuple/unit type instead of
        // propagating the statement's Never type -- making this `if`
        // wrongly fail to unify `int` against `()`.
        let mut cx = TypeCheckContext::new();
        let t = cx.check_expr(&expr("if true { 5 } else { return 0; }"), None);
        assert_eq!(resolved_con(&mut cx, t), Some(TyCon::Int));
    }

    // -- check_block --------------------------------------------------------

    #[test]
    fn check_block_empty_is_unit() {
        let mut cx = TypeCheckContext::new();
        let t = cx.check_block(&block("{}"), None);
        assert_eq!(resolved_con(&mut cx, t), Some(TyCon::Tuple));
    }

    #[test]
    fn check_block_trailing_expr_with_no_semicolon_is_its_type() {
        let mut cx = TypeCheckContext::new();
        let t = cx.check_block(&block("{ 5 }"), None);
        assert_eq!(resolved_con(&mut cx, t), Some(TyCon::Int));
    }

    #[test]
    fn check_block_trailing_expr_with_a_semicolon_does_not_count() {
        let mut cx = TypeCheckContext::new();
        // The semicolon makes this a Semi statement, not a trailing
        // Expr -- check_block should treat it the same as an empty block.
        let t = cx.check_block(&block("{ 5; }"), None);
        assert_eq!(resolved_con(&mut cx, t), Some(TyCon::Tuple));
    }

    #[test]
    fn check_block_a_semicolon_terminated_return_makes_the_block_never() {
        let mut cx = TypeCheckContext::new();
        let t = cx.check_block(&block("{ return 0; }"), None);
        assert_eq!(resolved_con(&mut cx, t), Some(TyCon::Never));
    }

    #[test]
    fn check_block_a_non_trailing_let_declares_a_symbol_visible_to_later_statements() {
        let mut cx = TypeCheckContext::new();
        // Regression test: check_block used to only ever look at the last
        // statement, so this `let` was never checked and `x` was never
        // declared -- the trailing reference to it would've resolved as
        // Err (see check_expr_path_to_an_undeclared_name_is_err).
        let t = cx.check_block(&block("{ let x = 5; x }"), None);
        assert_eq!(resolved_con(&mut cx, t), Some(TyCon::Int));
    }

    #[test]
    fn check_block_a_non_trailing_lets_ascription_propagates_to_a_later_reference() {
        let mut cx = TypeCheckContext::new();
        let t = cx.check_block(&block("{ let x: float; let y = x; y }"), None);
        assert_eq!(resolved_con(&mut cx, t), Some(TyCon::Float));
    }

    // -- check_pat ------------------------------------------------------------

    #[test]
    fn check_pat_ident_declares_a_local_symbol() {
        let mut cx = TypeCheckContext::new();
        let never_term = term!(cx.uni_cx, TyCon::Never);
        cx.check_pat(&pat("x"), never_term);

        assert!(lookup(&cx, cx.current_scope, Namespace::Value, "x"));
    }

    #[test]
    fn check_pat_ident_binds_the_locals_type_to_expected() {
        let mut cx = TypeCheckContext::new();
        let never_term = term!(cx.uni_cx, TyCon::Never);
        cx.check_pat(&pat("x"), never_term);

        let symbol = declared_symbol(&cx, cx.current_scope, Namespace::Value, "x")
            .expect("x should be declared");
        let symbol_ty = cx.symbols[symbol].ty;
        assert_eq!(resolved_con(&mut cx, symbol_ty), Some(TyCon::Never));
    }

    #[test]
    fn check_pat_wild_matches_anything_and_binds_nothing() {
        let mut cx = TypeCheckContext::new();
        let never_term = term!(cx.uni_cx, TyCon::Never);
        let t = cx.check_pat(&pat("_"), never_term);
        assert_eq!(t, never_term);
        assert!(cx.symbols.is_empty());
    }

    #[test]
    fn check_pat_tuple_declares_one_local_per_position() {
        let mut cx = TypeCheckContext::new();
        let never_term = term!(cx.uni_cx, TyCon::Never);
        let int_term = term!(cx.uni_cx, TyCon::Int);
        let expected = term!(cx.uni_cx, TyCon::Tuple => [never_term, int_term]);

        cx.check_pat(&pat("(a, b)"), expected);

        let a = declared_symbol(&cx, cx.current_scope, Namespace::Value, "a")
            .expect("a should be declared");
        let b = declared_symbol(&cx, cx.current_scope, Namespace::Value, "b")
            .expect("b should be declared");
        let a_ty = cx.symbols[a].ty;
        let b_ty = cx.symbols[b].ty;
        assert_eq!(resolved_con(&mut cx, a_ty), Some(TyCon::Never));
        assert_eq!(resolved_con(&mut cx, b_ty), Some(TyCon::Int));
    }

    #[test]
    fn check_pat_tuple_with_no_matching_expected_shape_uses_fresh_vars_per_position() {
        let mut cx = TypeCheckContext::new();
        let int_term = term!(cx.uni_cx, TyCon::Int);
        // `expected` isn't a 2-tuple, so each position gets its own
        // fresh var instead of panicking or forcing the wrong shape.
        let t = cx.check_pat(&pat("(a, b)"), int_term);
        let (con, args) = resolved_args(&mut cx, t).expect("should be an App term");
        assert_eq!(con, TyCon::Tuple);
        assert_eq!(args.len(), 2);
    }

    // -- check_local ----------------------------------------------------------

    #[test]
    fn check_local_declares_the_pattern_with_the_initializers_type() {
        let mut cx = TypeCheckContext::new();
        cx.check_local(&local("let x = 5;"));

        let symbol = declared_symbol(&cx, cx.current_scope, Namespace::Value, "x")
            .expect("x should be declared");
        let symbol_ty = cx.symbols[symbol].ty;
        assert_eq!(resolved_con(&mut cx, symbol_ty), Some(TyCon::Int));
    }

    #[test]
    fn check_local_with_no_initializer_uses_the_ascription() {
        let mut cx = TypeCheckContext::new();
        cx.check_local(&local("let x: !;"));

        let symbol = declared_symbol(&cx, cx.current_scope, Namespace::Value, "x")
            .expect("x should be declared");
        let symbol_ty = cx.symbols[symbol].ty;
        assert_eq!(resolved_con(&mut cx, symbol_ty), Some(TyCon::Never));
    }

    #[test]
    fn check_local_ascription_constrains_the_initializer() {
        let mut cx = resolve("fn foo() {}");
        let symbol = cx
            .resolve_path(&path(&["foo"]), Namespace::Value)
            .expect("foo should resolve");

        cx.check_local(&local("let x: ! = foo();"));

        // The `: !` ascription should have flowed into checking the
        // initializer, which pins foo's own return type to Never too.
        let symbol_ty = cx.symbols[symbol].ty;
        let (_, fn_args) = resolved_args(&mut cx, symbol_ty).expect("should be a Fn term");
        assert_eq!(resolved_con(&mut cx, fn_args[1]), Some(TyCon::Never));
    }

    // -- lower_signatures -------------------------------------------------

    #[test]
    fn lower_signatures_fn_with_typed_params_and_return() {
        let mut cx = resolve_and_lower("fn add(a: int, b: int) -> float { a }");
        let symbol = cx
            .resolve_path(&path(&["add"]), Namespace::Value)
            .expect("add should resolve");
        let symbol_ty = cx.symbols[symbol].ty;

        let (con, args) = resolved_args(&mut cx, symbol_ty).expect("should be a Fn term");
        assert_eq!(con, TyCon::Fn);
        let (_, input_args) = resolved_args(&mut cx, args[0]).expect("should be a Tuple term");
        assert_eq!(resolved_con(&mut cx, input_args[0]), Some(TyCon::Int));
        assert_eq!(resolved_con(&mut cx, input_args[1]), Some(TyCon::Int));
        assert_eq!(resolved_con(&mut cx, args[1]), Some(TyCon::Float));
    }

    #[test]
    fn lower_signatures_fn_with_no_return_type_is_a_fresh_unbound_var() {
        let mut cx = resolve_and_lower("fn foo() {}");
        let symbol = cx
            .resolve_path(&path(&["foo"]), Namespace::Value)
            .expect("foo should resolve");
        let symbol_ty = cx.symbols[symbol].ty;

        let (_, args) = resolved_args(&mut cx, symbol_ty).expect("should be a Fn term");
        let resolved = cx.uni_cx.resolve(args[1]);
        assert!(matches!(cx.uni_cx.term(resolved), Some(Term::Var(_))));
    }

    #[test]
    fn lower_signatures_fn_with_an_untyped_param_gets_a_fresh_var() {
        let mut cx = resolve_and_lower("fn foo(x) {}");
        let symbol = cx
            .resolve_path(&path(&["foo"]), Namespace::Value)
            .expect("foo should resolve");
        let symbol_ty = cx.symbols[symbol].ty;

        let (_, args) = resolved_args(&mut cx, symbol_ty).expect("should be a Fn term");
        let (_, input_args) = resolved_args(&mut cx, args[0]).expect("should be a Tuple term");
        let resolved = cx.uni_cx.resolve(input_args[0]);
        assert!(matches!(cx.uni_cx.term(resolved), Some(Term::Var(_))));
    }

    #[test]
    fn lower_signatures_ty_alias() {
        let mut cx = resolve_and_lower("type MyInt = int;");
        let symbol = cx
            .resolve_path(&path(&["MyInt"]), Namespace::Type)
            .expect("MyInt should resolve");
        let symbol_ty = cx.symbols[symbol].ty;
        assert_eq!(resolved_con(&mut cx, symbol_ty), Some(TyCon::Int));
    }

    #[test]
    fn lower_signatures_recurses_into_a_fns_own_body() {
        let mut cx = resolve_and_lower("fn outer() { fn inner(x: int) -> bool { true } }");
        let body_scope = cx
            .scopes
            .iter()
            .find_map(|(id, scope)| (scope.parent == Some(cx.current_scope)).then_some(id))
            .expect("outer's body should have a child scope");
        let symbol = declared_symbol(&cx, body_scope, Namespace::Value, "inner")
            .expect("inner should be declared");
        let symbol_ty = cx.symbols[symbol].ty;
        let (con, _) = resolved_args(&mut cx, symbol_ty).expect("should be a Fn term");
        assert_eq!(con, TyCon::Fn);
    }

    #[test]
    fn lower_signatures_recurses_into_a_mod() {
        let mut cx = resolve_and_lower("mod m { fn baz(x: bool) {} }");
        let m_symbol = cx
            .resolve_path(&path(&["m"]), Namespace::Type)
            .expect("m should resolve");
        let SymbolKind::Mod(m_scope) = &cx.symbols[m_symbol].kind else {
            panic!("m should be a Mod symbol");
        };
        let m_scope = *m_scope;

        let symbol =
            declared_symbol(&cx, m_scope, Namespace::Value, "baz").expect("baz should resolve");
        let symbol_ty = cx.symbols[symbol].ty;
        let (con, _) = resolved_args(&mut cx, symbol_ty).expect("should be a Fn term");
        assert_eq!(con, TyCon::Fn);
    }

    #[test]
    fn lower_signatures_makes_the_declared_signature_authoritative() {
        let mut cx = resolve_and_lower("fn foo(x: int) {}");
        let symbol = cx
            .resolve_path(&path(&["foo"]), Namespace::Value)
            .expect("foo should resolve");
        let symbol_ty = cx.symbols[symbol].ty;

        // A mismatched call must not corrupt the already-declared
        // signature -- the declaration stays authoritative even when a
        // caller disagrees with it, unlike the pre-signature-lowering
        // "first caller wins" behavior.
        cx.check_expr(&expr("foo(\"wrong\")"), None);

        let (_, args) = resolved_args(&mut cx, symbol_ty).expect("should still be a Fn term");
        let (_, input_args) =
            resolved_args(&mut cx, args[0]).expect("should still be a Tuple term");
        assert_eq!(resolved_con(&mut cx, input_args[0]), Some(TyCon::Int));
    }

    // -- Checker (fn bodies) ---------------------------------------------

    fn check_all(source: &str) -> TypeCheckContext {
        let tokens = lexer::tokenize_all(source).expect("should lex");
        let items = parser::module()
            .parse(parser::input(tokens))
            .into_result()
            .expect("should parse");

        let mut cx = TypeCheckContext::new();
        cx.resolve(&items);
        cx.lower_signatures(&items);
        cx.check(&items);
        cx
    }

    fn fn_body_scope(cx: &TypeCheckContext, symbol: SymbolId) -> ScopeId {
        match &cx.symbols[symbol].kind {
            SymbolKind::Fn(scope) => *scope,
            _ => panic!("expected a Fn symbol"),
        }
    }

    #[test]
    fn check_all_infers_an_untyped_params_type_from_the_bodys_declared_return_type() {
        let mut cx = check_all("fn identity(x) -> int { x }");
        let fn_symbol = cx
            .resolve_path(&path(&["identity"]), Namespace::Value)
            .expect("identity should resolve");
        let body_scope = fn_body_scope(&cx, fn_symbol);

        let x_symbol = declared_symbol(&cx, body_scope, Namespace::Value, "x")
            .expect("x should be declared as a param");
        let x_ty = cx.symbols[x_symbol].ty;
        assert_eq!(resolved_con(&mut cx, x_ty), Some(TyCon::Int));
    }

    #[test]
    fn check_all_recurses_into_a_nested_fns_body() {
        let mut cx = check_all("fn outer() { fn inner(x) -> int { x } }");
        let outer_symbol = cx
            .resolve_path(&path(&["outer"]), Namespace::Value)
            .expect("outer should resolve");
        let outer_scope = fn_body_scope(&cx, outer_symbol);

        let inner_symbol = declared_symbol(&cx, outer_scope, Namespace::Value, "inner")
            .expect("inner should be declared inside outer's body");
        let inner_scope = fn_body_scope(&cx, inner_symbol);

        let x_symbol = declared_symbol(&cx, inner_scope, Namespace::Value, "x")
            .expect("x should be declared as inner's param");
        let x_ty = cx.symbols[x_symbol].ty;
        assert_eq!(resolved_con(&mut cx, x_ty), Some(TyCon::Int));
    }

    #[test]
    fn check_all_call_reports_the_specific_mismatching_argument_not_the_whole_call() {
        let source = r#"
fn add(a: int, b: int) {}
fn main() {
    add("wrong", 5);
}
"#;
        let cx = check_all(source);
        let diagnostics = cx.diagnostics();
        assert_eq!(diagnostics.len(), 1, "{diagnostics:#?}");

        let d = &diagnostics[0];
        assert_eq!(
            &source[d.primary_span.start..d.primary_span.end],
            "\"wrong\""
        );
        assert_eq!(d.message, "expected `int`, found `String`");
    }

    #[test]
    fn check_all_call_mismatch_against_an_annotated_param_points_at_the_annotation() {
        let source = r#"
fn add(a: int, b: int) {}
fn main() {
    add("wrong", 5);
}
"#;
        let cx = check_all(source);
        let diagnostics = cx.diagnostics();
        assert_eq!(diagnostics.len(), 1, "{diagnostics:#?}");

        let d = &diagnostics[0];
        assert_eq!(d.related.len(), 1, "{:#?}", d.related);
        let (span, message) = &d.related[0];
        assert_eq!(&source[span.start..span.end], "int");
        assert_eq!(message, "expected due to this");
    }

    #[test]
    fn check_all_call_mismatch_against_an_unannotated_param_has_no_related_span() {
        // `x` has no type annotation of its own -- its expected type
        // (`int`) only comes from how the body of `takes_something`
        // happens to use it, not from anything written at the
        // parameter itself. Pointing "expected due to this" at the
        // bare parameter name anyway would be misleading (it looks
        // like an explanation but isn't one), so there should be no
        // related span at all rather than a low-quality one.
        let source = r#"
fn takes_something(x) {
    let y: int = x;
    x
}
fn use_it() {
    takes_something("wrong");
}
"#;
        let cx = check_all(source);
        let diagnostics = cx.diagnostics();
        assert_eq!(diagnostics.len(), 1, "{diagnostics:#?}");
        assert!(diagnostics[0].related.is_empty(), "{:#?}", diagnostics[0].related);
    }

    #[test]
    fn check_all_call_keeps_checking_later_arguments_after_an_earlier_one_mismatches() {
        let source = r#"
fn add(a: int, b: int) {}
fn main() {
    add("wrong1", "wrong2");
}
"#;
        let cx = check_all(source);
        let diagnostics = cx.diagnostics();
        assert_eq!(diagnostics.len(), 2, "{diagnostics:#?}");

        assert_eq!(
            &source[diagnostics[0].primary_span.start..diagnostics[0].primary_span.end],
            "\"wrong1\""
        );
        assert_eq!(
            &source[diagnostics[1].primary_span.start..diagnostics[1].primary_span.end],
            "\"wrong2\""
        );
    }

    #[test]
    fn check_all_call_reports_too_few_arguments() {
        let source = r#"
fn add(a: int, b: int) {}
fn main() {
    add(1);
}
"#;
        let cx = check_all(source);
        let diagnostics = cx.diagnostics();
        assert_eq!(diagnostics.len(), 1, "{diagnostics:#?}");

        let d = &diagnostics[0];
        assert_eq!(
            d.message,
            "this function takes 2 arguments but 1 argument was supplied"
        );
        // No closing-paren tracking, so the span runs from the callee
        // through the last argument given, not including `)`.
        assert_eq!(&source[d.primary_span.start..d.primary_span.end], "add(1");
    }

    #[test]
    fn check_all_call_reports_too_many_arguments_pointing_at_the_extra_ones() {
        let source = r#"
fn add(a: int, b: int) {}
fn main() {
    add(1, 2, 3);
}
"#;
        let cx = check_all(source);
        let diagnostics = cx.diagnostics();
        assert_eq!(diagnostics.len(), 1, "{diagnostics:#?}");

        let d = &diagnostics[0];
        assert_eq!(
            d.message,
            "this function takes 2 arguments but 3 arguments were supplied"
        );
        assert_eq!(&source[d.primary_span.start..d.primary_span.end], "3");
    }

    // -- strongly_connected_components ------------------------------------

    /// Mints `n` distinct [`SymbolId`]s with no [`TypeCheckContext`]
    /// attached, for exercising the graph algorithm on its own.
    fn symbol_ids(n: usize) -> Vec<SymbolId> {
        let mut map: SlotMap<SymbolId, ()> = SlotMap::with_key();
        (0..n).map(|_| map.insert(())).collect()
    }

    #[test]
    fn scc_a_chain_with_no_cycles_is_all_singletons_in_dependency_order() {
        let ids = symbol_ids(3);
        let (a, b, c) = (ids[0], ids[1], ids[2]);

        // a -> b -> c, no edges back -- not a cycle anywhere.
        let mut edges = HashMap::new();
        edges.insert(a, vec![b]);
        edges.insert(b, vec![c]);
        edges.insert(c, vec![]);

        let sccs = strongly_connected_components(&ids, &edges);

        // c depends on nothing, b depends on c, a depends on b -- so
        // that's the order components must come out in.
        assert_eq!(sccs, vec![vec![c], vec![b], vec![a]]);
    }

    #[test]
    fn scc_a_two_cycle_is_one_component() {
        let ids = symbol_ids(2);
        let (a, b) = (ids[0], ids[1]);

        let mut edges = HashMap::new();
        edges.insert(a, vec![b]);
        edges.insert(b, vec![a]);

        let sccs = strongly_connected_components(&ids, &edges);

        assert_eq!(sccs.len(), 1, "{sccs:?}");
        assert_eq!(sccs[0].len(), 2);
        assert!(sccs[0].contains(&a));
        assert!(sccs[0].contains(&b));
    }

    #[test]
    fn scc_three_way_cycle_is_one_component_that_pops_before_an_unrelated_caller() {
        let ids = symbol_ids(4);
        let (a, b, c, caller) = (ids[0], ids[1], ids[2], ids[3]);

        // a -> b -> c -> a is a genuine 3-cycle; caller -> a is a
        // separate, one-directional reference into it, not part of
        // the cycle itself.
        let mut edges = HashMap::new();
        edges.insert(a, vec![b]);
        edges.insert(b, vec![c]);
        edges.insert(c, vec![a]);
        edges.insert(caller, vec![a]);

        let sccs = strongly_connected_components(&ids, &edges);

        assert_eq!(sccs.len(), 2, "{sccs:?}");
        assert_eq!(sccs[0].len(), 3);
        for member in [a, b, c] {
            assert!(sccs[0].contains(&member));
        }
        // The cycle depends on nothing outside itself, so it must be
        // fully popped before `caller`, which depends on it.
        assert_eq!(sccs[1], vec![caller]);
    }

    // -- Checker: strongly-connected generalization -----------------------

    #[test]
    fn check_all_mutually_recursive_siblings_generalize_together_and_stay_reusable() {
        // The `if true { x } else { ... }` base case is what makes this
        // a useful test of `generalize_group`'s shared-variable handling
        // specifically: it forces each function's own parameter and
        // return type to unify into *one* variable, and that variable
        // is *also* shared across ping/pong via their mutual calls -- so
        // there's exactly one free variable across the whole component,
        // and both symbols' `generics` need to end up naming the same
        // id for it. (A version of this test without the `if` base case
        // -- just `fn ping(x) { pong(x) }` -- still generalizes
        // correctly, but produces *two* independent generics per
        // function, one for the parameter and one for the return, since
        // nothing ever forces those two positions to unify: it's the
        // same shape Standard ML/OCaml gives `let rec f x = f x`.)
        let source = r#"
fn ping(x) {
    if true { x } else { pong(x) }
}
fn pong(y) {
    if true { y } else { ping(y) }
}
fn use_both() {
    ping(1);
    pong("hi");
}
"#;
        let mut cx = check_all(source);
        assert!(cx.diagnostics().is_empty(), "{:#?}", cx.diagnostics());

        let ping = cx
            .resolve_path(&path(&["ping"]), Namespace::Value)
            .expect("ping should resolve");
        let pong = cx
            .resolve_path(&path(&["pong"]), Namespace::Value)
            .expect("pong should resolve");
        assert_eq!(cx.symbols[ping].generics.len(), 1);
        assert_eq!(cx.symbols[pong].generics.len(), 1);
        // Same shared variable, so the same id both times.
        assert_eq!(cx.symbols[ping].generics[0], cx.symbols[pong].generics[0]);
    }

    #[test]
    fn check_all_a_one_directional_sibling_call_is_not_treated_as_a_cycle() {
        // `caller` references a sibling (`helper`), but `helper` never
        // calls back -- not a cycle, so unlike the old whole-sibling-
        // reference guard, `caller` should still generalize and be
        // reusable at more than one type.
        let source = r#"
fn helper(x) {
    x
}
fn caller(y) {
    helper(y)
}
fn use_it() {
    caller(1);
    caller("hi");
}
"#;
        let mut cx = check_all(source);
        assert!(cx.diagnostics().is_empty(), "{:#?}", cx.diagnostics());

        let helper = cx
            .resolve_path(&path(&["helper"]), Namespace::Value)
            .expect("helper should resolve");
        let caller = cx
            .resolve_path(&path(&["caller"]), Namespace::Value)
            .expect("caller should resolve");
        assert_eq!(cx.symbols[helper].generics.len(), 1);
        assert_eq!(cx.symbols[caller].generics.len(), 1);
    }

    #[test]
    fn check_all_self_recursive_function_generalizes_and_stays_reusable() {
        let source = r#"
fn identity_rec(x) {
    if true { x } else { identity_rec(x) }
}
fn use_it() {
    identity_rec(1);
    identity_rec("hi");
}
"#;
        let mut cx = check_all(source);
        assert!(cx.diagnostics().is_empty(), "{:#?}", cx.diagnostics());

        let identity_rec = cx
            .resolve_path(&path(&["identity_rec"]), Namespace::Value)
            .expect("identity_rec should resolve");
        assert_eq!(cx.symbols[identity_rec].generics.len(), 1);
    }

    #[test]
    fn check_all_a_three_way_cycle_generalizes_together() {
        let source = r#"
fn a(x) {
    if true { x } else { b(x) }
}
fn b(y) {
    if true { y } else { c(y) }
}
fn c(z) {
    if true { z } else { a(z) }
}
fn use_it() {
    a(1);
    b("hi");
}
"#;
        let mut cx = check_all(source);
        assert!(cx.diagnostics().is_empty(), "{:#?}", cx.diagnostics());

        for name in ["a", "b", "c"] {
            let symbol = cx
                .resolve_path(&path(&[name]), Namespace::Value)
                .unwrap_or_else(|| panic!("{name} should resolve"));
            assert_eq!(cx.symbols[symbol].generics.len(), 1, "{name}");
        }
    }

    #[test]
    fn check_all_a_fully_annotated_cycle_has_nothing_left_to_generalize() {
        let source = r#"
fn ping2(x: int) -> int {
    pong2(x)
}
fn pong2(y: int) -> int {
    ping2(y)
}
"#;
        let mut cx = check_all(source);
        assert!(cx.diagnostics().is_empty(), "{:#?}", cx.diagnostics());

        let ping2 = cx
            .resolve_path(&path(&["ping2"]), Namespace::Value)
            .expect("ping2 should resolve");
        let pong2 = cx
            .resolve_path(&path(&["pong2"]), Namespace::Value)
            .expect("pong2 should resolve");
        assert_eq!(cx.symbols[ping2].generics.len(), 0);
        assert_eq!(cx.symbols[pong2].generics.len(), 0);
    }

    #[test]
    fn check_all_a_newly_generalized_param_never_reuses_an_explicit_generics_name() {
        // `T` is already claimed by the explicit `<T>`. `f`'s return
        // type and `g`'s `_` return type both end up as one more,
        // separate free variable (`g`'s `Fn(int) -> _` return flows
        // straight into `f`'s parameter), which needs its own generic
        // parameter -- and that parameter must not render as `T` too,
        // even though the two are perfectly sound as distinct
        // `GenericId`s regardless of what they're named.
        let source = r#"
fn compose<T>(f, g: Fn(int) -> _, x) -> Fn(T) -> String {
    f(g(x))
}
"#;
        let mut cx = check_all(source);
        assert!(cx.diagnostics().is_empty(), "{:#?}", cx.diagnostics());

        let compose = cx
            .resolve_path(&path(&["compose"]), Namespace::Value)
            .expect("compose should resolve");
        let generics = cx.symbols[compose].generics.clone();
        assert_eq!(generics.len(), 2, "{generics:?}");

        let explicit = generics[0];
        let inferred = generics[1];
        assert_ne!(explicit, inferred, "should be two distinct GenericIds");

        let explicit_name = cx.generic_names.get(&explicit).cloned();
        let inferred_name = cx.generic_names.get(&inferred).cloned();
        assert_eq!(explicit_name.as_deref(), Some("T"));
        assert_ne!(
            explicit_name, inferred_name,
            "the newly-generalized parameter must not render under the \
             same name as the explicit `<T>`"
        );
    }

    #[test]
    fn check_all_a_real_type_error_inside_a_cyclic_group_is_still_reported() {
        let source = r#"
fn ping3(x: int) {
    pong3(x)
}
fn pong3(y: int) {
    ping3("wrong")
}
"#;
        let cx = check_all(source);
        let diagnostics = cx.diagnostics();
        assert_eq!(diagnostics.len(), 1, "{diagnostics:#?}");
        assert_eq!(diagnostics[0].message, "expected `int`, found `String`");
    }

    #[test]
    fn check_all_nested_mutually_recursive_fns_generalize_together() {
        // `ping`/`pong` are called from a third nested sibling
        // (`use_both`) rather than directly from `outer`'s own
        // statements. That's not incidental: `check_block_expecting`
        // skips `StmtKind::Item` entirely (nested items are checked
        // separately, in `Checker::check_items`), so a call written
        // directly in `outer`'s own body runs *before* `outer`'s nested
        // sibling group is checked at all -- a pre-existing hoisting-
        // order quirk unrelated to strongly-connected generalization,
        // which this test isn't about. Routing the calls through a
        // sibling that's part of the same `check_items` pass sidesteps
        // it, the same way top-level calls already need a `use_both`-
        // style caller rather than a bare module-level statement (roo
        // has no module-level `let`/statements at all -- see
        // `book/src/bindings/variables.md`).
        let source = r#"
fn outer() {
    fn ping(x) {
        if true { x } else { pong(x) }
    }
    fn pong(y) {
        if true { y } else { ping(y) }
    }
    fn use_both() {
        ping(1);
        pong("hi");
    }
}
"#;
        let mut cx = check_all(source);
        assert!(cx.diagnostics().is_empty(), "{:#?}", cx.diagnostics());

        let outer = cx
            .resolve_path(&path(&["outer"]), Namespace::Value)
            .expect("outer should resolve");
        let outer_scope = fn_body_scope(&cx, outer);

        let ping = declared_symbol(&cx, outer_scope, Namespace::Value, "ping")
            .expect("ping should be declared inside outer's body");
        let pong = declared_symbol(&cx, outer_scope, Namespace::Value, "pong")
            .expect("pong should be declared inside outer's body");
        assert_eq!(cx.symbols[ping].generics.len(), 1);
        assert_eq!(cx.symbols[pong].generics.len(), 1);
    }

    #[test]
    fn check_all_a_nested_fn_never_generalizes_a_variable_free_in_an_enclosing_signature() {
        // A curried `compose`: `inner` and `innermost` are nested
        // inside `compose` and returned as values, so their types end
        // up structurally embedded in `compose`'s own return type --
        // meaning the type variables for `f`'s result, `g`'s result,
        // and `x` are *also* free in `compose`'s own, still-open
        // signature at the point `inner`/`innermost` would otherwise
        // be generalized. If either of them claimed one of those
        // variables for itself, `compose` would be left with no way
        // to generalize it at all, pinning `compose` to whatever
        // concrete types happened to flow through first -- exactly
        // the bug this test exists to catch a regression of.
        let source = r#"
fn compose(f) {
    fn inner(g) {
        fn innermost(x) {
            f(g(x))
        }
        innermost
    }
    inner
}
"#;
        let mut cx = check_all(source);
        assert!(cx.diagnostics().is_empty(), "{:#?}", cx.diagnostics());

        let compose = cx
            .resolve_path(&path(&["compose"]), Namespace::Value)
            .expect("compose should resolve");
        let compose_scope = fn_body_scope(&cx, compose);

        let inner = declared_symbol(&cx, compose_scope, Namespace::Value, "inner")
            .expect("inner should be declared inside compose's body");
        let inner_scope = fn_body_scope(&cx, inner);
        let innermost = declared_symbol(&cx, inner_scope, Namespace::Value, "innermost")
            .expect("innermost should be declared inside inner's body");

        // Every free variable is deferred all the way up to `compose`,
        // the only one of the three that's ever independently callable
        // from outside this whole nest -- `inner`/`innermost` have
        // nothing left of their own to generalize.
        assert_eq!(cx.symbols[compose].generics.len(), 3, "{:#?}", cx.symbols[compose].generics);
        assert_eq!(cx.symbols[inner].generics.len(), 0);
        assert_eq!(cx.symbols[innermost].generics.len(), 0);
    }

    #[test]
    fn check_all_a_nested_fns_deferred_generalization_is_actually_usable_at_two_types() {
        // The behavioral proof behind the structural assertions in
        // the test above: if `compose` weren't *fully* generalized,
        // calling it twice at genuinely different, incompatible `f`
        // shapes would conflict, the same way an under-generalized
        // sibling function would.
        let source = r#"
fn compose(f) {
    fn inner(g) {
        fn innermost(x) {
            f(g(x))
        }
        innermost
    }
    inner
}
fn int_to_string(n: int) -> String {
    "hi"
}
fn bool_to_int(b: bool) -> int {
    1
}
fn use_it() {
    compose(int_to_string);
    compose(bool_to_int);
}
"#;
        let cx = check_all(source);
        assert!(cx.diagnostics().is_empty(), "{:#?}", cx.diagnostics());
    }
}
