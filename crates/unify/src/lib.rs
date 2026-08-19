//! Facilitates the creation and unification of terms
use std::fmt;
use std::mem;

use slotmap::SlotMap;
use union_find::{QuickUnionUf, Union, UnionFind, UnionResult};

slotmap::new_key_type! {
    pub struct TermId;

    pub struct VarId;
}

#[derive(Debug, Clone, PartialEq)]
pub enum Term<C> {
    Var(VarId),
    App { constructor: C, args: Vec<TermId> },
}

#[macro_export]
macro_rules! term {
    ($cx:expr, var $id:expr) => {
        $cx.insert_term($crate::Term::Var($id))
    };
    ($cx:expr, $constructor:expr => [ $($arg:expr),* $(,)? ]) => {{
        let args = ::std::vec![ $($arg),* ];
        $cx.insert_term($crate::Term::App {
            constructor: $constructor,
            args,
        })
    }};
    ($cx:expr, $constructor:expr => $args:expr) => {{
        $cx.insert_term($crate::Term::App {
            constructor: $constructor,
            args: $args,
        })
    }};
    ($cx:expr, $constructor:expr) => {
        $cx.insert_term($crate::Term::App {
            constructor: $constructor,
            args: ::std::vec::Vec::new(),
        })
    };
}

#[derive(Clone, Copy, Default)]
struct Binding(Option<TermId>);

impl Union for Binding {
    fn union(lval: Self, rval: Self) -> UnionResult<Self> {
        match lval.0 {
            Some(_) => UnionResult::Left(lval),
            None => UnionResult::Right(rval),
        }
    }
}

struct ProvenanceEntry<R> {
    var: VarId,
    reason: R,
}

pub struct UnificationContext<C, R = ()> {
    terms: SlotMap<TermId, Term<C>>,
    var_slots: SlotMap<VarId, usize>,
    uf_key_to_var: Vec<VarId>,
    bindings: QuickUnionUf<Binding>,
    wildcards: Vec<C>,
    provenance: Vec<ProvenanceEntry<R>>,
}

impl<C, R> UnificationContext<C, R> {
    pub fn new() -> Self {
        Self {
            terms: SlotMap::with_key(),
            var_slots: SlotMap::with_key(),
            uf_key_to_var: Vec::new(),
            bindings: QuickUnionUf::new(0),
            wildcards: Vec::new(),
            provenance: Vec::new(),
        }
    }

    pub fn with_wildcard(wildcard: C) -> Self {
        Self::with_wildcards(vec![wildcard])
    }

    pub fn with_wildcards(wildcards: Vec<C>) -> Self {
        Self {
            terms: SlotMap::with_key(),
            var_slots: SlotMap::with_key(),
            uf_key_to_var: Vec::new(),
            bindings: QuickUnionUf::new(0),
            wildcards,
            provenance: Vec::new(),
        }
    }

    pub fn insert_term(&mut self, term: Term<C>) -> TermId {
        self.terms.insert(term)
    }

    pub fn term(&self, id: TermId) -> Option<&Term<C>> {
        self.terms.get(id)
    }

    pub fn fresh_var(&mut self) -> VarId {
        let uf_key = self.bindings.insert(Binding(None));
        let var = self.var_slots.insert(uf_key);
        debug_assert_eq!(uf_key, self.uf_key_to_var.len());
        self.uf_key_to_var.push(var);
        var
    }

    pub fn find(&mut self, var: VarId) -> VarId {
        let uf_key = self.var_slots[var];
        let root_key = self.bindings.find(uf_key);
        self.uf_key_to_var[root_key]
    }

    pub fn binding(&mut self, var: VarId) -> Option<TermId> {
        let uf_key = self.var_slots[var];
        self.bindings.get(uf_key).0
    }

    pub fn bind(&mut self, var: VarId, term: TermId) -> Option<TermId> {
        let uf_key = self.var_slots[var];
        mem::replace(self.bindings.get_mut(uf_key), Binding(Some(term))).0
    }

