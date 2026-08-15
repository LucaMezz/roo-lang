//! `typecheck` is fig-lang's type checker.
//!
//! Combines two things: the concrete type representation (`TyCon`, the
//! `C` that plugs into [`unify::Term`]) and the actual checking logic
//! that uses it -- lowering `ast::Ty`/`ast::ExprKind` into terms,
//! generating `t1 ≟ t2` constraints, and driving `unify::unify` to solve
//! them. Deliberately one crate, not split into a separate "just the
//! enum" crate and a logic crate -- `unify` is the reusable, fig-agnostic
//! piece; `TyCon` is fig-specific by definition, so it lives with the
//! rest of fig's type checking rather than off on its own. (rustc did
//! the same thing for most of its history: `rustc_typeck` held both,
//! only splitting into `rustc_hir_analysis`/`rustc_hir_typeck` once it
//! grew large enough for that to be worth it.)
//!
//! Not yet implemented -- nothing here yet.

use std::collections::HashMap;

use ast::visit::{Visitor, Walkable};
use ast::{
    Block, Expr, ExprKind, Fn, FnRetTy, FnTy, Item, ItemKind, LitKind, Local, LocalKind, ModKind,
    Pat, PatKind, Path, Stmt, StmtKind, Ty, TyKind, VariantData,
};
use slotmap::SlotMap;
use unify::{Term, TermId, UnificationContext, term};

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

    Err,
}

