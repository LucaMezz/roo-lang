//! `unify` is a general-purpose, reusable unification engine for solving
//! systems of type equations — not tied to fig-lang's own type system, so
//! it can be pulled in by any crate that needs to solve a set of
//! constraints of the form `t1 ≟ t2` ("does term `t1` unify with term
//! `t2`?") down to a set of variable bindings, or an occurs-check/mismatch
//! error.
//!
//! Notation used throughout these docs: an inference variable is written
//! `?a`, `?b`, `?T`, ...; `?a ↦ t` means "`?a` is currently bound to
//! `t`" (an unbound variable has no `↦` at all); `f(t1, ..., tn)` is an
//! application of constructor `f` to argument terms `t1, ..., tn`.
//!
//! Shape:
//! - Terms and variables live in a [`slotmap`] arena and are referred to
//!   by id ([`TermId`]/[`VarId`]), rather than a directly recursive
//!   `enum Term { ... }` of owned/boxed references — keeps terms `Copy`,
//!   sidesteps the recursive ownership/lifetime issues a self-referential
//!   term graph would otherwise have, and matches how egraph/union-find-
//!   based unifiers are usually built.
//! - Variable-to-term binding (`?a ↦ t`) and the union-find "which
//!   variables are already known to be equal" structure,
//!   [`UnificationContext`], are backed by the [`union_find`] crate.
//! - [`UnificationContext::unify`] is the actual algorithm: given two
//!   term ids, it either records enough bindings/merges to make `t1 ≟
//!   t2` hold and returns `Ok`, or returns a [`UnifyError`] explaining
//!   why they can't be made equal.
//! - `proptest` is a dev-dependency for property-based testing of
//!   `unify` (e.g. "unifying a term with itself always succeeds",
//!   "unification is symmetric").

use std::fmt;
use std::mem;

use slotmap::SlotMap;
use union_find::{QuickUnionUf, Union, UnionFind, UnionResult};

slotmap::new_key_type! {
    /// Id of a [`Term`] interned in a unifier's arena. Opaque and
    /// generation-tagged (via `slotmap`) — the only way to get one is
    /// from the arena you inserted into, so a stale id from a removed
    /// term can never silently alias whatever gets inserted in its place.
    pub struct TermId;

    /// Id of an inference variable (written `?a`, `?b`, ... in these
    /// docs) tracked by a unifier.
    pub struct VarId;
}

/// A term `t`: either an inference variable `?a`, or an application
/// `f(t1, ..., tn)` of a constructor `f` to some argument terms.
/// Arguments are referenced by [`TermId`] rather than owned directly, so
/// `Term` stays flat/`Copy`-friendly instead of being a directly
/// recursive, boxed tree.
#[derive(Debug, Clone, PartialEq)]
pub enum Term<C> {
    /// An inference variable `?a`. This node itself never changes once
    /// inserted — whether/what `?a` is currently bound to lives
    /// separately, in the owning [`UnificationContext`], and has to be
    /// looked up via [`UnificationContext::resolve`] or
    /// [`UnificationContext::binding`] rather than read off this variant
    /// directly.
    Var(VarId),
    /// An application `f(t1, ..., tn)` of constructor `f` to some
    /// argument terms.
    App {
        /// The constructor `f` being applied.
        constructor: C,
        /// The argument terms `t1, ..., tn`, by id.
        args: Vec<TermId>,
    },
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

/// A variable class's current binding, if any — `union_find::Union` is a
/// foreign trait and `Option` isn't a local or fundamental type, so the
/// orphan rules require this local newtype rather than `impl Union for
/// Option<TermId>` directly.
#[derive(Clone, Copy, Default)]
struct Binding(Option<TermId>);

/// How two variable classes' bindings are combined when
/// [`UnificationContext::union_vars`] merges them. Needs *some*
/// deterministic answer to satisfy `union_find::Union` (which can't fail
/// or see the rest of the arena), so it arbitrarily prefers whichever
/// side is already bound.
///
/// This is safe to be this naive because [`UnificationContext::unify`]
/// never lets two *different* bindings reach this code: it always
/// [`resolve`](UnificationContext::resolve)s both sides of a `t1 ≟ t2`
/// unification first, so by the time its `(?a, ?b)` case calls
/// `union_vars`, both `?a` and `?b` have already been established to be
/// currently unbound — this merge only ever really has to combine
/// `(None, None)`, and which side "wins" doesn't matter. Calling
/// `union_vars` directly, bypassing `unify`, forgoes that guarantee.
impl Union for Binding {
    fn union(lval: Self, rval: Self) -> UnionResult<Self> {
        match lval.0 {
            Some(_) => UnionResult::Left(lval),
            None => UnionResult::Right(rval),
        }
    }
}

/// Owns every [`Term`] and variable created during a unification session.
///
/// Terms live in a `slotmap` arena (`terms`). Variables are tracked two
/// ways at once: a `slotmap` arena (`var_slots`) gives out the opaque,
/// generation-tagged [`VarId`]s callers hold, while a parallel
/// `union_find::QuickUnionUf` (`bindings`) tracks which variables have
/// been unified with each other and what (if anything) their class is
/// currently bound to. `union_find`'s own keys are plain sequential
/// integers with no notion of generations, so a `VarId` can't be
/// *derived* from one — `var_slots`/`uf_key_to_var` keep an explicit
/// mapping in both directions, populated in lockstep every time
/// [`fresh_var`](Self::fresh_var) allocates one of each.
pub struct UnificationContext<C> {
    terms: SlotMap<TermId, Term<C>>,
    var_slots: SlotMap<VarId, usize>,
    uf_key_to_var: Vec<VarId>,
    bindings: QuickUnionUf<Binding>,
    wildcards: Vec<C>,
}

impl<C> UnificationContext<C> {
    /// Creates an empty context with no terms or variables yet.
    pub fn new() -> Self {
        Self {
            terms: SlotMap::with_key(),
            var_slots: SlotMap::with_key(),
            uf_key_to_var: Vec::new(),
            bindings: QuickUnionUf::new(0),
            wildcards: Vec::new(),
        }
    }

