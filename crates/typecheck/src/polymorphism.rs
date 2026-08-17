use std::collections::{HashMap, HashSet};

use ast::{GenericArg, Path, Span};
use unify::{Term, TermId, VarId, term};

use crate::errors::GenericArgumentCountMismatch;
use crate::{GenericId, SymbolId, TyCon, TypeCheckContext};

impl TypeCheckContext {
    fn free_vars(&mut self, term: TermId, out: &mut Vec<VarId>) {
        let resolved = self.uni_cx.resolve(term);
        match self.uni_cx.term(resolved).cloned() {
            Some(Term::Var(v)) => {
                let root = self.uni_cx.find(v);
                if !out.contains(&root) {
                    out.push(root);
                }
            }
            Some(Term::App { args, .. }) => {
                for arg in args {
                    self.free_vars(arg, out);
                }
            }
            None => {}
        }
    }

    fn enclosing_free_vars(&mut self) -> HashSet<VarId> {
        let mut out = Vec::new();
        for i in 0..self.checking_stack.len() {
            let ty = self.symbols[self.checking_stack[i]].ty;
            self.free_vars(ty, &mut out);
        }
        out.into_iter().collect()
    }

    pub(crate) fn generalize_group(&mut self, members: &[SymbolId]) {
        self.generic_names.reset_synthetic_counter();

        let enclosing = self.enclosing_free_vars();

        let mut taken: HashSet<String> = HashSet::new();
        for &symbol in members {
            for &id in &self.symbols[symbol].generics {
                if let Some(name) = self.generic_names.get(&id) {
                    taken.insert(name.clone());
                }
            }
        }

        let mut per_member_vars: Vec<(SymbolId, Vec<VarId>)> = Vec::with_capacity(members.len());
        for &symbol in members {
            let ty = self.symbols[symbol].ty;
            let mut vars = Vec::new();
            self.free_vars(ty, &mut vars);
            vars.retain(|v| !enclosing.contains(v));
            per_member_vars.push((symbol, vars));
        }

        let mut assigned: HashMap<VarId, GenericId> = HashMap::new();
        for (_, vars) in &per_member_vars {
            for &var in vars {
                if let std::collections::hash_map::Entry::Vacant(entry) = assigned.entry(var) {
                    let id = self.generic_ids.insert(());
                    let name = self.generic_names.fresh_synthetic(&mut taken);
                    self.generic_names.declare(id, name);
                    let generic_term = term!(self.uni_cx, TyCon::Generic(id));
                    self.uni_cx.bind(var, generic_term);
                    entry.insert(id);
                }
            }
        }

        for (symbol, vars) in per_member_vars {
            for var in vars {
                let id = assigned[&var];
                self.symbols[symbol].generics.push(id);
            }
        }
    }

    fn instantiate(&mut self, symbol: SymbolId) -> TermId {
        self.instantiate_with(symbol, &[])
    }

    fn instantiate_with(&mut self, symbol: SymbolId, explicit: &[(TermId, Span)]) -> TermId {
        let ty = self.symbols[symbol].ty;
        if self.symbols[symbol].generics.is_empty() {
            return ty;
        }
        let generics = self.symbols[symbol].generics.clone();
        let mut subst = HashMap::new();
        for (&id, &(term, span)) in generics.iter().zip(explicit) {
            let var = self.uni_cx.fresh_var();
            let var_term = term!(self.uni_cx, var var);
            let _ = self.uni_cx.unify_because(var_term, term, span);
            subst.insert(id, var_term);
        }
        self.instantiate_term(ty, &mut subst)
    }

    fn instantiate_term(&mut self, term: TermId, subst: &mut HashMap<GenericId, TermId>) -> TermId {
        let resolved = self.uni_cx.resolve(term);
        match self.uni_cx.term(resolved).cloned() {
            Some(Term::Var(_)) => resolved,
            Some(Term::App {
                constructor: TyCon::Generic(id),
                ..
            }) => *subst.entry(id).or_insert_with(|| {
                let var = self.uni_cx.fresh_var();
                term!(self.uni_cx, var var)
            }),
            Some(Term::App { constructor, args }) => {
                let new_args = args
                    .iter()
                    .map(|&arg| self.instantiate_term(arg, subst))
                    .collect();
                term!(self.uni_cx, constructor => new_args)
            }
            None => resolved,
        }
    }

    pub(crate) fn instantiate_path(&mut self, symbol: SymbolId, path: &Path) -> TermId {
        match path.segments.last().and_then(|seg| seg.args.as_ref()) {
            Some(generic_args) => {
                let arg_tys: Vec<(TermId, Span)> = generic_args
                    .args
                    .iter()
                    .filter_map(|arg| match arg {
                        GenericArg::Arg(ty) => Some((self.lower_ty(ty), ty.span)),
                        GenericArg::Constraint(_) => None,
                    })
                    .collect();

                let max = self.symbols[symbol].generics.len();
                let actual = arg_tys.len();
                if actual != max {
                    self.diagnostics.push(GenericArgumentCountMismatch {
                        span: generic_args.span,
                        expected: max,
                        found: actual,
                    });
                }

                self.instantiate_with(symbol, &arg_tys[..actual.min(max)])
            }
            None => self.instantiate(symbol),
        }
    }
}