    pub fn union_vars(&mut self, a: VarId, b: VarId) -> bool {
        let ka = self.var_slots[a];
        let kb = self.var_slots[b];
        self.bindings.union(ka, kb)
    }

    pub fn resolve(&mut self, id: TermId) -> TermId {
        match self.term(id).expect("valid TermId") {
            Term::Var(v) => {
                let v = *v;
                match self.binding(v) {
                    Some(bound_id) => self.resolve(bound_id),
                    None => id,
                }
            }
            Term::App { .. } => id,
        }
    }

    fn occurs(&mut self, v: VarId, id: TermId) -> bool {
        let id = self.resolve(id);
        match self.term(id).expect("valid TermId") {
            Term::Var(v2) => {
                let v2 = *v2;
                self.find(v) == self.find(v2)
            }
            Term::App { args, .. } => {
                let args = args.clone();
                args.iter().any(|&arg| self.occurs(v, arg))
            }
        }
    }

    pub fn unify(&mut self, t1: TermId, t2: TermId) -> Result<(), UnifyError<C>>
    where
        C: Clone + fmt::Debug + PartialEq,
        R: Clone,
    {
        self.unify_impl(t1, t2, None)
    }

    pub fn unify_because(&mut self, t1: TermId, t2: TermId, reason: R) -> Result<(), UnifyError<C>>
    where
        C: Clone + fmt::Debug + PartialEq,
        R: Clone,
    {
        self.unify_impl(t1, t2, Some(reason))
    }

    fn unify_impl(&mut self, t1: TermId, t2: TermId, reason: Option<R>) -> Result<(), UnifyError<C>>
    where
        C: Clone + fmt::Debug + PartialEq,
        R: Clone,
    {
        let resolved_t1 = self.resolve(t1);
        let resolved_t2 = self.resolve(t2);

        match (
            self.term(resolved_t1).expect("valid TermId"),
            self.term(resolved_t2).expect("valid TermId"),
        ) {
            (Term::Var(v1), Term::Var(v2)) => {
                let (v1, v2) = (*v1, *v2);
                if self.find(v1) != self.find(v2) {
                    self.union_vars(v1, v2);
                    if let Some(reason) = reason {
                        self.record_provenance(v1, reason);
                    }
                }
                Ok(())
            }
            (Term::Var(v), _) => {
                let v = *v;
                if self.occurs(v, resolved_t2) {
                    return Err(UnifyError::OccursCheck(v));
                }
                self.bind(v, resolved_t2);
                if let Some(reason) = reason {
                    self.record_provenance(v, reason);
                }
                Ok(())
            }
            (_, Term::Var(v)) => {
                let v = *v;
                if self.occurs(v, resolved_t1) {
                    return Err(UnifyError::OccursCheck(v));
                }
                self.bind(v, resolved_t1);
                if let Some(reason) = reason {
                    self.record_provenance(v, reason);
                }
                Ok(())
            }
            (
                Term::App {
                    constructor: c1,
                    args: args1,
                },
                Term::App {
                    constructor: c2,
                    args: args2,
                },
            ) => {
                if self.wildcards.contains(c1) || self.wildcards.contains(c2) {
                    return Ok(());
                }
                if c1 != c2 {
                    return Err(UnifyError::ConstructorMismatch {
                        t1,
                        c1: c1.clone(),
                        t2,
                        c2: c2.clone(),
                    });
                }
                if args1.len() != args2.len() {
                    return Err(UnifyError::ArityMismatch {
                        t1,
                        arity1: args1.len(),
                        t2,
                        arity2: args2.len(),
                    });
                }
                let pairs: Vec<(TermId, TermId)> =
                    args1.iter().copied().zip(args2.iter().copied()).collect();
                for (x, y) in pairs {
                    self.unify_impl(x, y, reason.clone())?;
                }
                Ok(())
            }
        }
    }

    fn record_provenance(&mut self, var: VarId, reason: R) {
        self.provenance.push(ProvenanceEntry { var, reason });
    }

