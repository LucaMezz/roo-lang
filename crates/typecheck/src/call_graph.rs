use std::collections::{HashMap, HashSet};

use ast::visit::{Visitor, Walkable};
use ast::{Expr, ExprKind, Item, ItemKind, Pat, PatKind, Stmt, StmtKind};

use crate::SymbolId;

/// Finds the strongly connected components within the given function
/// call graph, using Tarjan's algorithm to do it in O(V+E) time.
///
/// Used to perform call-graph SCC analysis on recursive or mutually
/// recursive functions so that free inference variables can be
/// generalised by introducing type parameters.
pub(crate) fn strongly_connected_components(
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

    /// Explores a single node.
    fn visit(node: SymbolId, edges: &HashMap<SymbolId, Vec<SymbolId>>, state: &mut State) {
        state.index.insert(node, state.next_index);
        state.lowlink.insert(node, state.next_index);
        state.next_index += 1;
        state.stack.push(node);
        state.on_stack.insert(node);

        // Explores each edge out of the current node.
        for &successor in edges.get(&node).map(Vec::as_slice).unwrap_or_default() {
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
    for &node in nodes {
        if !state.index.contains_key(&node) {
            visit(node, edges, &mut state);
        }
    }
    state.sccs
}

/// Responsible for finding paths to functions, to help construct
/// the call graph so that SCC analysis can be performed on it.
pub(crate) struct CallGraphCollector<'a> {
    /// The name of all functions in the same scope as the one
    /// being explored.
    pub(crate) sibling_names: &'a HashMap<&'a str, SymbolId>,
    /// A list of functions that have been shadowed and so any
    /// occurrences of that name should not have an edge made
    /// because the name no longer represents a function.
    pub(crate) shadowed: HashSet<String>,
    pub(crate) edges: Vec<SymbolId>,
}

impl Visitor for CallGraphCollector<'_> {
    fn visit_expr(&mut self, expr: &Expr) {
        // For any reference to one of the sibling functions,
        // create a new edge which will be included in the
        // constructed call graph. Ignore shadowed functions.
        if let ExprKind::Path(None, path) = &expr.kind
            && let [segment] = path.segments.as_slice()
            && !self.shadowed.contains(segment.ident.name.as_str())
            && let Some(&symbol) = self.sibling_names.get(segment.ident.name.as_str())
        {
            self.edges.push(symbol);
        }
        expr.walk(self);
    }

    fn visit_stmt(&mut self, stmt: &Stmt) {
        stmt.walk(self);
        // Builds the list of shadowed names.
        match &stmt.kind {
            StmtKind::Let(local) => collect_pat_names(&local.pat, &mut self.shadowed),
            StmtKind::Item(item) => {
                if let ItemKind::Fn(f) = &item.kind {
                    self.shadowed.insert(f.ident.name.clone());
                }
            }
            _ => {}
        }
    }

    fn visit_item(&mut self, _item: &Item) {}
}

/// Collects the names of all local variables that are introduced
/// by the pattern, to help build a set of shadowed names.
pub(crate) fn collect_pat_names(pat: &Pat, names: &mut HashSet<String>) {
    match &pat.kind {
        PatKind::Ident(ident, sub) => {
            names.insert(ident.name.clone());
            if let Some(sub) = sub {
                collect_pat_names(sub, names);
            }
        }
        PatKind::Tuple(pats) => {
            for pat in pats {
                collect_pat_names(pat, names);
            }
        }
        _ => {}
    }
}
