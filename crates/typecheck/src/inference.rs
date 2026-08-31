//! Facilitates the creation and unification of tys
use ast::Span;
use slotmap::SlotMap;
use union_find::{QuickUnionUf, Union, UnionFind, UnionResult};

use crate::types::TyKind;

slotmap::new_key_type! {
    /// A handle to a ty
    pub(crate) struct TyId;

    /// A handle to an inference variable.
    pub(crate) struct VarId;
}

/// Represents the binding of an inference variable `?a`.
///
/// An inference variable `?a` can either be bound to Some
/// ty `t`, in which case the [`Binding`] contains the
/// id of the ty that it is bound to. Otherwise, if the
/// inference variable is free, then this binding will
/// contain `None`.
#[derive(Clone, Copy, Default)]
struct Binding(Option<TyId>);

impl Union for Binding {
    fn union(lval: Self, rval: Self) -> UnionResult<Self> {
        match lval.0 {
            Some(_) => UnionResult::Left(lval),
            None => UnionResult::Right(rval),
        }
    }
}

/// Documents a reason for why a variable was bound to
/// a specific ty.
struct ProvenanceEntry {
    /// The id of the inference variable that this entry
    /// corresponds to.
    var: VarId,

    /// The reason for why the variable was bound.
    reason: Span,
}

/// Stores everything needed to keep track of all inference
/// variables and tys in some context, and perform
/// unifications on them.
pub(crate) struct InferenceTable {
    /// A generational arena containing all of the tys in
    /// this context.
    tys: SlotMap<TyId, TyKind>,

    /// A generational arena containing all of the inference
    /// variables within this context.
    var_slots: SlotMap<VarId, usize>,

    /// A mapping from union_find key to variable id
    uf_key_to_var: Vec<VarId>,

    /// A UnionFind data structure containing all of the
    /// bindings of inference variables. Facilitates
    /// efficient unification.
    bindings: QuickUnionUf<Binding>,

    /// Stores all provinence entries recorded.
    provenance: Vec<ProvenanceEntry>,
}

impl InferenceTable {
    /// Creates a new empty [`InferenceTable`].
    pub(crate) fn new() -> Self {
        Self {
            tys: SlotMap::with_key(),
            var_slots: SlotMap::with_key(),
            uf_key_to_var: Vec::new(),
            bindings: QuickUnionUf::new(0),
            provenance: Vec::new(),
        }
    }

    /// Inserts a new ty into this [`InferenceTable`]
    pub(crate) fn insert_ty(&mut self, kind: TyKind) -> TyId {
        self.tys.insert(kind)
    }

    /// Gets a reference to the ty with the given id,
    /// if it exists.
    pub(crate) fn ty(&self, id: TyId) -> Option<&TyKind> {
        self.tys.get(id)
    }

    /// Creates a new free inference variable, and returns
    /// its id.
    pub(crate) fn fresh_var(&mut self) -> VarId {
        let uf_key = self.bindings.insert(Binding(None));
        let var = self.var_slots.insert(uf_key);
        debug_assert_eq!(uf_key, self.uf_key_to_var.len());
        self.uf_key_to_var.push(var);
        var
    }

    /// Gets the id of the representative of the equivalence
    /// class of the variable with the given id.
    ///
    /// If many inference variables are bound to the same
    /// ty, this method will consistently return the
    /// same inference variable among that group, for a
    /// certain state.
    ///
    /// This corresponds to the representative / root node
    /// within the underlying union-find data structure.
    pub(crate) fn find(&mut self, var: VarId) -> VarId {
        let uf_key = self.var_slots[var];
        let root_key = self.bindings.find(uf_key);
        self.uf_key_to_var[root_key]
    }

    /// Returns the id of the ty that a given inference
    /// variable is bound to, if it is not unbound.
    pub(crate) fn binding(&mut self, var: VarId) -> Option<TyId> {
        let uf_key = self.var_slots[var];
        self.bindings.get(uf_key).0
    }

    /// Binds an inference variable to a ty. Gives back
    /// the ty that it was bound to.
    pub(crate) fn bind(&mut self, var: VarId, ty: TyId) -> Option<TyId> {
        let uf_key = self.var_slots[var];
        std::mem::replace(self.bindings.get_mut(uf_key), Binding(Some(ty))).0
    }

    /// Unions two variables.
    pub(crate) fn union_vars(&mut self, a: VarId, b: VarId) -> bool {
        let ka = self.var_slots[a];
        let kb = self.var_slots[b];
        self.bindings.union(ka, kb)
    }

    /// Resolves a ty. Recursively resolve tys which are
    /// just inference variables with no constructor, until
    /// it arrives at the ty which is actually represented
    /// by the given ty.
    pub(crate) fn resolve(&mut self, id: TyId) -> TyId {
        match self.ty(id).expect("valid TyId") {
            TyKind::Var(v) => {
                let v = *v;
                match self.binding(v) {
                    Some(bound_id) => self.resolve(bound_id),
                    None => id,
                }
            }
            _ => id,
        }
    }