    /// Retrieves the most recent recorded provenance related
    /// to the type variable provided.
    pub fn provenance(&mut self, var: VarId) -> Option<&R> {
        // Get the representative of the set.
        let target = self.find(var);
        let mut found = None;
        // Search backwards through the provenance list to
        // find the most recent entry which has the same
        // representative as the `var` provided.
        for i in (0..self.provenance.len()).rev() {
            let entry_var = self.provenance[i].var;
            if self.find(entry_var) == target {
                found = Some(i);
                break;
            }
        }
        found.map(|i| &self.provenance[i].reason)
    }
}

impl<C, R> Default for UnificationContext<C, R> {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, thiserror::Error)]
pub enum UnifyError<C> {
    #[error("constructor mismatch: {c1:?} != {c2:?}")]
    ConstructorMismatch {
        t1: TermId,
        c1: C,
        t2: TermId,
        c2: C,
    },
    #[error("arity mismatch: {arity1} args vs {arity2} args")]
    ArityMismatch {
        t1: TermId,
        arity1: usize,
        t2: TermId,
        arity2: usize,
    },
    #[error("occurs check failed for {0:?}")]
    OccursCheck(VarId),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fresh_vars_start_distinct_and_unbound() {
        let mut cx = UnificationContext::<()>::new();
        let a = cx.fresh_var();
        let b = cx.fresh_var();
        assert_ne!(a, b);
        assert_eq!(cx.binding(a), None);
        assert_eq!(cx.binding(b), None);
        assert_eq!(cx.find(a), a);
        assert_eq!(cx.find(b), b);
    }

    #[test]
    fn union_vars_makes_them_find_equal() {
        let mut cx = UnificationContext::<()>::new();
        let a = cx.fresh_var();
        let b = cx.fresh_var();
        assert!(cx.union_vars(a, b));
        assert_eq!(cx.find(a), cx.find(b));
        assert!(!cx.union_vars(a, b));
    }

    #[test]
    fn bind_then_binding_round_trips() {
        let mut cx = UnificationContext::<()>::new();
        let t = cx.insert_term(Term::App {
            constructor: (),
            args: vec![],
        });
        let v = cx.fresh_var();
        assert_eq!(cx.bind(v, t), None);
        assert_eq!(cx.binding(v), Some(t));
    }

    #[test]
    fn bind_returns_the_previous_binding() {
        let mut cx = UnificationContext::<()>::new();
        let t1 = cx.insert_term(Term::App {
            constructor: (),
            args: vec![],
        });
        let t2 = cx.insert_term(Term::App {
            constructor: (),
            args: vec![],
        });
        let v = cx.fresh_var();
        cx.bind(v, t1);
        assert_eq!(cx.bind(v, t2), Some(t1));
        assert_eq!(cx.binding(v), Some(t2));
    }

    #[test]
    fn binding_survives_union_with_an_unbound_var() {
        let mut cx = UnificationContext::<()>::new();
        let t = cx.insert_term(Term::App {
            constructor: (),
            args: vec![],
        });
        let bound = cx.fresh_var();
        let free = cx.fresh_var();
        cx.bind(bound, t);
        cx.union_vars(bound, free);
        assert_eq!(cx.binding(bound), Some(t));
        assert_eq!(cx.binding(free), Some(t));
    }

    #[test]
    fn insert_term_and_term_round_trip() {
        let mut cx = UnificationContext::<&'static str>::new();
        let leaf = cx.insert_term(Term::App {
            constructor: "int",
            args: vec![],
        });
        let id = cx.insert_term(Term::App {
            constructor: "list",
            args: vec![leaf],
        });
        let Some(Term::App { constructor, args }) = cx.term(id) else {
            panic!("expected Term::App");
        };
        assert_eq!(*constructor, "list");
        assert_eq!(args, &vec![leaf]);
    }

    #[test]
    fn term_returns_none_for_a_foreign_id() {
        let cx_a = UnificationContext::<()>::new();
        let mut cx_b = UnificationContext::<()>::new();
        let id_from_b = cx_b.insert_term(Term::App {
            constructor: (),
            args: vec![],
        });
        assert!(cx_a.term(id_from_b).is_none());
    }

