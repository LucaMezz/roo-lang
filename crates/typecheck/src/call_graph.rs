use std::collections::{HashMap, HashSet};

use ast::{
    Expr, ExprKind, Stmt, StmtKind,
    visit::{Visitor, Walkable},
};
use indexmap::{IndexMap, IndexSet};

use crate::{Namespace, SymbolId, SymbolKind, TypeCheckContext};

pub trait AdjacencyListGraph {
    fn nodes(&self) -> &IndexSet<SymbolId>;
    fn edges(&self) -> &IndexMap<SymbolId, IndexSet<SymbolId>>;
}

#[derive(Debug)]
pub struct CallGraph {
    /// A list of all of the functions which appear in this call graph.
    nodes: IndexSet<SymbolId>,
    /// A list of calls from the body of one function to another.
    edges: IndexMap<SymbolId, IndexSet<SymbolId>>,
    /// The current function.
    curr: Option<SymbolId>,
}

impl CallGraph {
    pub fn new() -> Self {
        Self {
            nodes: IndexSet::new(),
            edges: IndexMap::new(),
            curr: None,
        }
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
                self.edges.insert(from, IndexSet::from([to]));
            }
        }
    }
}

impl AdjacencyListGraph for CallGraph {
    fn nodes(&self) -> &IndexSet<SymbolId> {
        &self.nodes
    }

    fn edges(&self) -> &IndexMap<SymbolId, IndexSet<SymbolId>> {
        &self.edges
    }
}

/// Bundles the call graph together with the SCC collector used to
/// analyse it, since the two are always threaded through the checker
/// together.
pub struct CallAnalysis<'a> {
    pub graph: &'a mut CallGraph,
    pub sccc: &'a mut SCCCollector,
}

pub struct CallGraphCollector<'a> {
    graph: &'a mut CallGraph,
    cx: &'a mut TypeCheckContext,
    from: SymbolId,
    calls: IndexSet<SymbolId>,
}

impl<'a> CallGraphCollector<'a> {
    pub fn new(from: SymbolId, graph: &'a mut CallGraph, cx: &'a mut TypeCheckContext) -> Self {
        Self {
            from,
            graph,
            cx,
            calls: IndexSet::new(),
        }
    }

    pub fn into_calls(self) -> IndexSet<SymbolId> {
        self.calls
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
                            self.calls.insert(to);
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

pub struct SCCCollector {
    index: HashMap<SymbolId, u32>,
    lowlink: HashMap<SymbolId, u32>,
    on_stack: HashSet<SymbolId>,
    stack: Vec<SymbolId>,
    next_index: u32,
    sccs: Vec<Vec<SymbolId>>,
}

impl SCCCollector {
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

    pub fn sccs(self) -> Vec<Vec<SymbolId>> {
        self.sccs
    }

    pub fn is_visited(&self, symbol: SymbolId) -> bool {
        self.index.contains_key(&symbol)
    }

    pub fn enter(&mut self, symbol: SymbolId) -> Frame<'_> {
        self.index.insert(symbol, self.next_index);
        self.lowlink.insert(symbol, self.next_index);
        self.next_index += 1;
        self.stack.push(symbol);
        self.on_stack.insert(symbol);
        Frame {
            sccc: self,
            symbol,
            finished: false,
        }
    }

    fn exit(&mut self, symbol: SymbolId) -> Option<Vec<SymbolId>> {
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
}

#[must_use]
pub struct Frame<'a> {
    sccc: &'a mut SCCCollector,
    symbol: SymbolId,
    finished: bool,
}

impl<'a> Frame<'a> {
    pub fn edge(
        &mut self,
        to: SymbolId,
        check: impl FnOnce(&mut SCCCollector) -> Option<Vec<SymbolId>>,
    ) -> Option<Vec<SymbolId>> {
        if !self.sccc.is_visited(to) {
            let completed = check(self.sccc);
            if self.sccc.is_visited(to) {
                let pulled = self.sccc.lowlink[&to];
                let mine = self.sccc.lowlink.get_mut(&self.symbol).unwrap();
                *mine = (*mine).min(pulled);
            }
            completed
        } else {
            if self.sccc.on_stack.contains(&to) {
                let idx = self.sccc.index[&to];
                let mine = self.sccc.lowlink.get_mut(&self.symbol).unwrap();
                *mine = (*mine).min(idx);
            }
            None
        }
    }

    pub fn finish(mut self) -> Option<Vec<SymbolId>> {
        self.finished = true;
        self.sccc.exit(self.symbol)
    }
}

impl Drop for Frame<'_> {
    fn drop(&mut self) {
        if !self.finished {
            panic!(
                "SCC frame for {:?} was dropped without calling `finish()`; \
                 the Tarjan stack is now inconsistent",
                self.symbol
            );
        }
    }
}