    /// Performs the 'occurs check' for the inference
    /// variable `v` in the given ty. Recursively checks if
    /// the given ty contains the inference variable `v`
    /// anywhere within it.
    fn occurs(&mut self, v: VarId, id: TyId) -> bool {
        let id = self.resolve(id);
        let kind = self.ty(id).expect("valid TyId").clone();
        match kind {
            TyKind::Var(v2) => self.find(v) == self.find(v2),
            other => child_tys(other).into_iter().any(|arg| self.occurs(v, arg)),
        }
    }

    /// Unify two tys.
    ///
    /// This forces the two tys to be equal to one another.
    /// If this is not possible, or if doing so would require
    /// an infinite cyclic type, then an error is raised.
    ///
    /// Unification of two tys `t_1` and `t_2` essentially
    /// produces a set of substitions `S` which replace
    /// inference variables within the tys `t_1` and `t_2`
    /// with tys, such that applying all substitutions
    /// results in both tys being equal.
    ///
    /// If S contains the substitution `?a |-> t`, then the
    /// inference variable `?a` will be bound to the ty
    /// `t`.
    pub(crate) fn unify(&mut self, t1: TyId, t2: TyId) -> Result<(), UnifyError> {
        self.unify_impl(t1, t2, None)
    }

    /// Performs unification given a reason.
    ///
    /// See [`Self::unify`] for more information about the
    /// unify operation.
    pub(crate) fn unify_because(
        &mut self,
        t1: TyId,
        t2: TyId,
        reason: Span,
    ) -> Result<(), UnifyError> {
        self.unify_impl(t1, t2, Some(reason))
    }

    fn unify_impl(&mut self, t1: TyId, t2: TyId, reason: Option<Span>) -> Result<(), UnifyError> {
        let resolved_t1 = self.resolve(t1);
        let resolved_t2 = self.resolve(t2);

        let kind1 = self.ty(resolved_t1).expect("valid TyId").clone();
        let kind2 = self.ty(resolved_t2).expect("valid TyId").clone();

        match (kind1, kind2) {
            (TyKind::Var(v1), TyKind::Var(v2)) => {
                if self.find(v1) != self.find(v2) {
                    self.union_vars(v1, v2);
                    if let Some(reason) = reason {
                        self.record_provenance(v1, reason);
                    }
                }
                Ok(())
            }
            (TyKind::Var(v), _) => {
                if self.occurs(v, resolved_t2) {
                    return Err(UnifyError::OccursCheck(v));
                }
                self.bind(v, resolved_t2);
                if let Some(reason) = reason {
                    self.record_provenance(v, reason);
                }
                Ok(())
            }
            (_, TyKind::Var(v)) => {
                if self.occurs(v, resolved_t1) {
                    return Err(UnifyError::OccursCheck(v));
                }
                self.bind(v, resolved_t1);
                if let Some(reason) = reason {
                    self.record_provenance(v, reason);
                }
                Ok(())
            }
            (k1, k2) => {
                if is_wildcard(&k1) || is_wildcard(&k2) {
                    return Ok(());
                }
                match (k1, k2) {
                    (TyKind::Never, TyKind::Never)
                    | (TyKind::Int, TyKind::Int)
                    | (TyKind::Float, TyKind::Float)
                    | (TyKind::Bool, TyKind::Bool)
                    | (TyKind::Str, TyKind::Str)
                    | (TyKind::Err, TyKind::Err) => Ok(()),
                    (TyKind::Array(a), TyKind::Array(b)) => self.unify_impl(a, b, reason),
                    (TyKind::Tuple(a), TyKind::Tuple(b)) => {
                        if a.len() != b.len() {
                            return Err(UnifyError::ArityMismatch {
                                t1,
                                arity1: a.len(),
                                t2,
                                arity2: b.len(),
                            });
                        }
                        for (x, y) in a.into_iter().zip(b) {
                            self.unify_impl(x, y, reason)?;
                        }
                        Ok(())
                    }
                    (TyKind::Fn(p1, r1), TyKind::Fn(p2, r2)) => {
                        if p1.len() != p2.len() {
                            return Err(UnifyError::ArityMismatch {
                                t1,
                                arity1: p1.len(),
                                t2,
                                arity2: p2.len(),
                            });
                        }
                        for (x, y) in p1.into_iter().zip(p2) {
                            self.unify_impl(x, y, reason)?;
                        }
                        self.unify_impl(r1, r2, reason)
                    }
                    (TyKind::Struct(s1, a1), TyKind::Struct(s2, a2)) if s1 == s2 => {
                        for (x, y) in a1.into_iter().zip(a2) {
                            self.unify_impl(x, y, reason)?;
                        }
                        Ok(())
                    }
                    (TyKind::Enum(s1, a1), TyKind::Enum(s2, a2)) if s1 == s2 => {
                        for (x, y) in a1.into_iter().zip(a2) {
                            self.unify_impl(x, y, reason)?;
                        }
                        Ok(())
                    }
                    (TyKind::TraitObject(s1, a1), TyKind::TraitObject(s2, a2)) if s1 == s2 => {
                        for (x, y) in a1.into_iter().zip(a2) {
                            self.unify_impl(x, y, reason)?;
                        }
                        Ok(())
                    }
                    (TyKind::Generic(g1), TyKind::Generic(g2)) if g1 == g2 => Ok(()),
                    (k1, k2) => Err(UnifyError::ConstructorMismatch { t1, k1, t2, k2 }),
                }
            }
        }
    }

