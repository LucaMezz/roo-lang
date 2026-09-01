//! Defines [`RecursionTracker`], which tracks the state needed to
//! check functions in dependency order: which function is
//! currently being checked (and the chain of its enclosing
//! callers), the call graph built up as functions reference each
//! other, and Tarjan's algorithm's discovery of strongly connected
//! components (SCCs) within it, so that mutually recursive
//! functions can be generalized together once their whole SCC has
//! been checked.

use crate::DefId;
use crate::TypeCheckContext;
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
    ///
    /// Private: pairing this with [`Self::exit`] by hand is exactly
    /// the mistake [`TypeCheckContext::checking`] exists to rule
    /// out. Go through that instead.
    fn enter(&mut self, def: DefId) -> Option<DefId> {
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
    ///
    /// Private for the same reason as [`Self::enter`].
    fn exit(&mut self, def: DefId, parent: Option<DefId>) -> Option<Vec<DefId>> {
        self.stack.pop();
        self.current = parent;
        self.sccc.exit(def)
    }
}

impl<'ast> TypeCheckContext<'ast> {
    /// Runs `f` with `def` marked as currently being checked for the
    /// call-graph/SCC tracking in [`RecursionTracker`], then
    /// unconditionally restores the previous state and reports the
    /// strongly connected component completed by `def`, if any.
    ///
    /// [`RecursionTracker::enter`]/[`RecursionTracker::exit`] are
    /// private specifically so that this closure-scoped wrapper is
    /// the only way to reach them: pairing them by hand (as this
    /// crate used to do, once, in `check_fn_body`) leaves a call
    /// site free to add an early return between the two, or to
    /// exit with the wrong def, and the mismatch would only surface
    /// later as an index-panic inside [`SCCCollector`]. Here, `def`
    /// is guaranteed to be exited exactly once, with its own
    /// `parent`, regardless of how `f` returns.
    pub(crate) fn checking<R>(
        &mut self,
        def: DefId,
        f: impl FnOnce(&mut Self) -> R,
    ) -> (R, Option<Vec<DefId>>) {
        let parent = self.recursion.enter(def);
        let result = f(self);
        let scc = self.recursion.exit(def, parent);
        (result, scc)
    }
}

#[cfg(test)]
mod tests {
    use crate::tests::*;