    #[test]
    fn unify_identical_constructors_succeeds() {
        let mut cx = UnificationContext::<&'static str>::new();
        let a = cx.insert_term(Term::App {
            constructor: "Int",
            args: vec![],
        });
        let b = cx.insert_term(Term::App {
            constructor: "Int",
            args: vec![],
        });
        assert!(cx.unify(a, b).is_ok());
    }

    #[test]
    fn unify_different_constructors_fails() {
        let mut cx = UnificationContext::<&'static str>::new();
        let a = cx.insert_term(Term::App {
            constructor: "Int",
            args: vec![],
        });
        let b = cx.insert_term(Term::App {
            constructor: "Bool",
            args: vec![],
        });
        assert!(cx.unify(a, b).is_err());
    }

    #[test]
    fn constructor_mismatch_at_the_top_level_carries_the_top_level_terms() {
        let mut cx = UnificationContext::<&'static str>::new();
        let a = cx.insert_term(Term::App {
            constructor: "Int",
            args: vec![],
        });
        let b = cx.insert_term(Term::App {
            constructor: "Bool",
            args: vec![],
        });
        let Err(UnifyError::ConstructorMismatch { t1, c1, t2, c2 }) = cx.unify(a, b) else {
            panic!("expected a ConstructorMismatch");
        };
        assert_eq!((t1, c1, t2, c2), (a, "Int", b, "Bool"));
    }

    #[test]
    fn constructor_mismatch_nested_in_a_matching_outer_constructor_carries_the_inner_terms() {
        let mut cx = UnificationContext::<&'static str>::new();
        let int_term = cx.insert_term(Term::App {
            constructor: "Int",
            args: vec![],
        });
        let bool_term = cx.insert_term(Term::App {
            constructor: "Bool",
            args: vec![],
        });
        let list_of_int = cx.insert_term(Term::App {
            constructor: "List",
            args: vec![int_term],
        });
        let list_of_bool = cx.insert_term(Term::App {
            constructor: "List",
            args: vec![bool_term],
        });

        let Err(UnifyError::ConstructorMismatch { t1, c1, t2, c2 }) =
            cx.unify(list_of_int, list_of_bool)
        else {
            panic!("expected a ConstructorMismatch");
        };
        assert_eq!((t1, c1, t2, c2), (int_term, "Int", bool_term, "Bool"));
    }

    #[test]
    fn arity_mismatch_carries_the_specific_terms_compared() {
        let mut cx = UnificationContext::<&'static str>::new();
        let unary_arg = cx.insert_term(Term::App {
            constructor: "Int",
            args: vec![],
        });
        let unary = cx.insert_term(Term::App {
            constructor: "Fn",
            args: vec![unary_arg],
        });
        let elem_a = cx.insert_term(Term::App {
            constructor: "Int",
            args: vec![],
        });
        let elem_b = cx.insert_term(Term::App {
            constructor: "Bool",
            args: vec![],
        });
        let binary = cx.insert_term(Term::App {
            constructor: "Fn",
            args: vec![elem_a, elem_b],
        });

        let Err(UnifyError::ArityMismatch {
            t1,
            arity1,
            t2,
            arity2,
        }) = cx.unify(unary, binary)
        else {
            panic!("expected an ArityMismatch");
        };
        assert_eq!((t1, arity1, t2, arity2), (unary, 1, binary, 2));
    }

    #[test]
    fn unify_binds_a_free_variable() {
        let mut cx = UnificationContext::<&'static str>::new();
        let v = cx.fresh_var();
        let v_term = cx.insert_term(Term::Var(v));
        let int_term = cx.insert_term(Term::App {
            constructor: "Int",
            args: vec![],
        });
        assert!(cx.unify(v_term, int_term).is_ok());
        assert_eq!(cx.binding(v), Some(int_term));
    }

