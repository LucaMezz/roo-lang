//! Contains all methods on the [`TypeCheckContext`] related to the
//! implementation of polymorphism via generic type parameters.
//! Specifically, these methods facilitate the generalisation of
//! free inference variables into generic type parameters for nested
//! functions, as well as the instantiation of terms which contain
//! generics by substituting them for concrete terms, or otherwise
//! substituting them for fresh inference variables.

use std::collections::{HashMap, HashSet};

use ast::{GenericArg, Path, Span};
use unify::{Term, TermId, VarId, term};

use crate::errors::GenericArgumentCountMismatch;
use crate::{GenericId, SymbolId, TyCon, TypeCheckContext};

impl TypeCheckContext {
    /// Returns all of the free inference variables which appear
    /// within the given term.
    ///
    /// A type inference variable is considered `free` when it has not
    /// yet been bound to any term. More precisely, given a term `t`,
    /// `free_vars(t)` returns the set of inference variables `?a` that
    /// occur in `t` and are not currently bound by the unification
    /// context.
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

    /// Return all free inference variables `?a` which are still
    /// free in all enclosing function bodies
    ///
    /// For each new function on the checking stack, it indicates
    /// we are one function deeper in a chain of nested functions.
    ///
    /// More precisely, it gives the union of, for each enclosing
    /// function `f` the set of all free variables which
    ///
    /// Helps to determine when an inference variable in a function
    /// `f` is truly free and can be generalised, or if its type is
    /// already b
    fn enclosing_free_vars(&mut self) -> HashSet<VarId> {
        let mut out = Vec::new();
        for i in 0..self.checking_stack.len() {
            let ty = self.symbols[self.checking_stack[i]].ty;
            self.free_vars(ty, &mut out);
        }
        out.into_iter().collect()
    }

    /// Used to infer generic type parameters on functions where they
    /// have not explicitly been specified, specifically where the
    /// functions are nested. For example, the function
    /// ```ignore
    /// fn outer(x) {
    ///     fn inner(y) {
    ///         y
    ///     }
    ///     inner(x)
    /// }
    /// ```
    /// will first check the nested `inner` function before even binding
    /// the type of `x` in the scope of the body of `outer`, which is
    /// before `x` even gets bound to a fresh inference variable.
    /// In that case `y` gets bound to a fresh inference variable `?a`.
    /// Since the inference variable `?a` does not appear in the
    /// surrounding function at all, inner generalises it and introduces
    /// a new generic type parameter `T`. Then the body of `outer`
    /// gets checked, given the fact that `inner` is now generic.
    pub(crate) fn generalize_group(&mut self, members: &[SymbolId]) {
        // Restart synthetic names for generic type parameters back to `T`.
        self.generic_names.reset_synthetic_counter();

        let enclosing = self.enclosing_free_vars();

        // Gather all explicit generic type parameters from the signature
        // of all of the functions within this group. Used to avoid
        // any synthesised names which may be produced from having the same
        // name as existing generic type parameters.
        //
        // FIXME Ensure that generic type parameter names being taken in
        // one function, do not prevent the names from being used in
        // another function in the group. This is likely the cause of an
        // issue with LSP of mutually recursive functions where only one
        // explicitly specifies its generic type parameter.
        let mut taken: HashSet<String> = HashSet::new();
        for &symbol in members {
            for &id in &self.symbols[symbol].generics {
                if let Some(name) = self.generic_names.get(&id) {
                    taken.insert(name.clone());
                }
            }
        }

        // Begin the process of generalisation of nested functions for
        // each individual function in the group with members provided.
        //
        // Collect each of the free variables `?a` of each function in the
        // group, such that `?a` is not free in any *enclosing function*.
        // Here we mean either `?a` does not exist at all in the closing
        // function context.
        //
        // These become candidates for generalisation.
        let mut per_member_vars: Vec<(SymbolId, Vec<VarId>)> = Vec::with_capacity(members.len());
        for &symbol in members {
            let ty = self.symbols[symbol].ty;
            let mut vars = Vec::new();
            self.free_vars(ty, &mut vars);
            vars.retain(|v| !enclosing.contains(v));
            per_member_vars.push((symbol, vars));
        }

        // For each unique variable collected, generalise it to a new
        // new unique generic type parameter.
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

        // For each function, append all new generics synthesised from
        // generalisation to its symbol.
        for (symbol, vars) in per_member_vars {
            for var in vars {
                let id = assigned[&var];
                self.symbols[symbol].generics.push(id);
            }
        }
    }