    #[test]
    fn check_all_mutually_recursive_siblings_generalize_together_and_stay_reusable() {
        let source = indoc! {r#"
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
        "#};
        let mut cx = check_all(source);
        assert!(cx.diagnostics.is_empty());

        let ping_target = path(&mut cx.symbols, &["ping"]);
        let ping = cx
            .resolve_path_to_value(&ping_target)
            .expect("ping should resolve");
        let pong_target = path(&mut cx.symbols, &["pong"]);
        let pong = cx
            .resolve_path_to_value(&pong_target)
            .expect("pong should resolve");
        assert_eq!(cx.def(ping).generics().len(), 1);
        assert_eq!(cx.def(pong).generics().len(), 1);
        assert_eq!(cx.def(ping).generics()[0], cx.def(pong).generics()[0]);
    }

    #[test]
    fn check_all_a_one_directional_sibling_call_is_not_treated_as_a_cycle() {
        let source = indoc! {r#"
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
        "#};
        let mut cx = check_all(source);
        assert!(cx.diagnostics.is_empty());

        let helper_target = path(&mut cx.symbols, &["helper"]);
        let helper = cx
            .resolve_path_to_value(&helper_target)
            .expect("helper should resolve");
        let caller_target = path(&mut cx.symbols, &["caller"]);
        let caller = cx
            .resolve_path_to_value(&caller_target)
            .expect("caller should resolve");
        assert_eq!(cx.def(helper).generics().len(), 1);
        assert_eq!(cx.def(caller).generics().len(), 1);
    }

    #[test]
    fn check_all_self_recursive_function_generalizes_and_stays_reusable() {
        let source = indoc! {r#"
            fn identity_rec(x) {
                if true { x } else { identity_rec(x) }
            }
            fn use_it() {
                identity_rec(1);
                identity_rec("hi");
            }
        "#};
        let mut cx = check_all(source);
        assert!(cx.diagnostics.is_empty());

        let target = path(&mut cx.symbols, &["identity_rec"]);
        let identity_rec = cx
            .resolve_path_to_value(&target)
            .expect("identity_rec should resolve");
        assert_eq!(cx.def(identity_rec).generics().len(), 1);
    }

    #[test]
    fn check_all_a_three_way_cycle_generalizes_together() {
        let source = indoc! {r#"
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
        "#};
        let mut cx = check_all(source);
        assert!(cx.diagnostics.is_empty());

        for symbol in ["a", "b", "c"] {
            let target = path(&mut cx.symbols, &[symbol]);
            let def = cx
                .resolve_path_to_value(&target)
                .unwrap_or_else(|| panic!("{symbol} should resolve"));
            assert_eq!(cx.def(def).generics().len(), 1, "{symbol}");
        }
    }

    #[test]
    fn check_all_a_fully_annotated_cycle_has_nothing_left_to_generalize() {
        let source = indoc! {r#"
            fn ping2(x: int) -> int {
                pong2(x)
            }
            fn pong2(y: int) -> int {
                ping2(y)
            }
        "#};
        let mut cx = check_all(source);
        assert!(cx.diagnostics.is_empty());

        let ping2_target = path(&mut cx.symbols, &["ping2"]);
        let ping2 = cx
            .resolve_path_to_value(&ping2_target)
            .expect("ping2 should resolve");
        let pong2_target = path(&mut cx.symbols, &["pong2"]);
        let pong2 = cx
            .resolve_path_to_value(&pong2_target)
            .expect("pong2 should resolve");
        assert_eq!(cx.def(ping2).generics().len(), 0);
        assert_eq!(cx.def(pong2).generics().len(), 0);
    }

    #[test]
    fn check_all_a_newly_generalized_param_never_reuses_an_explicit_generics_symbol() {
        let source = indoc! {r#"
            fn compose<T>(f, g: Fn(int) -> _, x) -> Fn(T) -> String {
                f(g(x))
            }
        "#};
        let mut cx = check_all(source);
        assert!(cx.diagnostics.is_empty());

        let target = path(&mut cx.symbols, &["compose"]);
        let compose = cx
            .resolve_path_to_value(&target)
            .expect("compose should resolve");
        let generics = cx.def(compose).generics();
        assert_eq!(generics.len(), 2, "{generics:?}");

        let explicit = generics[0];
        let inferred = generics[1];
        assert_ne!(
            explicit, inferred,
            "should be two distinct DefIdOf<GenericParamDef>s"
        );

        let explicit_symbol = cx.generic_name(explicit);
        let inferred_symbol = cx.generic_name(inferred);
        assert_eq!(explicit_symbol, "T");
        assert_ne!(
            explicit_symbol, inferred_symbol,
            "the newly-generalized parameter must not render under the \
                 same symbol as the explicit `<T>`"
        );
    }

    #[test]
    fn check_all_nested_mutually_recursive_fns_generalize_together() {
        let source = indoc! {r#"
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
        "#};
        let mut cx = check_all(source);
        assert!(cx.diagnostics.is_empty());

        let target = path(&mut cx.symbols, &["outer"]);
        let outer = cx
            .resolve_path_to_value(&target)
            .expect("outer should resolve");
        let outer_scope = fn_body_scope(&cx, outer);

        let ping = declared_def(&cx, outer_scope, Namespace::Value, "ping")
            .expect("ping should be declared inside outer's body");
        let pong = declared_def(&cx, outer_scope, Namespace::Value, "pong")
            .expect("pong should be declared inside outer's body");
        assert_eq!(cx.def(ping).generics().len(), 1);
        assert_eq!(cx.def(pong).generics().len(), 1);
    }

    #[test]
    fn check_all_mutual_recursion_reached_only_through_a_nested_fn_value_is_one_scc() {
        let source = indoc! {r#"
            fn outer_1(x) {
                fn inner_1(y) {
                    outer_2(y)
                }
                let _inner_1 = inner_1;
                _inner_1(x)
            }

            fn outer_2(x) {
                fn inner_2(y) {
                    outer_1(y)
                }
                let _inner_2 = inner_2;
                _inner_2(x)
            }
        "#};
        let mut cx = check_all(source);
        assert!(cx.diagnostics.is_empty());

        let outer_1_target = path(&mut cx.symbols, &["outer_1"]);
        let outer_1 = cx
            .resolve_path_to_value(&outer_1_target)
            .expect("outer_1 should resolve");
        let outer_2_target = path(&mut cx.symbols, &["outer_2"]);
        let outer_2 = cx
            .resolve_path_to_value(&outer_2_target)
            .expect("outer_2 should resolve");
        let inner_1 = declared_def(
            &cx,
            fn_body_scope(&cx, outer_1),
            Namespace::Value,
            "inner_1",
        )
        .expect("inner_1 should be declared inside outer_1's body");
        let inner_2 = declared_def(
            &cx,
            fn_body_scope(&cx, outer_2),
            Namespace::Value,
            "inner_2",
        )
        .expect("inner_2 should be declared inside outer_2's body");

        let expected = "<T, U> Fn(T) -> U";
        assert_eq!(cx.renderer().render_def_type(outer_1), expected);
        assert_eq!(cx.renderer().render_def_type(outer_2), expected);
        assert_eq!(cx.renderer().render_def_type(inner_1), expected);
        assert_eq!(cx.renderer().render_def_type(inner_2), expected);
    }

    #[test]
    fn check_all_a_nested_fn_never_generalizes_a_variable_free_in_an_enclosing_signature() {
        let source = indoc! {r#"
            fn compose(f) {
                fn inner(g) {
                    fn innermost(x) {
                        f(g(x))
                    }
                    innermost
                }
                inner
            }
        "#};
        let mut cx = check_all(source);
        assert!(cx.diagnostics.is_empty());

        let target = path(&mut cx.symbols, &["compose"]);
        let compose = cx
            .resolve_path_to_value(&target)
            .expect("compose should resolve");
        let compose_scope = fn_body_scope(&cx, compose);

        let inner = declared_def(&cx, compose_scope, Namespace::Value, "inner")
            .expect("inner should be declared inside compose's body");
        let inner_scope = fn_body_scope(&cx, inner);
        let innermost = declared_def(&cx, inner_scope, Namespace::Value, "innermost")
            .expect("innermost should be declared inside inner's body");

        assert_eq!(
            cx.def(compose).generics().len(),
            3,
            "{:#?}",
            cx.def(compose).generics()
        );
        // `inner` legitimately generalizes 1 variable of its own: the type shared
        // between `innermost`'s parameter `x` and `inner`'s own parameter `g`'s
        // domain. That variable is not free in the enclosing signature (`f`'s
        // domain/codomain never mention it), so under standard let-polymorphism
        // it's sound for `inner` to generalize it rather than deferring to
        // `compose`. The other 2 variables that stay free at this point (`f`'s
        // domain and codomain) are correctly excluded, and end up on `compose`
        // instead -- which is what this test is actually asserting.
        assert_eq!(cx.def(inner).generics().len(), 1);
        assert_eq!(cx.def(innermost).generics().len(), 0);
    }
}