    /// Creates an empty context with no terms or variables yet, but
    /// with `wildcard` registered as a constructor that always unifies
    /// successfully against anything (see [`unify`](Self::unify)) — for
    /// a caller whose `C` has some "matches anything" constructor of its
    /// own (e.g. a gradually-typed language's `any`), not a concept
    /// `unify` has any built-in notion of otherwise.
    pub fn with_wildcard(wildcard: C) -> Self {
        Self::with_wildcards(vec![wildcard])
    }

    /// Same as [`with_wildcard`](Self::with_wildcard), but for a caller
    /// that needs more than one absorbing constructor at once — e.g. a
    /// gradually-typed language's `any` *and* a type checker's own
    /// error-recovery placeholder, each wildcard for its own unrelated
    /// reason. `unify` doesn't care why a constructor is a wildcard, or
    /// how many there are, only whether a given one is in this list.
    pub fn with_wildcards(wildcards: Vec<C>) -> Self {
        Self {
            terms: SlotMap::with_key(),
            var_slots: SlotMap::with_key(),
            uf_key_to_var: Vec::new(),
            bindings: QuickUnionUf::new(0),
            wildcards,
        }
    }

    /// Allocates a new term in the arena and returns its id.
    ///
    /// No hash-consing/deduplication — every call allocates a fresh
    /// [`TermId`], even for a `term` equal to one already stored.
    pub fn insert_term(&mut self, term: Term<C>) -> TermId {
        self.terms.insert(term)
    }

    /// Looks up a previously inserted term. `None` if `id` doesn't refer
    /// to a live term in this context (e.g. it's from a different
    /// [`UnificationContext`]).
    pub fn term(&self, id: TermId) -> Option<&Term<C>> {
        self.terms.get(id)
    }

    /// Creates a fresh, unbound type variable.
    pub fn fresh_var(&mut self) -> VarId {
        let uf_key = self.bindings.insert(Binding(None));
        let var = self.var_slots.insert(uf_key);
        debug_assert_eq!(uf_key, self.uf_key_to_var.len());
        self.uf_key_to_var.push(var);
        var
    }

    /// Returns the canonical representative of `?a`'s union-find class
    /// — two variables that have been [`union_vars`](Self::union_vars)'d
    /// together, directly or transitively, always find to the same
    /// representative.
    pub fn find(&mut self, var: VarId) -> VarId {
        let uf_key = self.var_slots[var];
        let root_key = self.bindings.find(uf_key);
        self.uf_key_to_var[root_key]
    }

    /// The term `?a` is currently bound to (`?a ↦ t`), if any.
    pub fn binding(&mut self, var: VarId) -> Option<TermId> {
        let uf_key = self.var_slots[var];
        self.bindings.get(uf_key).0
    }