    /// Replace all generic type parameters of the type of a symbol with
    /// fresh inference variables.
    ///
    /// See [`Self::instantiate_term`] for more information. This simply
    /// calls that but with an empty set of substitutions.
    fn instantiate(&mut self, symbol: SymbolId) -> TermId {
        self.instantiate_with(symbol, &[])
    }

    /// Instantiates the given symbol's type for use at a single call site.
    /// Any `explitit` generic type arguments provided via turbofish first
    /// constrain the fresh inference variable introduced for that generic
    /// parameter. instantiate_term will fill in the rest with fresh
    /// inference variables.
    ///
    /// See [`Self::instantiate_term`] for more information.
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

    /// Instantiates a new term by substituting all generic type parameters
    /// which appear in the term with the terms they represent, according
    /// to the substitution map provided. If there is no substitution specified
    /// for a given generic type parameter, then substitute it with a fresh
    /// inference variable.
    ///
    /// For example, given `t` as the term `(T, U)`, and the substitution map
    /// given by `T |-> String` and `U -> int`, it would produce a new term
    /// given by applying the substitutions to `t`, which would be (String, int).
    /// However, if we were only given the first substitution, `T |-> String`,
    /// then `U` would instead be substituted with some new inference variable
    /// `?a`.
    ///
    /// This is used when generic type parameters of a symbol need to be
    /// replaced by concrete types. For example, consider a function with
    /// the signature
    /// ```ignore
    /// fn add<T>(a: T, b: T) -> T
    /// ```
    /// This function has type `Fn<T>(T, T) -> T`. When calling this function,
    /// the generic type parameter `T` must be replaced by some concrete type.
    /// For example
    /// ```ignore
    /// let _ = add::<int>(first, second);
    /// ```
    /// Needs to instantiate the term representing `Fn<T>(T, T) -> T` given
    /// the substitution map made of the single substitution `T |-> int`.
    /// This allows us to resolve the type of `add::<int>` to the function
    /// `Fn<int>(int, int) -> int`.
    fn instantiate_term(&mut self, term: TermId, subst: &mut HashMap<GenericId, TermId>) -> TermId {
        let resolved = self.uni_cx.resolve(term);
        match self.uni_cx.term(resolved).cloned() {
            // The term is just some inference variable `?a`. Nothing to do.
            Some(Term::Var(_)) => resolved,
            // The term is a generic type parameter `T`. If there is a mapping
            // `T |-> t` where `t` is some term, then make the substitution,
            // otherwise make the substitution `T |-> ?a` where `?a` is a
            // fresh instance variable.
            Some(Term::App {
                constructor: TyCon::Generic(id),
                ..
            }) => *subst.entry(id).or_insert_with(|| {
                let var = self.uni_cx.fresh_var();
                term!(self.uni_cx, var var)
            }),
            // The term is some arbitrary constructor applied to some arbitrary
            // arguments. So recursively check the arguments for generic
            // type parameters that still need to be substituted.
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

    /// Makes substitutions for all the generic type parameters of the given
    /// symbol. If the given path contains any explicit type arguments, then
    /// it will directly make those substitutions. Otherwise, fresh inference
    /// variables are introduced in place of each generic type parameter.
    ///
    /// For example, suppose we have some function
    /// ```ignore
    /// fn identity<T>(x: T) -> T {
    ///     x
    /// }
    /// ```
    /// and then we call this function with an explitit type argument `int`:
    /// ```ignore
    /// let y = 10;
    /// let z = identity::<int>(y);
    /// ```
    /// Then this function returns a term which represents the type of the
    /// `identity` function such that the type parameter `T` is replaced with
    /// the concrete type `int`, giving `identity::<int>` the type
    /// `Fn(int) -> int`.
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