    #[test]
    fn unify_recurses_into_matching_constructors() {
        let mut cx = UnificationContext::<&'static str>::new();
        let v = cx.fresh_var();
        let v_term = cx.insert_term(Term::Var(v));
        let int_term = cx.insert_term(Term::App {
            constructor: "Int",
            args: vec![],
        });
        let list_of_var = cx.insert_term(Term::App {
            constructor: "List",
            args: vec![v_term],
        });
        let list_of_int = cx.insert_term(Term::App {
            constructor: "List",
            args: vec![int_term],
        });
        assert!(cx.unify(list_of_var, list_of_int).is_ok());
        assert_eq!(cx.binding(v), Some(int_term));
    }

    #[test]
    fn unify_rejects_a_variable_already_bound_to_something_else() {
        let mut cx = UnificationContext::<&'static str>::new();
        let v = cx.fresh_var();
        let v_term = cx.insert_term(Term::Var(v));
        let int_term = cx.insert_term(Term::App {
            constructor: "Int",
            args: vec![],
        });
        let bool_term = cx.insert_term(Term::App {
            constructor: "Bool",
            args: vec![],
        });

        cx.unify(v_term, int_term).unwrap();
        let result = cx.unify(v_term, bool_term);

        assert!(
            result.is_err(),
            "v was already bound to Int; unifying it with Bool should fail"
        );
        assert_eq!(
            cx.binding(v),
            Some(int_term),
            "the rejected attempt must not have disturbed the existing binding"
        );
    }

    #[test]
    fn constructor_mismatch_against_a_bound_variable_reports_the_variable_unresolved() {
        let mut cx = UnificationContext::<&'static str>::new();
        let v = cx.fresh_var();
        let v_term = cx.insert_term(Term::Var(v));
        let int_term = cx.insert_term(Term::App {
            constructor: "Int",
            args: vec![],
        });
        let bool_term = cx.insert_term(Term::App {
            constructor: "Bool",
            args: vec![],
        });

        cx.unify(v_term, int_term).unwrap();
        let Err(UnifyError::ConstructorMismatch { t1, c1, t2, c2 }) = cx.unify(v_term, bool_term)
        else {
            panic!("expected a ConstructorMismatch");
        };

        assert_eq!(
            t1, v_term,
            "t1 should be ?v itself, not what it resolves to"
        );
        assert_eq!(c1, "Int", "the *constructor* is still the resolved one");
        assert_eq!((t2, c2), (bool_term, "Bool"));
        assert_eq!(
            cx.term(t1),
            Some(&Term::Var(v)),
            "confirms t1 is still literally a Term::Var, ready for a caller to look up"
        );
    }

    #[test]
    fn wildcard_unifies_with_anything() {
        let mut cx = UnificationContext::<&'static str>::with_wildcard("any");
        let any_term = cx.insert_term(Term::App {
            constructor: "any",
            args: vec![],
        });
        let int_term = cx.insert_term(Term::App {
            constructor: "Int",
            args: vec![],
        });
        assert!(cx.unify(any_term, int_term).is_ok());
        assert!(cx.unify(int_term, any_term).is_ok());
    }

    #[test]
    fn wildcard_does_not_require_matching_arity() {
        let mut cx = UnificationContext::<&'static str>::with_wildcard("any");
        let any_term = cx.insert_term(Term::App {
            constructor: "any",
            args: vec![],
        });
        let elem = cx.insert_term(Term::App {
            constructor: "Int",
            args: vec![],
        });
        let list_term = cx.insert_term(Term::App {
            constructor: "List",
            args: vec![elem],
        });
        assert!(cx.unify(any_term, list_term).is_ok());
    }

    #[test]
    fn with_wildcards_registers_more_than_one_absorbing_constructor() {
        let mut cx = UnificationContext::<&'static str>::with_wildcards(vec!["any", "err"]);
        let any_term = cx.insert_term(Term::App {
            constructor: "any",
            args: vec![],
        });
        let err_term = cx.insert_term(Term::App {
            constructor: "err",
            args: vec![],
        });
        let int_term = cx.insert_term(Term::App {
            constructor: "Int",
            args: vec![],
        });
        assert!(cx.unify(any_term, int_term).is_ok());
        assert!(cx.unify(err_term, int_term).is_ok());
    }

