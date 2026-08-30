//! Contains all methods on the [`TypeCheckContext`] related to the
//! implementation of polymorphism via generic type parameters.
//! Specifically, these methods facilitate the generalisation of
//! free inference variables into generic type parameters for nested
//! functions, as well as the instantiation of tys which contain
//! generics by substituting them for concrete tys, or otherwise
//! substituting them for fresh inference variables.

use std::collections::{HashMap, HashSet};

use ast::{GenericArg, Path, Span};

use crate::errors::GenericArgumentCountMismatch;
use crate::generics::SyntheticNames;
use crate::inference::{child_tys, map_children};
use crate::types::TyKind;
use crate::{DefId, DefIdOf, FnDef, GenericId, TyId, TypeCheckContext, VarId};

impl<'ast> TypeCheckContext<'ast> {
    /// Returns all of the free inference variables which appear
    /// within the given ty.
    ///
    /// A type inference variable is considered `free` when it has not
    /// yet been bound to any ty. More precisely, given a ty `t`,
    /// `free_vars(t)` returns the set of inference variables `?a` that
    /// occur in `t` and are not currently bound by the unification
    /// context.
    fn free_vars(&mut self, ty: TyId, out: &mut Vec<VarId>) {
        let resolved = self.inf.resolve(ty);
        match self.inf.ty(resolved).cloned() {
            Some(TyKind::Var(v)) => {
                let root = self.inf.find(v);
                if !out.contains(&root) {
                    out.push(root);
                }
            }
            Some(kind) => {
                child_tys(kind)
                    .into_iter()
                    .for_each(|arg| self.free_vars(arg, out));
            }
            None => {}
        }
    }

    fn enclosing_free_vars(&mut self) -> HashSet<VarId> {
        let mut out = Vec::new();
        self.recursion.stack().to_vec().into_iter().for_each(|def| {
            let ty = self.def(def).ty();
            self.free_vars(ty, &mut out);
        });
        out.into_iter().collect()
    }

    /// Reserves `def`'s own generic parameters' names in `names`, so a
    /// synthesised name never collides with one belonging to a
    /// *different* [`GenericId`] that's actually relevant to the
    /// current synthesis scope: `def`'s own generics, or (for the
    /// caller who loops this over `self.recursion.stack()`) an
    /// enclosing function's generics, since a nested function's
    /// synthesised generic can end up embedded in an enclosing
    /// function's rendered signature once it returns it.
    pub(crate) fn reserve_declared_generics(&self, def: DefId, names: &mut SyntheticNames) {
        self.def(def).generics().iter().for_each(|id| {
            if let Some(name) = self.generics.get(id) {
                names.reserve(name.clone());
            }
        });
    }

    pub(crate) fn generalize_group(&mut self, members: &[DefIdOf<FnDef>]) {
        let enclosing = self.enclosing_free_vars();

        // A fresh, local source of synthesised names for this group --
        // always starts at `T`, see `SyntheticNames`. Reserve names
        // already used by this group's own members, and by any
        // enclosing functions currently being checked (see
        // `reserve_declared_generics`).
        //
        // FIXME Ensure that generic type parameter names being taken in
        // one function, do not prevent the names from being used in
        // another function in the group. This is likely the cause of an
        // issue with LSP of mutually recursive functions where only one
        // explicitly specifies its generic type parameter.
        let mut names = SyntheticNames::new();
        members
            .iter()
            .for_each(|&def| self.reserve_declared_generics(def.id(), &mut names));
        self.recursion
            .stack()
            .to_vec()
            .into_iter()
            .for_each(|def| self.reserve_declared_generics(def, &mut names));

        // Begin the process of generalisation of nested functions for
        // each individual function in the group with members provided.
        //
        // Collect each of the free variables `?a` of each function in the
        // group, such that `?a` is not free in any *enclosing function*.
        // Here we mean either `?a` does not exist at all in the closing
        // function context.
        //
        // These become candidates for generalisation.
        let per_member_vars: Vec<(DefIdOf<FnDef>, Vec<VarId>)> = members
            .iter()
            .map(|&def| {
                let ty = self.defs.fn_ref(def).ty;
                let mut vars = Vec::new();
                self.free_vars(ty, &mut vars);
                vars.retain(|v| !enclosing.contains(v));
                (def, vars)
            })
            .collect();

        // For each unique variable collected, generalise it to a new
        // new unique generic type parameter.
        let mut assigned: HashMap<VarId, GenericId> = HashMap::new();
        per_member_vars.iter().for_each(|(_, vars)| {
            vars.iter().for_each(|&var| {
                if let std::collections::hash_map::Entry::Vacant(entry) = assigned.entry(var) {
                    let id = self.generics.declare_synthetic(&mut names);
                    let generic_ty = self.ty(TyKind::Generic(id));
                    self.inf.bind(var, generic_ty);
                    entry.insert(id);
                }
            });
        });

        // For each function, append all new generics synthesised from
        // generalisation to its def.
        per_member_vars.into_iter().for_each(|(fn_def, vars)| {
            vars.into_iter().for_each(|var| {
                let id = assigned[&var];
                self.defs.fn_mut(fn_def).generics.push(id);
            });
        });
    }

