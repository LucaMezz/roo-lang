use std::collections::{HashMap, HashSet};

use ast::{
    Expr, ExprKind, Stmt, StmtKind,
    visit::{Visitor, Walkable},
};

use crate::{Namespace, SymbolId, SymbolKind, TypeCheckContext};

pub trait AdjacencyListGraph {
    fn nodes(&self) -> &HashSet<SymbolId>;
    fn edges(&self) -> &HashMap<SymbolId, HashSet<SymbolId>>;
}

#[derive(Debug)]
pub struct CallGraph {
    /// A list of all of the functions which appear in this call graph.
    nodes: HashSet<SymbolId>,
    /// A list of calls from the body of one function to another.
    edges: HashMap<SymbolId, HashSet<SymbolId>>,
    /// The current function.
    curr: Option<SymbolId>,
}

impl CallGraph {
    pub fn new() -> Self {
        Self {
            nodes: HashSet::new(),
            edges: HashMap::new(),
            curr: None,
        }
    }

    pub fn declare(&mut self, symbol: SymbolId) {
        self.nodes.insert(symbol);
    }

    pub fn call(&mut self, from: SymbolId, to: SymbolId) {
        self.nodes.insert(from);
        self.nodes.insert(to);
        let edge_set = self.edges.get_mut(&from);
        match edge_set {
            Some(edges) => {
                edges.insert(to);
            }
            None => {
                self.edges.insert(from, HashSet::from([to]));
            }
        }
    }
}

impl AdjacencyListGraph for CallGraph {
    fn nodes(&self) -> &HashSet<SymbolId> {
        &self.nodes
    }

    fn edges(&self) -> &HashMap<SymbolId, HashSet<SymbolId>> {
        &self.edges
    }
}

/// Finds the strongly connected components within the given function
/// call graph, using Tarjan's algorithm to do it in O(V+E) time.
///
/// Used to perform call-graph SCC analysis on recursive or mutually
/// recursive functions so that free inference variables can be
/// generalised by introducing type parameters.
pub(crate) fn strongly_connected_components<G>(graph: &G) -> Vec<Vec<SymbolId>>
where
    G: AdjacencyListGraph,
{
    struct State {
        index: HashMap<SymbolId, u32>,
        lowlink: HashMap<SymbolId, u32>,
        on_stack: HashSet<SymbolId>,
        stack: Vec<SymbolId>,
        next_index: u32,
        sccs: Vec<Vec<SymbolId>>,
    }

    /// Explores a single node.
    fn visit(node: SymbolId, edges: &HashMap<SymbolId, HashSet<SymbolId>>, state: &mut State) {
        state.index.insert(node, state.next_index);
        state.lowlink.insert(node, state.next_index);
        state.next_index += 1;
        state.stack.push(node);
        state.on_stack.insert(node);

        // Explores each edge out of the current node.
        for &successor in edges.get(&node).into_iter().flatten() {
            if !state.index.contains_key(&successor) {
                // If the node this edge points to has not been visited previously, then
                // visit it, since the connected component it belongs to has not yet been
                // determined. Its update the current node's low-link value to the min
                // of the current low-link value and that of the visitied node's computed
                // low-link value.
                visit(successor, edges, state);
                let pulled = state.lowlink[&successor];
                let current = state.lowlink[&node];
                state.lowlink.insert(node, current.min(pulled));
            } else if state.on_stack.contains(&successor) {
                // At this point, if the node this edge points to has been visited
                // previously and is still on the stack, that means its still being
                // explored, and in fact its one of this node's own callers. That
                // means that the `lowlink` value of this successor node is still
                // in the process of being computed and is incomplete, so we can't
                // min it with the current lowlink value of this node. Instead,
                // all we know for certain is the index of the node, and obviously
                // the lowlink value of the successor is bounded above by its index.
                let successor_index = state.index[&successor];
                let current = state.lowlink[&node];
                state.lowlink.insert(node, current.min(successor_index));
            }
        }

        // After exploring all outgoing edges, if the lowlink of the node is equal to its
        // index, then it means the smallest id node that can be reached from the current
        // node `u` is `u` itself. If any node `v` in the subtree of `u` does not have
        // a way back to `u`, then `v` would have already been turned into an SCC on its
        // own, and so must not be part of the same SCC as `u`.
        // That means that anything remaining on the stack above `node` must have a way
        // back to `u`. Since `v` is reachable from `u`, and `u` is reachable from `v`,
        // then `u` and `v` must be part of the same SCC. So pop them all from the stack,
        // including `v` itself, and create a new SCC containing them.
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

    // Visit an unexplored node in the call graph. Explore it until
    // an entire strongly connected component is found. Repeat until
    // all nodes have been explored.
    for &node in graph.nodes() {
        if !state.index.contains_key(&node) {
            visit(node, graph.edges(), &mut state);
        }
    }
    state.sccs
}

pub struct CallGraphCollector<'a> {
    graph: &'a mut CallGraph,
    cx: &'a mut TypeCheckContext,
    from: SymbolId,
}

impl<'a> CallGraphCollector<'a> {
    pub fn new(from: SymbolId, graph: &'a mut CallGraph, cx: &'a mut TypeCheckContext) -> Self {
        Self { from, graph, cx }
    }
}

impl<'a> Visitor for CallGraphCollector<'a> {
    fn visit_expr(&mut self, expr: &Expr) {
        match &expr.kind {
            ExprKind::Path(_, path) => {
                if let Some(to) = self.cx.resolve_path(path, Namespace::Value) {
                    if let Some(symbol) = self.cx.symbols.get(to) {
                        if let SymbolKind::Fn(_) = symbol.kind {
                            self.graph.call(self.from, to);
                        }
                    }
                }
            }
            _ => expr.walk(self),
        };
    }

    fn visit_stmt(&mut self, stmt: &Stmt) {
        match &stmt.kind {
            StmtKind::Item(_) => {}
            _ => stmt.walk(self),
        }
    }
}