    #[test]
    fn a_wildcard_short_circuits_even_nested_inside_a_matching_constructor() {
        let mut cx = UnificationContext::<&'static str>::with_wildcards(vec!["any", "err"]);
        let err_term = cx.insert_term(Term::App {
            constructor: "err",
            args: vec![],
        });
        let int_term = cx.insert_term(Term::App {
            constructor: "Int",
            args: vec![],
        });
        let list_of_err = cx.insert_term(Term::App {
            constructor: "List",
            args: vec![err_term],
        });
        let list_of_int = cx.insert_term(Term::App {
            constructor: "List",
            args: vec![int_term],
        });
        assert!(cx.unify(list_of_err, list_of_int).is_ok());
    }

    #[test]
    fn without_a_registered_wildcard_the_same_named_constructor_gets_no_special_treatment() {
        let mut cx = UnificationContext::<&'static str>::new();
        let a = cx.insert_term(Term::App {
            constructor: "any",
            args: vec![],
        });
        let b = cx.insert_term(Term::App {
            constructor: "Int",
            args: vec![],
        });
        assert!(cx.unify(a, b).is_err());
    }

    #[test]
    fn resolve_follows_a_chain_of_bound_variables() {
        let mut cx = UnificationContext::<&'static str>::new();
        let a = cx.fresh_var();
        let b = cx.fresh_var();
        let a_term = cx.insert_term(Term::Var(a));
        let b_term = cx.insert_term(Term::Var(b));
        let int_term = cx.insert_term(Term::App {
            constructor: "Int",
            args: vec![],
        });

        cx.bind(a, b_term);
        cx.bind(b, int_term);

        assert_eq!(cx.resolve(a_term), int_term);
    }

    #[test]
    fn provenance_is_none_for_an_untouched_variable() {
        let mut cx = UnificationContext::<&'static str, &'static str>::new();
        let v = cx.fresh_var();
        assert_eq!(cx.provenance(v), None);
    }

    #[test]
    fn plain_unify_records_no_provenance() {
        let mut cx = UnificationContext::<&'static str, &'static str>::new();
        let v = cx.fresh_var();
        let v_term = cx.insert_term(Term::Var(v));
        let int_term = cx.insert_term(Term::App {
            constructor: "Int",
            args: vec![],
        });

        cx.unify(v_term, int_term).unwrap();

        assert_eq!(cx.binding(v), Some(int_term), "unify itself still binds");
        assert_eq!(
            cx.provenance(v),
            None,
            "but plain unify records no reason for it"
        );
    }

    #[test]
    fn unify_because_records_the_reason_a_variable_was_bound() {
        let mut cx = UnificationContext::<&'static str, &'static str>::new();
        let v = cx.fresh_var();
        let v_term = cx.insert_term(Term::Var(v));
        let int_term = cx.insert_term(Term::App {
            constructor: "Int",
            args: vec![],
        });

        cx.unify_because(v_term, int_term, "because line 12")
            .unwrap();

        assert_eq!(cx.provenance(v), Some(&"because line 12"));
    }

    #[test]
    fn unify_because_records_the_reason_for_a_var_var_merge() {
        let mut cx = UnificationContext::<&'static str, &'static str>::new();
        let a = cx.fresh_var();
        let b = cx.fresh_var();
        let a_term = cx.insert_term(Term::Var(a));
        let b_term = cx.insert_term(Term::Var(b));

        cx.unify_because(a_term, b_term, "because they flow together")
            .unwrap();

        assert_eq!(cx.provenance(a), Some(&"because they flow together"));
        assert_eq!(cx.provenance(b), Some(&"because they flow together"));
    }