    fn build_subst(
        &mut self,
        generics: &[GenericId],
        explicit: &[(TyId, Span)],
    ) -> HashMap<GenericId, TyId> {
        let mut subst = HashMap::new();
        generics
            .iter()
            .zip(explicit)
            .for_each(|(&id, &(ty, span))| {
                let var_ty = self.fresh_var();
                let _ = self.inf.unify_because(var_ty, ty, span);
                subst.insert(id, var_ty);
            });
        subst
    }

    fn explicit_generic_args(&mut self, path: &Path, generics: &[GenericId]) -> Vec<(TyId, Span)> {
        let Some(generic_args) = path.segments.last().and_then(|seg| seg.args.as_ref()) else {
            return Vec::new();
        };

        let arg_tys: Vec<(TyId, Span)> = generic_args
            .args
            .iter()
            .filter_map(|arg| match arg {
                GenericArg::Arg(ty) => Some((self.lower_ty(ty), ty.span)),
                GenericArg::Constraint(_) => None,
            })
            .collect();

        let max = generics.len();
        let actual = arg_tys.len();
        if actual != max {
            self.diagnostics.push(GenericArgumentCountMismatch {
                span: generic_args.span,
                expected: max,
                found: actual,
            });
        }
        arg_tys.into_iter().take(max).collect()
    }

    /// Instantiates a new ty by substituting all generic type parameters
    /// which appear in the ty with the tys they represent, according
    /// to the substitution map provided. If there is no substitution specified
    /// for a given generic type parameter, then substitute it with a fresh
    /// inference variable.
    ///
    /// For example, given `t` as the ty `(T, U)`, and the substitution map
    /// given by `T |-> String` and `U -> int`, it would produce a new ty
    /// given by applying the substitutions to `t`, which would be (String, int).
    /// However, if we were only given the first substitution, `T |-> String`,
    /// then `U` would instead be substituted with some new inference variable
    /// `?a`.
    ///
    /// This is used when generic type parameters of a def need to be
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
    /// Needs to instantiate the ty representing `Fn<T>(T, T) -> T` given
    /// the substitution map made of the single substitution `T |-> int`.
    /// This allows us to resolve the type of `add::<int>` to the function
    /// `Fn<int>(int, int) -> int`.
    pub(crate) fn instantiate_ty(
        &mut self,
        ty: TyId,
        subst: &mut HashMap<GenericId, TyId>,
    ) -> TyId {
        let resolved = self.inf.resolve(ty);
        match self.inf.ty(resolved).cloned() {
            // The ty is just some inference variable `?a`. Nothing to do.
            Some(TyKind::Var(_)) => resolved,
            // The ty is a generic type parameter `T`. If there is a mapping
            // `T |-> t` where `t` is some ty, then make the substitution,
            // otherwise make the substitution `T |-> ?a` where `?a` is a
            // fresh instance variable.
            Some(TyKind::Generic(id)) => *subst.entry(id).or_insert_with(|| self.fresh_var()),
            // The ty is some arbitrary constructor applied to some arbitrary
            // arguments. So recursively check the arguments for generic
            // type parameters that still need to be substituted.
            Some(kind) => {
                let mapped = map_children(kind, |arg| self.instantiate_ty(arg, subst));
                self.ty(mapped)
            }
            None => resolved,
        }
    }

    /// Makes substitutions for all the generic type parameters of the given
    /// def. If the given path contains any explicit type arguments, then
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
    /// Then this function returns a ty which represents the type of the
    /// `identity` function such that the type parameter `T` is replaced with
    /// the concrete type `int`, giving `identity::<int>` the type
    /// `Fn(int) -> int`.
    pub(crate) fn instantiate_path(&mut self, def: DefId, path: &Path) -> TyId {
        let ty = self.def(def).ty();
        let generics = self.def(def).generics().to_vec();
        if generics.is_empty() {
            return ty;
        }
        let mut subst = self.subst_for(&generics, path);
        self.instantiate_ty(ty, &mut subst)
    }

    pub(crate) fn subst_for(
        &mut self,
        generics: &[GenericId],
        path: &Path,
    ) -> HashMap<GenericId, TyId> {
        let explicit = self.explicit_generic_args(path, generics);
        self.build_subst(generics, &explicit)
    }

    pub(crate) fn args_from_subst(
        &mut self,
        generics: &[GenericId],
        subst: &mut HashMap<GenericId, TyId>,
    ) -> Vec<TyId> {
        generics
            .iter()
            .map(|&id| {
                let placeholder = self.ty(TyKind::Generic(id));
                self.instantiate_ty(placeholder, subst)
            })
            .collect()
    }

    pub(crate) fn instantiate_adt_args(
        &mut self,
        generics: &[GenericId],
        path: &Path,
    ) -> Vec<TyId> {
        if generics.is_empty() {
            return Vec::new();
        }
        let mut subst = self.subst_for(generics, path);
        self.args_from_subst(generics, &mut subst)
    }

    pub(crate) fn instantiate_struct_fields(
        &mut self,
        generics: &[GenericId],
        path: &Path,
        field_tys: &[TyId],
    ) -> (Vec<TyId>, Vec<TyId>) {
        if generics.is_empty() {
            return (field_tys.to_vec(), Vec::new());
        }

        let mut subst = self.subst_for(generics, path);
        let fields = field_tys
            .iter()
            .map(|&ty| self.instantiate_ty(ty, &mut subst))
            .collect();
        let args = self.args_from_subst(generics, &mut subst);

        (fields, args)
    }
}
