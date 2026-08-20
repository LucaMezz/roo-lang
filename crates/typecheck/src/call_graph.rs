//! Contains everything used to build and analyse the call graph of
//! functions.
//!
//! Currently, the call graph is built specifically so that we can
//! find all strongly connected components (SCCs) of the call graph.
//! This information is then used to work out which functions depend
//! on others, and type check them in the correct order, so that we
//! can generalise and infer generic type parameters as soon as a
//! complete SCC has been found
//!
//! An online version of Tarjan's SCC algorithm is used to discover
//! SCCs in the call graph as it is constructed.
use std::collections::{HashMap, HashSet};

use indexmap::{IndexMap, IndexSet};

use crate::SymbolId;

/// A directed graph where each node is a function, and each directed
/// edge from a node `f` to a node `g` represents that the function
/// `f` uses the function `g`.
///
/// Used to help with inferring generic type parameters for functions
/// by performing call-graph strongly connected component analysis
/// on the call graph of functions.
///
/// Will likely also be used for other static analysis, or later for
/// optimisation of generated IR / bytecode.
#[derive(Debug)]
pub struct CallGraph {
    /// A list of all of the functions which appear in this call graph.
    nodes: IndexSet<SymbolId>,
    /// A list of calls from the body of one function to another.
    edges: IndexMap<SymbolId, IndexSet<SymbolId>>,
}

impl CallGraph {
    /// Creates a new empty [`CallGraph`].
    pub fn new() -> Self {
        Self {
            nodes: IndexSet::new(),
            edges: IndexMap::new(),
        }
    }

    /// Records an edge from some node `f` to another node `g`.
    ///
    /// Indicates a call from the function `f` to the function `g`.
    pub fn call(&mut self, from: SymbolId, to: SymbolId) {
        self.nodes.insert(from);
        self.nodes.insert(to);
        let edge_set = self.edges.get_mut(&from);
        match edge_set {
            Some(edges) => {
                edges.insert(to);
            }
            None => {
                self.edges.insert(from, IndexSet::from([to]));
            }
        }
    }

    #[allow(unused)]
    pub fn nodes(&self) -> &IndexSet<SymbolId> {
        &self.nodes
    }

    #[allow(unused)]
    pub fn edges(&self) -> &IndexMap<SymbolId, IndexSet<SymbolId>> {
        &self.edges
    }
}

/// A struct that performs an `online` version of Tarjan's algorithm
/// to discover all strongly connected components within the call
/// graph.
///
/// Contains the state required by the algorithm, as well as methods
/// that can be called while traversing the tree when function
/// calls are discovered.
pub struct SCCCollector {
    index: HashMap<SymbolId, u32>,
    lowlink: HashMap<SymbolId, u32>,
    on_stack: HashSet<SymbolId>,
    stack: Vec<SymbolId>,
    next_index: u32,
    sccs: Vec<Vec<SymbolId>>,
}

impl SCCCollector {
    /// Creates a new empty [`SCCCollector`].
    pub fn new() -> Self {
        Self {
            index: HashMap::new(),
            lowlink: HashMap::new(),
            on_stack: HashSet::new(),
            stack: Vec::new(),
            next_index: 0,
            sccs: Vec::new(),
        }
    }

    /// Consumes the [`SCCCollector`], returning a list of all
    /// of the strongly connected components which have been
    /// discovered by it.
    pub fn sccs(self) -> Vec<Vec<SymbolId>> {
        self.sccs
    }

    /// Checks if a function has already been visited by the
    /// [`SCCCollector`].
    ///
    /// Used to prevent repeated checking of functions when they
    /// may have already been checked at an earlier point due
    /// to some other function calling it.
    pub fn is_visited(&self, symbol: SymbolId) -> bool {
        self.index.contains_key(&symbol)
    }

    /// Called when a new function begins being checked.
    pub fn enter(&mut self, symbol: SymbolId) {
        self.index.insert(symbol, self.next_index);
        self.lowlink.insert(symbol, self.next_index);
        self.next_index += 1;
        self.stack.push(symbol);
        self.on_stack.insert(symbol);
    }

    /// Called when a function has completed its checking.
    pub fn exit(&mut self, symbol: SymbolId) -> Option<Vec<SymbolId>> {
        if self.lowlink[&symbol] != self.index[&symbol] {
            return None;
        }
        let mut component = Vec::new();
        loop {
            let member = self
                .stack
                .pop()
                .expect("symbol's own frame is still on the stack until its root pops it");
            self.on_stack.remove(&member);
            component.push(member);
            if member == symbol {
                break;
            }
        }
        self.sccs.push(component.clone());
        Some(component)
    }

    /// Called when there is a call from a function `f` to a
    /// function `g` which has not yet been visited at all.
    ///
    /// [`Self::pull_lowlink`] assumes that the function `g` will
    /// first be visited immediately beforehand.
    ///
    /// Essentially, this says the smallest index node that the
    /// calling function `f` stays the same *unless* `g` can
    /// reach something with an even smaller index, and since
    /// `f` reaches `g`, it means so can `f`.
    pub fn pull_lowlink(&mut self, from: SymbolId, to: SymbolId) {
        let pulled = self.lowlink[&to];
        let mine = self.lowlink.get_mut(&from).unwrap();
        *mine = (*mine).min(pulled);
    }

    /// Called when there is a call from a function `f` to a
    /// function `g` which has already been visited previously
    /// and which is still on the stack.
    ///
    /// Essentially, this says that the smallest index node
    /// that the calling function `f` can reach stays the same
    /// *unless* `g` has a smaller index.
    ///
    /// This situation represents when the edge from `f` to `g`
    /// represents a 'back edge' in the DFS tree.
    pub fn note_back_edge(&mut self, from: SymbolId, to: SymbolId) {
        if self.on_stack.contains(&to) {
            let idx = self.index[&to];
            let mine = self.lowlink.get_mut(&from).unwrap();
            *mine = (*mine).min(idx);
        }
    }
}