    /// Binds `?a`'s class directly to `t` (records `?a ↦ t`), returning
    /// whatever it was previously bound to (if anything). Overwrites
    /// unconditionally — doesn't check whether `?a` was already bound to
    /// something else, let alone unify the two.
    /// [`unify`](Self::unify) never runs into that, since it only ever
    /// calls `bind` after [`resolve`](Self::resolve) has established
    /// that `?a` is currently unbound; calling `bind` directly bypasses
    /// that safeguard.
    pub fn bind(&mut self, var: VarId, term: TermId) -> Option<TermId> {
        let uf_key = self.var_slots[var];
        mem::replace(self.bindings.get_mut(uf_key), Binding(Some(term))).0
    }

    /// Merges `?a` and `?b`'s union-find classes, so that
    /// `find(?a) == find(?b)` afterward. Returns `true` if they belonged
    /// to different classes (and were therefore actually merged),
    /// matching [`union_find::UnionFind::union`]'s own return value.
    ///
    /// See `Union for Binding`'s doc comment above for what happens to
    /// each side's existing binding.
    pub fn union_vars(&mut self, a: VarId, b: VarId) -> bool {
        let ka = self.var_slots[a];
        let kb = self.var_slots[b];
        self.bindings.union(ka, kb)
    }

    /// Follows `id` through any existing variable binding to find out
    /// what it *currently* denotes.
    ///
    /// A [`Term::Var`] arena node never changes once inserted, so
    /// without this, code looking at a term is not paying attention to
    /// the existing bindings of the inference variables that appear in
    /// it — it only ever sees a variable's original, unbound shape, even
    /// long after that variable has been bound to something else.
    /// `resolve` fixes that: it "replaces" an inference variable with
    /// whatever term it's currently bound to (looked up on demand here,
    /// not physically rewritten anywhere), following the chain if that
    /// term is itself an already-bound variable, until it reaches either
    /// an unbound variable or a concrete application.
    ///
    /// Formally, for a term `t`:
    /// - if `t = ?a` and `?a ↦ u`, then `resolve(t) = resolve(u)`
    /// - if `t = ?a` and `?a` is unbound, `resolve(t) = t`
    /// - if `t = f(t1, ..., tn)`, `resolve(t) = t` (already concrete)
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

    /// Does `?a` occur, directly or transitively through `App`
    /// arguments, within the term that `id` currently denotes?
    ///
    /// Resolves as it goes, so a variable buried behind other,
    /// already-bound variables is still found — this is what stops
    /// [`unify`](Self::unify) from ever producing a cyclic binding like
    /// `?a ↦ List(?a)`.
    ///
    /// Formally, `occurs(?a, t)` holds iff `?a` occurs free in
    /// `resolve(t)`:
    /// - `occurs(?a, ?b)` iff `find(?a) == find(?b)` (comparing
    ///   union-find classes, not raw ids, so a variable already unioned
    ///   with `?a` under a different id still counts)
    /// - `occurs(?a, f(t1, ..., tn))` iff `occurs(?a, ti)` for some `ti`
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

    /// Unifies `t1` and `t2` — checks whether `t1 ≟ t2` holds and, if
    /// so, records whatever bindings/merges make it hold, directly into
    /// this context.
    ///
    /// Always [`resolve`](Self::resolve)s both sides first, so this
    /// compares what each term *currently* means rather than the shape
    /// it had when it was first inserted — see `resolve`'s doc comment
    /// for why that distinction matters.
    ///
    /// If this context has any [`wildcard`](Self::with_wildcard)
    /// constructors registered, an application whose constructor is one
    /// of them always unifies successfully against anything — no
    /// constructor/arity check, no descending into either side's
    /// arguments. See `with_wildcard`'s doc comment for what that's for.
    pub fn unify(&mut self, t1: TermId, t2: TermId) -> Result<(), UnifyError<C>>
    where
        C: Clone + fmt::Debug + PartialEq,
    {
        let t1 = self.resolve(t1);
        let t2 = self.resolve(t2);

        match (
            self.term(t1).expect("valid TermId"),
            self.term(t2).expect("valid TermId"),
        ) {
            (Term::Var(v1), Term::Var(v2)) => {
                let (v1, v2) = (*v1, *v2);
                if self.find(v1) != self.find(v2) {
                    self.union_vars(v1, v2);
                }
                Ok(())
            }
            (Term::Var(v), _) => {
                let v = *v;
                if self.occurs(v, t2) {
                    return Err(UnifyError::OccursCheck(v));
                }
                self.bind(v, t2);
                Ok(())
            }
            (_, Term::Var(v)) => {
                let v = *v;
                if self.occurs(v, t1) {
                    return Err(UnifyError::OccursCheck(v));
                }
                self.bind(v, t1);
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
                    return Err(UnifyError::ConstructorMismatch(c1.clone(), c2.clone()));
                }
                if args1.len() != args2.len() {
                    return Err(UnifyError::ArityMismatch(args1.len(), args2.len()));
                }
                let pairs: Vec<(TermId, TermId)> =
                    args1.iter().copied().zip(args2.iter().copied()).collect();
                for (x, y) in pairs {
                    self.unify(x, y)?;
                }
                Ok(())
            }
        }
    }
}

