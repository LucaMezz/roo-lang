use std::collections::HashMap;

use ast::Item;
use diagnostics::Diagnostic;
use intern::Interner;

use crate::errors::{Locale, TypeCheckDiagnostic};
use crate::position_index::PositionIndex;
use crate::types::{Type, resolve_type};
use crate::{BindingId, BindingKind, FnBinding, TypeCheckContext};

impl<'ast> TypeCheckContext<'ast> {
    /// Constructs the final result of the type checking stage, which
    /// will be output to the client of this crate.
    ///
    /// The process of freezing the [`TypeCheckContext`] to arrive at
    /// the final checked program involves converting  all bindings
    /// within the binding table into frozen bindings. This replaces
    /// interned symbol ids with the actual symbol strings, converts the
    /// BindingKind to a FrozenBindingKind, and resolves the `Ty`
    /// representing the type of the binding to a frozen `Type`.
    ///
    /// TODO Make the CheckedProgram result produced by the
    /// TypeCheckContext actually preserve the module -> exported
    /// binding structure, since the embedding API will need to allow
    /// for calling / getting bindings of items within modules.
    fn freeze(mut self) -> CheckedProgram {
        let mut bindings = HashMap::with_capacity(self.bindings.len());
        for (id, binding) in self.bindings.iter() {
            let ty = match binding.kind.ty() {
                Some(ty) => resolve_type(
                    &mut self.inf,
                    &self.bindings,
                    &self.symbols,
                    &self.generic_names,
                    ty,
                ),
                None => Type::Unresolved,
            };
            let symbol = self.symbols.resolve(binding.symbol).to_owned();
            let generics = binding
                .generics()
                .iter()
                .map(|id| {
                    self.generic_names
                        .get(id)
                        .cloned()
                        .unwrap_or_else(|| "<generic>".to_owned())
                })
                .collect();
            let kind = match &binding.kind {
                BindingKind::Fn(FnBinding { param_symbols, .. }) => FrozenBindingKind::Fn {
                    param_symbols: param_symbols.clone(),
                },
                BindingKind::Param(_) => FrozenBindingKind::Param,
                BindingKind::Local(_) => FrozenBindingKind::Local,
                BindingKind::TyAlias(_) => FrozenBindingKind::TyAlias,
                BindingKind::Mod(_) => FrozenBindingKind::Mod,
                BindingKind::Struct
                | BindingKind::Enum
                | BindingKind::Variant
                | BindingKind::Trait
                | BindingKind::GenericParam(_) => FrozenBindingKind::Other,
            };
            bindings.insert(
                id,
                FrozenBinding {
                    symbol,
                    kind,
                    ty,
                    generics,
                },
            );
        }

        CheckedProgram {
            diagnostics: self.diagnostics.into_vec(),
            positions: self.positions,
            bindings,
        }
    }
}

struct FrozenBinding {
    symbol: String,
    kind: FrozenBindingKind,
    ty: Type,
    generics: Vec<String>,
}

enum FrozenBindingKind {
    Fn { param_symbols: Vec<String> },
    Param,
    Local,
    TyAlias,
    Mod,
    Other,
}

/// The final result produced by the type checking process.
/// Also provides several functions to query the typed program.
pub struct CheckedProgram {
    diagnostics: Vec<TypeCheckDiagnostic>,
    positions: PositionIndex,
    bindings: HashMap<BindingId, FrozenBinding>,
}

impl CheckedProgram {
    /// Creates a new [`CheckedProgram`] by performing the entire
    /// type checking process on the given items. Performs type
    /// inference and confirms all types are compatible, and then
    /// returns the [`CheckedProgram`] containing information
    /// about the discovered items and their bindings and types.
    pub fn check(items: &[Box<Item>], symbols: Interner) -> CheckedProgram {
        let mut cx = TypeCheckContext::new(symbols);
        cx.resolve(items);
        cx.lower_signatures(items);
        cx.check(items);
        cx.freeze()
    }

    /// Gets all the diagnostics that were emitted during the
    /// type checking process, given a Locale.
    ///
    /// The actual diagnostic messages are stored in `Fluent`
    /// `.lft` files, so a [`Locale`] can be passed which will
    /// render the diagnostics with that locale.
    pub fn diagnostics(&self, locale: Locale) -> Vec<Diagnostic> {
        let catalog = crate::errors::catalog(locale);
        self.diagnostics.iter().map(|d| d.render(catalog)).collect()
    }

    /// Queries the checked program to see if there is a binding
    /// associated with a specific offset in the source file.
    pub fn binding_at(&self, offset: usize) -> Option<BindingId> {
        self.positions.binding_at(offset)
    }

    /// Queries the checked program to see if there is a concrete
    /// primitive type symbol at a certain offset in the source file.
    pub fn type_symbol_at(&self, offset: usize) -> Option<&'static str> {
        self.positions.type_name_at(offset)
    }

    fn binding(&self, binding: BindingId) -> &FrozenBinding {
        &self.bindings[&binding]
    }

    /// Returns a string representation of the type associated with
    /// a given binding.
    pub fn render_binding_type(&self, binding: BindingId) -> String {
        let bind = self.binding(binding);
        let rendered = bind.ty.render();
        let generics_rendered = generics_list(&bind.generics);
        if generics_rendered.is_empty() {
            rendered
        } else {
            format!("{generics_rendered} {rendered}")
        }
    }

    /// Returns a string representation of a binding.
    pub fn describe_binding(&self, binding: BindingId) -> String {
        let bind = self.binding(binding);
        match &bind.kind {
            FrozenBindingKind::Fn { param_symbols } => describe_fn_item(bind, param_symbols),
            FrozenBindingKind::Param => format!("{}: {}", bind.symbol, bind.ty.render()),
            FrozenBindingKind::Local => format!("let {}: {}", bind.symbol, bind.ty.render()),
            FrozenBindingKind::TyAlias => {
                format!("type {}", alias_symbol_with_generics(bind))
            }
            FrozenBindingKind::Mod => format!("mod {}", bind.symbol),
            FrozenBindingKind::Other => self.render_binding_type(binding),
        }
    }
}

/// Returns a string representation of a generic list.
fn generics_list(generics: &[String]) -> String {
    if generics.is_empty() {
        return String::new();
    }
    format!("<{}>", generics.join(", "))
}

/// Returns a string with the binding symbol followed by a
/// generic list of the binding.
fn alias_symbol_with_generics(bind: &FrozenBinding) -> String {
    format!("{}{}", bind.symbol, generics_list(&bind.generics))
}

/// Returns a full string representation of the entire signature
/// of a function.
fn describe_fn_item(bind: &FrozenBinding, param_symbols: &[String]) -> String {
    let generics_rendered = generics_list(&bind.generics);
    let Type::Fn(params, output) = &bind.ty else {
        return bind.ty.render();
    };
    let params: Vec<String> = params
        .iter()
        .enumerate()
        .map(|(i, ty)| {
            let rendered = ty.render();
            match param_symbols.get(i) {
                Some(symbol) => format!("{symbol}: {rendered}"),
                None => rendered,
            }
        })
        .collect();
    format!(
        "fn {}{generics_rendered}({}) -> {}",
        bind.symbol,
        params.join(", "),
        output.render()
    )
}
