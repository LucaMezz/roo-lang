//! Defines [`RecursionTracker`], which tracks the state needed to
//! check functions in dependency order: which function is
//! currently being checked (and the chain of its enclosing
//! callers), the call graph built up as functions reference each
//! other, and Tarjan's algorithm's discovery of strongly connected
//! components (SCCs) within it, so that mutually recursive
//! functions can be generalized together once their whole SCC has
//! been checked.

use crate::DefId;
use crate::call_graph::{CallGraph, SCCCollector};

pub(crate) struct RecursionTracker {
    graph: CallGraph,
    sccc: SCCCollector,
    current: Option<DefId>,
    stack: Vec<DefId>,
}

impl RecursionTracker {
    pub(crate) fn new() -> Self {
        Self {
            graph: CallGraph::new(),
            sccc: SCCCollector::new(),
            current: None,
            stack: Vec::new(),
        }
    }

    /// The def currently being checked, if any.
    pub(crate) fn current(&self) -> Option<DefId> {
        self.current
    }

    /// The chain of defs currently being checked, outermost
    /// first. Used to tell which free type variables belong to an
    /// enclosing function, so a nested function does not
    /// generalize them itself.
    pub(crate) fn stack(&self) -> &[DefId] {
        &self.stack
    }

    /// Whether `def` has already begun being checked.
    pub(crate) fn is_visited(&self, def: DefId) -> bool {
        self.sccc.is_visited(def)
    }

    /// Records a call from `from` to `to` in the call graph.
    pub(crate) fn record_call(&mut self, from: DefId, to: DefId) {
        self.graph.call(from, to);
    }

    pub(crate) fn pull_lowlink(&mut self, from: DefId, to: DefId) {
        self.sccc.pull_lowlink(from, to);
    }

    pub(crate) fn note_back_edge(&mut self, from: DefId, to: DefId) {
        self.sccc.note_back_edge(from, to);
    }

    /// Begins checking `def`: pushes it as the current function
    /// and onto the ancestor stack, and starts Tarjan's algorithm
    /// for it. Returns the previously-current def, to be handed
    /// back to [`Self::exit`] once `def` finishes checking.
    pub(crate) fn enter(&mut self, def: DefId) -> Option<DefId> {
        self.sccc.enter(def);
        let parent = self.current;
        self.current = Some(def);
        self.stack.push(def);
        parent
    }

    /// Finishes checking `def`, restoring `parent` (as returned by
    /// [`Self::enter`]) as the current function. Returns the
    /// strongly connected component that was completed, if `def`
    /// was its root.
    pub(crate) fn exit(&mut self, def: DefId, parent: Option<DefId>) -> Option<Vec<DefId>> {
        self.stack.pop();
        self.current = parent;
        self.sccc.exit(def)
    }
}