    #[test]
    fn unify_because_threads_the_same_reason_through_nested_constructors() {
        let mut cx = UnificationContext::<&'static str, &'static str>::new();
        let v = cx.fresh_var();
        let v_term = cx.insert_term(Term::Var(v));
        let int_term = cx.insert_term(Term::App {
            constructor: "Int",
            args: vec![],
        });
        let list_of_var = cx.insert_term(Term::App {
            constructor: "List",
            args: vec![v_term],
        });
        let list_of_int = cx.insert_term(Term::App {
            constructor: "List",
            args: vec![int_term],
        });

        cx.unify_because(list_of_var, list_of_int, "because of this list")
            .unwrap();

        assert_eq!(cx.provenance(v), Some(&"because of this list"));
    }

    #[test]
    fn provenance_reflects_the_most_recent_binding() {
        let mut cx = UnificationContext::<&'static str, &'static str>::new();
        let a = cx.fresh_var();
        let b = cx.fresh_var();
        let a_term = cx.insert_term(Term::Var(a));
        let b_term = cx.insert_term(Term::Var(b));
        let int_term = cx.insert_term(Term::App {
            constructor: "Int",
            args: vec![],
        });
        let bool_term = cx.insert_term(Term::App {
            constructor: "Bool",
            args: vec![],
        });

        cx.unify_because(a_term, int_term, "a is Int here").unwrap();
        cx.unify_because(b_term, bool_term, "b is Bool here")
            .unwrap();

        assert_eq!(cx.provenance(a), Some(&"a is Int here"));
        assert_eq!(cx.provenance(b), Some(&"b is Bool here"));
    }

    #[test]
    fn provenance_survives_a_later_merge_into_the_same_class() {
        let mut cx = UnificationContext::<&'static str, &'static str>::new();
        let a = cx.fresh_var();
        let b = cx.fresh_var();
        let a_term = cx.insert_term(Term::Var(a));
        let int_term = cx.insert_term(Term::App {
            constructor: "Int",
            args: vec![],
        });

        cx.unify_because(a_term, int_term, "a was pinned here")
            .unwrap();
        assert_eq!(cx.provenance(b), None, "b is unrelated so far");

        cx.union_vars(a, b);

        assert_eq!(cx.provenance(b), Some(&"a was pinned here"));
    }

    #[test]
    fn term_macro_builds_a_zero_arg_constructor() {
        let mut cx = UnificationContext::<&'static str>::new();
        let id = term!(cx, "Int");
        assert_eq!(
            cx.term(id),
            Some(&Term::App {
                constructor: "Int",
                args: vec![],
            })
        );
    }

    #[test]
    fn term_macro_builds_nested_constructors() {
        let mut cx = UnificationContext::<&'static str>::new();
        let id = term!(cx, "List" => [term!(cx, "Int")]);
        let Some(Term::App { constructor, args }) = cx.term(id) else {
            panic!("expected Term::App");
        };
        assert_eq!(*constructor, "List");
        assert_eq!(args.len(), 1);
        assert_eq!(
            cx.term(args[0]),
            Some(&Term::App {
                constructor: "Int",
                args: vec![],
            })
        );
    }

    #[test]
    fn term_macro_builds_multiple_args_and_deep_nesting() {
        let mut cx = UnificationContext::<&'static str>::new();
        let id = term!(cx, "Fn" => [
            term!(cx, "Int"),
            term!(cx, "Pair" => [term!(cx, "Bool"), term!(cx, "Int")]),
            term!(cx, "Bool"),
        ]);
        let Some(Term::App { args, .. }) = cx.term(id) else {
            panic!("expected Term::App");
        };
        assert_eq!(args.len(), 3);
        let Some(Term::App {
            constructor: pair_constructor,
            args: pair_args,
        }) = cx.term(args[1])
        else {
            panic!("expected Term::App");
        };
        assert_eq!(*pair_constructor, "Pair");
        assert_eq!(pair_args.len(), 2);
    }

    #[test]
    fn term_macro_accepts_a_plain_expression_as_an_arg() {
        let mut cx = UnificationContext::<&'static str>::new();
        let existing = cx.insert_term(Term::App {
            constructor: "Int",
            args: vec![],
        });
        let id = term!(cx, "List" => [existing]);
        let Some(Term::App { args, .. }) = cx.term(id) else {
            panic!("expected Term::App");
        };
        assert_eq!(args[0], existing);
    }