slotmap::new_key_type! {
    /// Generational id for the scope arena.
    pub struct ScopeId;

    /// Generational id for the symbol arena.
    pub struct SymbolId;
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
enum Namespace {
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
}

/// The different kinds of symbols.
enum SymbolKind {
    Struct,
    Enum,
    Variant,
    Trait,
    TyAlias,
    Mod(ScopeId),
    Fn(ScopeId),
    Local,
}

impl SymbolKind {
    /// Determines whether this kind of symbol belongs in the type
    /// or value namespace of a scope.
    fn namespace(&self) -> Namespace {
        match self {
            SymbolKind::Struct
            | SymbolKind::Enum
            | SymbolKind::Trait
            | SymbolKind::TyAlias
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
struct TypeCheckContext {
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

    /// The current scope being checked.
    current_scope: ScopeId,
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
            symbols: SlotMap::with_key(),
            current_scope: root,
        }
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

    /// Second pass, run after `resolve`: lowers every `Fn`/`TyAlias`
    /// item's *declared* signature and unifies it into the symbol
    /// `resolve` already created a placeholder for -- without this, a
    /// symbol's type only ever gets pinned down reactively, by
    /// whichever caller happens to check first, which is wrong (the
    /// declaration should be authoritative, not a guess from usage).
    /// Struct/enum fields and trait associated items aren't handled
    /// here -- they need their own per-symbol storage this doesn't
    /// have yet, not just one term to unify in.
    pub fn lower_signatures(&mut self, items: &[Box<Item>]) {
        let mut lowerer = SignatureLowerer { cx: self };
        for item in items {
            lowerer.visit_item(item);
        }
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
        let symbol = self.symbols.insert(Symbol { name, kind, ty });
        self.insert_in_scope(name, symbol, namespace);
        symbol
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
                    FnRetTy::Default(_) => term!(self.uni_cx, TyCon::Tuple),
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
                _ => match self.resolve_path(path, Namespace::Type) {
                    Some(symbol) => match &self.symbols[symbol].kind {
                        SymbolKind::Struct => term!(self.uni_cx, TyCon::Struct(symbol)),
                        SymbolKind::Enum => term!(self.uni_cx, TyCon::Enum(symbol)),
                        _ => self.symbols[symbol].ty,
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

    /// Determines the type of an Expression node from the AST, possibly given
    /// additional information about what the type is expected to be.
    fn check_expr(&mut self, expr: &Expr, expected: Option<TermId>) -> TermId {
        let actual = self.check_expr_kind(&expr.kind, expected);
        if let Some(expected) = expected {
            let _ = self.uni_cx.unify(actual, expected);
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
                let _ = self.uni_cx.unify(body_ty, els_ty);

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
                    Some(symbol) => self.symbols[symbol].ty,
                    None => term!(self.uni_cx, TyCon::Err),
                }
            }
            ExprKind::Call(callee, args) => {
                // Check the type of the expression which is being called
                let callee_ty = self.check_expr(callee, None);

                // Check the type of all of the arguments in the call
                let arg_tys = args.iter().map(|arg| self.check_expr(arg, None)).collect();

                // Build a tuple with the input types
                let inputs_term = term!(self.uni_cx, TyCon::Tuple => arg_tys);

                // Just based on the call we know the type of all the arguments to
                // the function, except the return type. So we introduce a new
                // inference variable to represent the return type.
                let ret_var = self.uni_cx.fresh_var();
                let ret_term = term!(self.uni_cx, var ret_var);

                // Enforce the constraint that the type of the callee must match
                // this new function type we have created. This fills in the
                // return type of the callee function if it is known, etc.
                let fn_term = term!(self.uni_cx, TyCon::Fn => [inputs_term, ret_term]);
                let _ = self.uni_cx.unify(callee_ty, fn_term);

                ret_term
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
        let mut ty = term!(self.uni_cx, TyCon::Tuple);
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
                    ty = self.check_expr(expr, expected);
                }

                // Type check the statements, however, since they are not the last
                // expression in the block, they are not related to the return type
                // of the block.
                StmtKind::Expr(expr) | StmtKind::Semi(expr) => {
                    self.check_expr(expr, None);
                }

                // Nothing to do. Items are already traversed as part of the resolve
                // stage.
                StmtKind::Item(_) | StmtKind::Empty => {}
            }
        }
        ty
    }

    /// Type checks a `Local` statement.
    fn check_local(&mut self, local: &Local) {
        // The type annotation for this local, which it may not have.
        let ascribed = local.ty.as_ref().map(|ty| self.lower_ty(ty));

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
                let actual = self.check_expr(init, ascribed);
                Some(ascribed.unwrap_or(actual))
            }

            // Local declaration with an initialiser and an `else` clause.
            // Example: `let Some(x) = y else { return };`
            //
            // Same as the case above, except that we need to also type-check
            // the `else` clause, which should be required to diverge (and
            // so have return type Never).
            LocalKind::InitElse(init, else_block) => {
                let actual = self.check_expr(init, ascribed);
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
        let resolved_a = self.uni_cx.resolve(a);
        match self.uni_cx.term(resolved_a) {
            Some(Term::App {
                constructor: TyCon::Never,
                ..
            }) => b,
            _ => a,
        }
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
            PatKind::Ident(_is_mutable, ident, sub) => {
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
                self.cx.declare(&f.ident.name, SymbolKind::Fn(scope));
                self.with_scope(scope, |this| item.walk(this));
                return;
            }
            ItemKind::TyAlias(alias) => {
                self.cx.declare(&alias.ident.name, SymbolKind::TyAlias);
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
            FnRetTy::Default(_) => term!(self.cx.uni_cx, TyCon::Tuple),
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

                if let Some(symbol) = symbol {
                    let fn_term = self.lower_fn_sig(f);
                    let symbol_ty = self.cx.symbols[symbol].ty;
                    let _ = self.cx.uni_cx.unify(symbol_ty, fn_term);
                }

                // Recurse into the fn's own body scope, so nested (hoisted)
                // items get their signatures lowered too.
                let scope = symbol.and_then(|symbol| match &self.cx.symbols[symbol].kind {
                    SymbolKind::Fn(scope) => Some(*scope),
                    _ => None,
                });
                if let Some(scope) = scope {
                    self.with_scope(scope, |this| item.walk(this));
                }
            }
            ItemKind::TyAlias(alias) => {
                let name = self.cx.names.id(&alias.ident.name);
                let symbol = self
                    .cx
                    .lookup_in_scope(self.cx.current_scope, name, Namespace::Type);
                if let (Some(symbol), Some(ty)) = (symbol, alias.ty.as_ref()) {
                    let aliased = self.cx.lower_ty(ty);
                    let symbol_ty = self.cx.symbols[symbol].ty;
                    let _ = self.cx.uni_cx.unify(symbol_ty, aliased);
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
}

impl Visitor for Checker<'_> {
    fn visit_item(&mut self, item: &Item) {
        // In the checking phase, we are only concerned about visiting
        // functions. If its not a function, skip the node and any
        // child nodes.
        let ItemKind::Fn(f) = &item.kind else {
            return;
        };

        // Get the symbol associated with the function being visited.
        let name = self.cx.names.id(&f.ident.name);
        let Some(symbol) = self
            .cx
            .lookup_in_scope(self.cx.current_scope, name, Namespace::Value)
        else {
            return;
        };

        // We expect that the symbol kind is `Fn`, and we get the scope
        // of the function body.
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
            this.cx.check_block(body, Some(output_term));

            // Recurse into any items hoisted into this fn's own body, so
            // nested fns get their bodies checked too.
            for stmt in &body.stmts {
                if let StmtKind::Item(nested) = &stmt.kind {
                    this.visit_item(nested);
                }
            }
        });
    }

    fn visit_stmt(&mut self, stmt: &Stmt) {
        match &stmt.kind {
            StmtKind::Let(local) => self.cx.check_local(local),
            _ => stmt.walk(self),
        }
    }

    fn visit_expr(&mut self, _expr: &Expr) {}
}

#[cfg(test)]
mod tests {
    use super::*;
    use chumsky::Parser;

    fn resolve(source: &str) -> TypeCheckContext {
        let tokens = lexer::tokenize_all(source).expect("should lex");
        let items = parser::module()
            .parse(&tokens)
            .into_result()
            .expect("should parse");

        let mut cx = TypeCheckContext::new();
        cx.resolve(&items);
        cx
    }

    fn resolve_and_lower(source: &str) -> TypeCheckContext {
        let tokens = lexer::tokenize_all(source).expect("should lex");
        let items = parser::module()
            .parse(&tokens)
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
            .parse(&tokens)
            .into_result()
            .expect("should parse")
    }

    fn ty(source: &str) -> Ty {
        let tokens = lexer::tokenize_all(source).expect("should lex");
        parser::ty()
            .parse(&tokens)
            .into_result()
            .expect("should parse")
    }

    fn pat(source: &str) -> Pat {
        let tokens = lexer::tokenize_all(source).expect("should lex");
        parser::pat(parser::expr())
            .parse(&tokens)
            .into_result()
            .expect("should parse")
    }

    fn block(source: &str) -> Block {
        let tokens = lexer::tokenize_all(source).expect("should lex");
        parser::block(parser::expr())
            .parse(&tokens)
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
    fn lower_ty_fn_with_no_return_type_defaults_to_unit() {
        let mut cx = TypeCheckContext::new();
        let t = cx.lower_ty(&ty("Fn(!)"));
        let (_, args) = resolved_args(&mut cx, t).expect("should be an App term");
        let (output_con, output_args) =
            resolved_args(&mut cx, args[1]).expect("should be an App term");
        assert_eq!(output_con, TyCon::Tuple);
        assert!(output_args.is_empty());
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
    fn lower_signatures_fn_with_no_return_type_is_unit() {
        let mut cx = resolve_and_lower("fn foo() {}");
        let symbol = cx
            .resolve_path(&path(&["foo"]), Namespace::Value)
            .expect("foo should resolve");
        let symbol_ty = cx.symbols[symbol].ty;

        let (_, args) = resolved_args(&mut cx, symbol_ty).expect("should be a Fn term");
        let (output_con, output_args) =
            resolved_args(&mut cx, args[1]).expect("should be a Tuple term");
        assert_eq!(output_con, TyCon::Tuple);
        assert!(output_args.is_empty());
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
            .parse(&tokens)
            .into_result()
            .expect("should parse");

        let mut cx = TypeCheckContext::new();
        cx.resolve(&items);
        cx.lower_signatures(&items);
        for item in &items {
            Checker { cx: &mut cx }.visit_item(item);
        }
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
}
