use std::collections::HashMap;

use ast::Item;
use diagnostics::Diagnostic;
use intern::Interner;

use crate::defs::{DefKind, FnDef};
use crate::errors::{Locale, TypeCheckDiagnostic};
use crate::position_index::PositionIndex;
use crate::types::{Type, TypeResolver};
use crate::{DefId, TypeCheckContext};

impl<'ast> TypeCheckContext<'ast> {
    /// Constructs the final result of the type checking stage, which
    /// will be output to the client of this crate.
    ///
    /// The process of freezing the [`TypeCheckContext`] to arrive at
    /// the final checked program involves converting  all defs
    /// within the def table into frozen defs. This replaces
    /// interned symbol ids with the actual symbol strings, converts the
    /// DefKind to a FrozenDefKind, and resolves the `Ty`
    /// representing the type of the def to a frozen `Type`.
    ///
    /// TODO Make the CheckedProgram result produced by the
    /// TypeCheckContext actually preserve the module -> exported
    /// def structure, since the embedding API will need to allow
    /// for calling / getting defs of items within modules.
    fn freeze(mut self) -> CheckedProgram {
        let mut defs = HashMap::with_capacity(self.defs.len());
        for (id, def) in self.defs.iter() {
            let ty = match def.kind.ty() {
                Some(ty) => TypeResolver {
                    inf: &mut self.inf,
                    defs: &self.defs,
                    names: &self.symbols,
                    generics: &self.generics,
                }
                .resolve(ty),
                None => Type::Unresolved,
            };
            let symbol = self.symbols.resolve(def.symbol).to_owned();
            let generics = def
                .generics()
                .iter()
                .map(|id| {
                    self.generics
                        .get(id)
                        .cloned()
                        .unwrap_or_else(|| "<generic>".to_owned())
                })
                .collect();
            let kind = match &def.kind {
                DefKind::Fn(FnDef { params, .. }) => FrozenDefKind::Fn {
                    param_symbols: params.iter().map(|p| p.symbol.clone()).collect(),
                },
                DefKind::Param(_) => FrozenDefKind::Param,
                DefKind::Local(_) => FrozenDefKind::Local,
                DefKind::TyAlias(_) => FrozenDefKind::TyAlias,
                DefKind::Mod(_) => FrozenDefKind::Mod,
                DefKind::Struct(_)
                | DefKind::Enum(_)
                | DefKind::Variant(_)
                | DefKind::Trait
                | DefKind::GenericParam(_) => FrozenDefKind::Other,
            };
            defs.insert(
                id,
                FrozenDef {
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
            defs,
        }
    }
}

struct FrozenDef {
    symbol: String,
    kind: FrozenDefKind,
    ty: Type,
    generics: Vec<String>,
}

enum FrozenDefKind {
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
    defs: HashMap<DefId, FrozenDef>,
}

impl CheckedProgram {
    /// Creates a new [`CheckedProgram`] by performing the entire
    /// type checking process on the given items. Performs type
    /// inference and confirms all types are compatible, and then
    /// returns the [`CheckedProgram`] containing information
    /// about the discovered items and their defs and types.
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

    /// Queries the checked program to see if there is a def
    /// associated with a specific offset in the source file.
    pub fn def_at(&self, offset: usize) -> Option<DefId> {
        self.positions.def_at(offset)
    }

    /// Queries the checked program to see if there is a concrete
    /// primitive type symbol at a certain offset in the source file.
    pub fn type_symbol_at(&self, offset: usize) -> Option<&'static str> {
        self.positions.type_name_at(offset)
    }

    fn def(&self, def: DefId) -> &FrozenDef {
        &self.defs[&def]
    }

    /// Returns a string representation of the type associated with
    /// a given def.
    pub fn render_def_type(&self, def: DefId) -> String {
        let bind = self.def(def);
        let rendered = bind.ty.render();
        let generics_rendered = generics_list(&bind.generics);
        if generics_rendered.is_empty() {
            rendered
        } else {
            format!("{generics_rendered} {rendered}")
        }
    }

    /// Returns a string representation of a def.
    pub fn describe_def(&self, def: DefId) -> String {
        let bind = self.def(def);
        match &bind.kind {
            FrozenDefKind::Fn { param_symbols } => describe_fn_item(bind, param_symbols),
            FrozenDefKind::Param => format!("{}: {}", bind.symbol, bind.ty.render()),
            FrozenDefKind::Local => format!("let {}: {}", bind.symbol, bind.ty.render()),
            FrozenDefKind::TyAlias => {
                format!("type {}", alias_symbol_with_generics(bind))
            }
            FrozenDefKind::Mod => format!("mod {}", bind.symbol),
            FrozenDefKind::Other => self.render_def_type(def),
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

/// Returns a string with the def symbol followed by a
/// generic list of the def.
fn alias_symbol_with_generics(bind: &FrozenDef) -> String {
    format!("{}{}", bind.symbol, generics_list(&bind.generics))
}

/// Returns a full string representation of the entire signature
/// of a function.
fn describe_fn_item(bind: &FrozenDef, param_symbols: &[String]) -> String {
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