    fn record_provenance(&mut self, var: VarId, reason: Span) {
        self.provenance.push(ProvenanceEntry { var, reason });
    }

    /// Retrieves the most recent recorded provenance related
    /// to the type variable provided.
    pub(crate) fn provenance(&mut self, var: VarId) -> Option<Span> {
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
        found.map(|i| self.provenance[i].reason)
    }

    pub(crate) fn snapshot(&self) -> Snapshot {
        Snapshot {
            bindings: self.bindings.clone(),
            provenance_len: self.provenance.len(),
            uf_key_to_var_length: self.uf_key_to_var.len(),
        }
    }

    pub(crate) fn rollback_to(&mut self, snapshot: Snapshot) {
        self.bindings = snapshot.bindings;
        self.provenance.truncate(snapshot.provenance_len);
        self.uf_key_to_var.truncate(snapshot.uf_key_to_var_length);
    }
}

pub(crate) struct Snapshot {
    bindings: QuickUnionUf<Binding>,
    provenance_len: usize,
    uf_key_to_var_length: usize,
}

impl Default for InferenceTable {
    fn default() -> Self {
        Self::new()
    }
}

fn is_wildcard(kind: &TyKind) -> bool {
    matches!(kind, TyKind::Err | TyKind::Never)
}

pub(crate) fn child_tys(kind: TyKind) -> Vec<TyId> {
    match kind {
        TyKind::Var(_)
        | TyKind::Never
        | TyKind::Int
        | TyKind::Float
        | TyKind::Bool
        | TyKind::Str
        | TyKind::Err
        | TyKind::Generic(_) => Vec::new(),
        TyKind::Array(elem) => vec![elem],
        TyKind::Tuple(elems) => elems,
        TyKind::Struct(_, args) | TyKind::Enum(_, args) | TyKind::TraitObject(_, args) => args,
        TyKind::Fn(mut params, ret) => {
            params.push(ret);
            params
        }
    }
}

pub(crate) fn map_children(kind: TyKind, mut f: impl FnMut(TyId) -> TyId) -> TyKind {
    match kind {
        TyKind::Array(elem) => TyKind::Array(f(elem)),
        TyKind::Tuple(args) => TyKind::Tuple(args.into_iter().map(&mut f).collect()),
        TyKind::Struct(def, args) => TyKind::Struct(def, args.into_iter().map(&mut f).collect()),
        TyKind::Enum(def, args) => TyKind::Enum(def, args.into_iter().map(&mut f).collect()),
        TyKind::Fn(params, ret) => {
            let params = params.into_iter().map(&mut f).collect();
            let ret = f(ret);
            TyKind::Fn(params, ret)
        }
        other => other,
    }
}

/// An error that is produced when the unification of two
/// tys fails. This means there is no set `S` of
/// substitutions of inference variables with tys that
/// can be made in order to make two tys `t_1` and `t_2`
/// equal.
#[derive(Debug, thiserror::Error)]
pub(crate) enum UnifyError {
    /// Somewhere at corresponding points within the two
    /// tys `t_1` and `t_2`, there are two constructors
    /// which differ. e.g.
    ///
    /// ```text
    /// ...f(...)... != ...g(...)...
    /// ```
    ///
    #[error("constructor mismatch: {k1:?} != {k2:?}")]
    ConstructorMismatch {
        /// Ty ID of the first ty
        t1: TyId,

        /// Kind being applied in the first ty
        k1: TyKind,

        /// Ty ID of the second ty
        t2: TyId,

        /// Kind being applied in the second ty
        k2: TyKind,
    },
    /// Somewhere at corresponding points within the two
    /// tys `t_1` and `t_2`, there are two constructors
    /// which have a differing number of arguments. e.g.
    ///
    /// ```text
    /// ...f(t_1, t_2, ..., t_n)... != ...g(t_1, t2, ..., t_m)...
    /// ```
    //
    /// where n != m
    ///
    #[error("arity mismatch: {arity1} args vs {arity2} args")]
    ArityMismatch {
        /// Ty ID of the first ty
        t1: TyId,

        /// The arity of the first ty
        arity1: usize,

        /// Ty ID of the second ty
        t2: TyId,

        /// The arity of the second ty
        arity2: usize,
    },
    /// At some point within the ty, if there is just
    /// an inference variable `?a` on one side and an
    /// application of a constructor on the other, then
    /// if anywhere in the arguments of that application,
    /// the same inference variable `?a` appears, then
    /// only an infinite cyclical type can work, which
    /// is not possible in practice. e.g.
    ///
    /// ```text
    /// ...f(t_1, ..., ?a, ..., t_n)... != ...?a...
    /// ```
    ///
    #[error("occurs check failed for {0:?}")]
    OccursCheck(VarId),
}
