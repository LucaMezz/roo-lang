use std::collections::{HashMap, HashSet};

use ast::visit::{Visitor, Walkable};
use ast::{Expr, ExprKind, Item, ItemKind, Pat, PatKind, Stmt, StmtKind};

use crate::SymbolId;

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

    fn visit(node: SymbolId, edges: &HashMap<SymbolId, Vec<SymbolId>>, state: &mut State) {
        state.index.insert(node, state.next_index);
        state.lowlink.insert(node, state.next_index);
        state.next_index += 1;
        state.stack.push(node);
        state.on_stack.insert(node);

        for &successor in edges.get(&node).map(Vec::as_slice).unwrap_or_default() {
            if !state.index.contains_key(&successor) {
                visit(successor, edges, state);
                let pulled = state.lowlink[&successor];
                let current = state.lowlink[&node];
                state.lowlink.insert(node, current.min(pulled));
            } else if state.on_stack.contains(&successor) {
                let successor_index = state.index[&successor];
                let current = state.lowlink[&node];
                state.lowlink.insert(node, current.min(successor_index));
            }
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

pub(crate) struct CallGraphCollector<'a> {
    pub(crate) sibling_names: &'a HashMap<&'a str, SymbolId>,
    pub(crate) shadowed: HashSet<String>,
    pub(crate) edges: Vec<SymbolId>,
}

impl Visitor for CallGraphCollector<'_> {
    fn visit_expr(&mut self, expr: &Expr) {
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
