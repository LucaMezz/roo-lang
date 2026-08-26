use ast::Span;
use slotmap::SlotMap;
use union_find::{QuickUnionUf, Union, UnionFind, UnionResult};

use crate::types::TyKind;

slotmap::new_key_type! {
    pub(crate) struct TyId;

    pub(crate) struct VarId;
}

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

struct ProvenanceEntry {
    var: VarId,
    reason: Span,
}

pub(crate) struct InferenceTable {
    terms: SlotMap<TyId, TyKind>,
    var_slots: SlotMap<VarId, usize>,
    uf_key_to_var: Vec<VarId>,
    bindings: QuickUnionUf<Binding>,
    provenance: Vec<ProvenanceEntry>,
}

impl InferenceTable {
    pub(crate) fn new() -> Self {
        Self {
            terms: SlotMap::with_key(),
            var_slots: SlotMap::with_key(),
            uf_key_to_var: Vec::new(),
            bindings: QuickUnionUf::new(0),
            provenance: Vec::new(),
        }
    }

    pub(crate) fn insert_term(&mut self, kind: TyKind) -> TyId {
        self.terms.insert(kind)
    }

    pub(crate) fn term(&self, id: TyId) -> Option<&TyKind> {
        self.terms.get(id)
    }

    pub(crate) fn fresh_var(&mut self) -> VarId {
        let uf_key = self.bindings.insert(Binding(None));
        let var = self.var_slots.insert(uf_key);
        debug_assert_eq!(uf_key, self.uf_key_to_var.len());
        self.uf_key_to_var.push(var);
        var
    }

    pub(crate) fn find(&mut self, var: VarId) -> VarId {
        let uf_key = self.var_slots[var];
        let root_key = self.bindings.find(uf_key);
        self.uf_key_to_var[root_key]
    }

    pub(crate) fn binding(&mut self, var: VarId) -> Option<TyId> {
        let uf_key = self.var_slots[var];
        self.bindings.get(uf_key).0
    }

    pub(crate) fn bind(&mut self, var: VarId, term: TyId) -> Option<TyId> {
        let uf_key = self.var_slots[var];
        std::mem::replace(self.bindings.get_mut(uf_key), Binding(Some(term))).0
    }

    pub(crate) fn union_vars(&mut self, a: VarId, b: VarId) -> bool {
        let ka = self.var_slots[a];
        let kb = self.var_slots[b];
        self.bindings.union(ka, kb)
    }

    pub(crate) fn resolve(&mut self, id: TyId) -> TyId {
        match self.term(id).expect("valid TyId") {
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

    fn occurs(&mut self, v: VarId, id: TyId) -> bool {
        let id = self.resolve(id);
        let kind = self.term(id).expect("valid TyId").clone();
        match kind {
            TyKind::Var(v2) => self.find(v) == self.find(v2),
            other => child_terms(&other)
                .into_iter()
                .any(|arg| self.occurs(v, arg)),
        }
    }

    pub(crate) fn unify(&mut self, t1: TyId, t2: TyId) -> Result<(), UnifyError> {
        self.unify_impl(t1, t2, None)
    }

    pub(crate) fn unify_because(
        &mut self,
        t1: TyId,
        t2: TyId,
        reason: Span,
    ) -> Result<(), UnifyError> {
        self.unify_impl(t1, t2, Some(reason))
    }

    fn unify_impl(
        &mut self,
        t1: TyId,
        t2: TyId,
        reason: Option<Span>,
    ) -> Result<(), UnifyError> {
        let resolved_t1 = self.resolve(t1);
        let resolved_t2 = self.resolve(t2);

        let kind1 = self.term(resolved_t1).expect("valid TyId").clone();
        let kind2 = self.term(resolved_t2).expect("valid TyId").clone();

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
                    (TyKind::Any, TyKind::Any)
                    | (TyKind::Never, TyKind::Never)
                    | (TyKind::Int, TyKind::Int)
                    | (TyKind::Float, TyKind::Float)
                    | (TyKind::Bool, TyKind::Bool)
                    | (TyKind::Char, TyKind::Char)
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
                    (TyKind::Struct(s1), TyKind::Struct(s2)) if s1 == s2 => Ok(()),
                    (TyKind::Enum(s1), TyKind::Enum(s2)) if s1 == s2 => Ok(()),
                    (TyKind::Generic(g1), TyKind::Generic(g2)) if g1 == g2 => Ok(()),
                    (k1, k2) => Err(UnifyError::ConstructorMismatch { t1, k1, t2, k2 }),
                }
            }
        }
    }

    fn record_provenance(&mut self, var: VarId, reason: Span) {
        self.provenance.push(ProvenanceEntry { var, reason });
    }

    pub(crate) fn provenance(&mut self, var: VarId) -> Option<Span> {
        let target = self.find(var);
        let mut found = None;
        for i in (0..self.provenance.len()).rev() {
            let entry_var = self.provenance[i].var;
            if self.find(entry_var) == target {
                found = Some(i);
                break;
            }
        }
        found.map(|i| self.provenance[i].reason)
    }
}

impl Default for InferenceTable {
    fn default() -> Self {
        Self::new()
    }
}

fn is_wildcard(kind: &TyKind) -> bool {
    matches!(kind, TyKind::Any | TyKind::Err | TyKind::Never)
}

fn child_terms(kind: &TyKind) -> Vec<TyId> {
    match kind {
        TyKind::Var(_)
        | TyKind::Any
        | TyKind::Never
        | TyKind::Int
        | TyKind::Float
        | TyKind::Bool
        | TyKind::Char
        | TyKind::Str
        | TyKind::Err
        | TyKind::Struct(_)
        | TyKind::Enum(_)
        | TyKind::Generic(_) => Vec::new(),
        TyKind::Array(elem) => vec![*elem],
        TyKind::Tuple(elems) => elems.clone(),
        TyKind::Fn(params, ret) => {
            let mut all = params.clone();
            all.push(*ret);
            all
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum UnifyError {
    #[error("constructor mismatch: {k1:?} != {k2:?}")]
    ConstructorMismatch {
        t1: TyId,
        k1: TyKind,
        t2: TyId,
        k2: TyKind,
    },
    #[error("arity mismatch: {arity1} args vs {arity2} args")]
    ArityMismatch {
        t1: TyId,
        arity1: usize,
        t2: TyId,
        arity2: usize,
    },
    #[error("occurs check failed for {0:?}")]
    OccursCheck(VarId),
}
