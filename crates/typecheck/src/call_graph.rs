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

pub struct CallGraphCollector<'a, 'ast> {
    cx: &'a mut TypeCheckContext<'ast>,
    from: SymbolId,
    calls: IndexSet<SymbolId>,
}

impl<'a, 'ast> CallGraphCollector<'a, 'ast> {
    pub fn new(from: SymbolId, cx: &'a mut TypeCheckContext<'ast>) -> Self {
        Self {
            from,
            cx,
            calls: IndexSet::new(),
        }
    }

    pub fn into_calls(self) -> IndexSet<SymbolId> {
        self.calls
    }
}

impl<'a, 'ast> Visitor for CallGraphCollector<'a, 'ast> {
    fn visit_expr(&mut self, expr: &Expr) {
        match &expr.kind {
            ExprKind::Path(_, path) => {
                if let Some(to) = self.cx.resolve_path(path, Namespace::Value) {
                    if let Some(symbol) = self.cx.symbols.get(to) {
                        if let SymbolKind::Fn(_) = symbol.kind {
                            self.cx.graph.call(self.from, to);
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

    pub fn is_on_stack(&self, symbol: SymbolId) -> bool {
        self.on_stack.contains(&symbol)
    }

    pub fn enter(&mut self, symbol: SymbolId) {
        self.index.insert(symbol, self.next_index);
        self.lowlink.insert(symbol, self.next_index);
        self.next_index += 1;
        self.stack.push(symbol);
        self.on_stack.insert(symbol);
    }

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

    pub fn pull_lowlink(&mut self, from: SymbolId, to: SymbolId) {
        let pulled = self.lowlink[&to];
        let mine = self.lowlink.get_mut(&from).unwrap();
        *mine = (*mine).min(pulled);
    }

    pub fn note_back_edge(&mut self, from: SymbolId, to: SymbolId) {
        if self.on_stack.contains(&to) {
            let idx = self.index[&to];
            let mine = self.lowlink.get_mut(&from).unwrap();
            *mine = (*mine).min(idx);
        }
    }
}
