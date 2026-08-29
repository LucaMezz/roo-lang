//! Defines [`ScopeTree`], the tree of all [`Scope`]s within a
//! program, and the namespace-aware lookup and insertion of defs
//! into it.

use std::collections::HashMap;

use intern::Symbol;
use slotmap::SlotMap;

use crate::DefId;

slotmap::new_key_type! {
    /// A handle to a [`Scope`] stored in a [`ScopeTree`].
    pub struct ScopeId;
}

/// A kind of Namespace within each scope.
///
/// A scope has two separate Namespaces for defs. One only
/// contains defs which represent types within the scope,
/// while the other only contains defs which represent
/// values within the scope.
#[derive(Clone, Copy)]
pub(crate) enum Namespace {
    /// The Namespace of Types within a scope.
    Type,

    /// The Namespace of Values within a scope.
    Value,
}

/// A scope. Represents a context where defs can be defined.
///
/// Scopes are created for things such as function bodies and
/// blocks, but also for anything else that introduces its own
/// region of visibility for defs, such as the generic parameters
/// of an impl block or a type alias.
#[derive(Debug)]
struct Scope {
    /// A handle to the enclosing scope.
    parent: Option<ScopeId>,

    /// The [`Namespace::Type`] Namespace. Maps the symbol of each
    /// type defined in this scope to its def's handle.
    types: HashMap<Symbol, DefId>,

    /// The [`Namespace::Value`] Namespace. Maps the symbol of
    /// each value defined in this scope to its def's
    /// handle.
    values: HashMap<Symbol, DefId>,
}

impl Scope {
    fn new(parent: Option<ScopeId>) -> Self {
        Self {
            parent,
            types: HashMap::new(),
            values: HashMap::new(),
        }
    }

    fn namespace(&self, namespace: Namespace) -> &HashMap<Symbol, DefId> {
        match namespace {
            Namespace::Type => &self.types,
            Namespace::Value => &self.values,
        }
    }

    fn namespace_mut(&mut self, namespace: Namespace) -> &mut HashMap<Symbol, DefId> {
        match namespace {
            Namespace::Type => &mut self.types,
            Namespace::Value => &mut self.values,
        }
    }
}

/// The tree of every [`Scope`] within a program, each linked to its
/// enclosing scope by a parent pointer.
///
/// Provides namespace-aware lookup and insertion of defs within a
/// single scope, as well as lookup that walks up the chain of
/// enclosing scopes.
#[derive(Debug)]
pub(crate) struct ScopeTree {
    scopes: SlotMap<ScopeId, Scope>,
}

impl ScopeTree {
    /// Creates a new [`ScopeTree`] containing a single root scope,
    /// and returns a handle to that root scope.
    pub(crate) fn new() -> (Self, ScopeId) {
        let mut scopes = SlotMap::with_key();
        let root = scopes.insert(Scope::new(None));
        (Self { scopes }, root)
    }

    /// Creates a new child [`Scope`] of the given parent scope, and
    /// returns a handle to it.
    pub(crate) fn new_child(&mut self, parent: ScopeId) -> ScopeId {
        self.scopes.insert(Scope::new(Some(parent)))
    }

    /// Directly checks if the given scope contains a def with a
    /// given symbol, which belongs to a certain Namespace.
    pub(crate) fn lookup(
        &self,
        scope: ScopeId,
        symbol: Symbol,
        namespace: Namespace,
    ) -> Option<DefId> {
        self.scopes[scope]
            .namespace(namespace)
            .get(&symbol)
            .copied()
    }

    /// Recursively searches the given scope and its enclosing
    /// scopes for a def with a given symbol and which belongs to
    /// the specified Namespace.
    pub(crate) fn lookup_up_chain(
        &self,
        scope: ScopeId,
        symbol: Symbol,
        namespace: Namespace,
    ) -> Option<DefId> {
        std::iter::successors(Some(scope), |&scope| self.scopes[scope].parent)
            .find_map(|scope| self.lookup(scope, symbol, namespace))
    }

    /// Inserts a def into the given scope under a certain
    /// Namespace.
    pub(crate) fn insert(
        &mut self,
        scope: ScopeId,
        symbol: Symbol,
        def: DefId,
        namespace: Namespace,
    ) {
        self.scopes[scope]
            .namespace_mut(namespace)
            .insert(symbol, def);
    }

    /// Returns every symbol -> def entry within a given scope's
    /// Namespace. Used to bring every item of a Namespace into
    /// another scope, e.g. for a glob import.
    pub(crate) fn entries(&self, scope: ScopeId, namespace: Namespace) -> Vec<(Symbol, DefId)> {
        self.scopes[scope]
            .namespace(namespace)
            .iter()
            .map(|(&symbol, &def)| (symbol, def))
            .collect()
    }

    /// Returns a handle to a direct child scope of the given scope,
    /// if one exists. Used in tests, where the id of a scope
    /// created for a nested construct (e.g. a mod or fn body) is
    /// not otherwise reachable except by knowing which scope
    /// encloses it.
    #[cfg(test)]
    pub(crate) fn child_of(&self, parent: ScopeId) -> Option<ScopeId> {
        self.scopes
            .iter()
            .find_map(|(id, scope)| (scope.parent == Some(parent)).then_some(id))
    }
}