    #[test]
    fn term_macro_wraps_a_var_id() {
        let mut cx = UnificationContext::<&'static str>::new();
        let v = cx.fresh_var();
        let id = term!(cx, var v);
        assert_eq!(cx.term(id), Some(&Term::Var(v)));
    }

    use proptest::prelude::*;

    #[derive(Debug, Clone)]
    enum Shape {
        Var(u8),
        Int,
        Bool,
        List(Box<Shape>),
    }

    fn any_shape() -> impl Strategy<Value = Shape> {
        let leaf = prop_oneof![
            (0u8..4).prop_map(Shape::Var),
            Just(Shape::Int),
            Just(Shape::Bool),
        ];
        leaf.prop_recursive(4, 16, 2, |inner| {
            inner.prop_map(|s| Shape::List(Box::new(s)))
        })
    }

    fn ground_shape() -> impl Strategy<Value = Shape> {
        let leaf = prop_oneof![Just(Shape::Int), Just(Shape::Bool)];
        leaf.prop_recursive(4, 16, 2, |inner| {
            inner.prop_map(|s| Shape::List(Box::new(s)))
        })
    }

    fn materialize(
        cx: &mut UnificationContext<&'static str>,
        vars: &[TermId],
        shape: &Shape,
    ) -> TermId {
        match shape {
            Shape::Var(i) => vars[*i as usize % vars.len()],
            Shape::Int => cx.insert_term(Term::App {
                constructor: "Int",
                args: vec![],
            }),
            Shape::Bool => cx.insert_term(Term::App {
                constructor: "Bool",
                args: vec![],
            }),
            Shape::List(inner) => {
                let inner_id = materialize(cx, vars, inner);
                cx.insert_term(Term::App {
                    constructor: "List",
                    args: vec![inner_id],
                })
            }
        }
    }

    fn fresh_context_with_vars(n: usize) -> (UnificationContext<&'static str>, Vec<TermId>) {
        let mut cx = UnificationContext::<&'static str>::new();
        let vars = (0..n)
            .map(|_| {
                let v = cx.fresh_var();
                cx.insert_term(Term::Var(v))
            })
            .collect();
        (cx, vars)
    }

    proptest! {
        #[test]
        fn unify_with_self_always_succeeds(shape in any_shape()) {
            let (mut cx, vars) = fresh_context_with_vars(4);
            let t = materialize(&mut cx, &vars, &shape);
            prop_assert!(cx.unify(t, t).is_ok());
        }

        #[test]
        fn unify_is_symmetric(shape1 in any_shape(), shape2 in any_shape()) {
            let (mut cx_fwd, vars_fwd) = fresh_context_with_vars(4);
            let a = materialize(&mut cx_fwd, &vars_fwd, &shape1);
            let b = materialize(&mut cx_fwd, &vars_fwd, &shape2);
            let forward = cx_fwd.unify(a, b).is_ok();

            let (mut cx_bwd, vars_bwd) = fresh_context_with_vars(4);
            let a = materialize(&mut cx_bwd, &vars_bwd, &shape1);
            let b = materialize(&mut cx_bwd, &vars_bwd, &shape2);
            let backward = cx_bwd.unify(b, a).is_ok();

            prop_assert_eq!(forward, backward);
        }

        #[test]
        fn unifying_a_fresh_var_with_a_ground_term_binds_it(shape in ground_shape()) {
            let (mut cx, vars) = fresh_context_with_vars(0);
            let t = materialize(&mut cx, &vars, &shape);
            let v = cx.fresh_var();
            let v_term = cx.insert_term(Term::Var(v));

            prop_assert!(cx.unify(v_term, t).is_ok());
            let resolved_v = cx.resolve(v_term);
            let resolved_t = cx.resolve(t);
            prop_assert_eq!(resolved_v, resolved_t);
        }
    }
}