impl<C> Default for UnificationContext<C> {
    fn default() -> Self {
        Self::new()
    }
}

/// Errors [`UnificationContext::unify`] can fail with when `t1 ≟ t2`
/// doesn't hold.
#[derive(Debug, thiserror::Error)]
pub enum UnifyError<C> {
    /// Two applications `f(...)`/`g(...)` had different constructors
    /// (`f != g`).
    #[error("constructor mismatch: {0:?} != {1:?}")]
    ConstructorMismatch(C, C),
    /// Two applications of the *same* constructor had a different
    /// number of arguments.
    #[error("arity mismatch: {0} args vs {1} args")]
    ArityMismatch(usize, usize),
    /// Binding `?a` here would create a cycle — `?a` occurs, directly or
    /// transitively, within the term it would be bound to (e.g. trying
    /// to bind `?a ↦ List(?a)`).
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
        // Already-equal vars: nothing left to merge.
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
        // ?v ↦ Int after the first call. Unifying ?v against Bool means
        // checking Int ≟ Bool, which doesn't hold -- `resolve` is what
        // makes `unify` see that, instead of treating ?v as still free
        // and silently rebinding it.
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
        // Different constructor *and* different arity -- the wildcard
        // check has to fire before either the constructor or the arity
        // comparison, or this would fail on one of those instead.
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
        // A nested wildcard argument must not force a real mismatch just
        // because the *outer* constructors and arities happen to match --
        // the wildcard check has to fire at every level of the recursive
        // structural comparison, not just at the top.
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
        // "any" is just an ordinary constructor value here -- nothing
        // about the string itself is special. Only a constructor
        // explicitly registered via `with_wildcard` gets short-circuited.
        assert!(cx.unify(a, b).is_err());
    }

    #[test]
    fn resolve_follows_a_chain_of_bound_variables() {
        // Manually construct ?a ↦ ?b ↦ Int. `unify` itself never creates
        // a variable-to-variable binding like this -- ?a/?b pairs go
        // through union_vars instead -- but `resolve` has to handle it
        // correctly regardless, since `bind` doesn't forbid it.
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

    // -- property-based tests --------------------------------------------

    use proptest::prelude::*;

    /// A small tree shape to build terms from. `Var(i)` names one of a
    /// handful of shared variables (see `materialize`) rather than
    /// creating a fresh one every time, so the same variable can show up
    /// more than once within (or across) generated shapes.
    #[derive(Debug, Clone)]
    enum Shape {
        Var(u8),
        Int,
        Bool,
        List(Box<Shape>),
    }

    /// Shapes that may contain variables.
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

    /// Shapes with no variables at all.
    fn ground_shape() -> impl Strategy<Value = Shape> {
        let leaf = prop_oneof![Just(Shape::Int), Just(Shape::Bool)];
        leaf.prop_recursive(4, 16, 2, |inner| {
            inner.prop_map(|s| Shape::List(Box::new(s)))
        })
    }

    /// Builds `shape` into `cx`, resolving `Shape::Var(i)` to `vars[i]`
    /// each time -- so `vars` should be the same slice across every
    /// `materialize` call in a given test, to keep "variable 0" meaning
    /// the same variable throughout. Only `any_shape()` ever produces a
    /// `Var`, and it's only ever paired with a non-empty `vars`.
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

    /// A fresh context pre-populated with `n` fresh variables (as both
    /// `VarId`s and the `TermId`s of their `Term::Var` wrappers, for
    /// `materialize` to hand out).
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
            // Two separately built, identically-seeded contexts, so
            // running the forward direction can't leave state behind
            // that would bias the backward check.
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
