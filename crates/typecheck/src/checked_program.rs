use std::collections::HashMap;

use ast::{Item, Span};

use crate::position_index::PositionIndex;
use crate::render::{Type, resolve_type};
use crate::{Diagnostic, FnSymbol, SymbolId, SymbolKind, TypeCheckContext};

impl TypeCheckContext {
    fn freeze(mut self) -> CheckedProgram {
        let mut symbols = HashMap::with_capacity(self.symbols.len());
        for (id, symbol) in self.symbols.iter() {
            let ty = resolve_type(
                &mut self.uni_cx,
                &self.symbols,
                &self.names,
                &self.generic_names,
                symbol.ty,
            );
            let name = self
                .names
                .name(symbol.name)
                .cloned()
                .unwrap_or_else(|| "_".to_owned());
            let generics = symbol
                .generics
                .iter()
                .map(|id| {
                    self.generic_names
                        .get(id)
                        .cloned()
                        .unwrap_or_else(|| "<generic>".to_owned())
                })
                .collect();
            let kind = match &symbol.kind {
                SymbolKind::Fn(FnSymbol { param_names, .. }) => FrozenSymbolKind::Fn {
                    param_names: param_names.clone(),
                },
                SymbolKind::Param => FrozenSymbolKind::Param,
                SymbolKind::Local => FrozenSymbolKind::Local,
                SymbolKind::TyAlias(_) => FrozenSymbolKind::TyAlias,
                SymbolKind::Struct
                | SymbolKind::Enum
                | SymbolKind::Variant
                | SymbolKind::Trait
                | SymbolKind::Mod(_)
                | SymbolKind::GenericParam => FrozenSymbolKind::Other,
            };
            symbols.insert(
                id,
                FrozenSymbol {
                    name,
                    kind,
                    ty,
                    generics,
                    declared_at: symbol.declared_at,
                },
            );
        }

        CheckedProgram {
            diagnostics: self.diagnostics.into_vec(),
            positions: self.positions,
            symbols,
        }
    }
}

struct FrozenSymbol {
    name: String,
    kind: FrozenSymbolKind,
    ty: Type,
    generics: Vec<String>,
    declared_at: Span,
}

enum FrozenSymbolKind {
    Fn { param_names: Vec<String> },
    Param,
    Local,
    TyAlias,
    Other,
}

pub struct CheckedProgram {
    diagnostics: Vec<Diagnostic>,
    positions: PositionIndex,
    symbols: HashMap<SymbolId, FrozenSymbol>,
}

impl CheckedProgram {
    pub fn check(items: &[Box<Item>]) -> CheckedProgram {
        let mut cx = TypeCheckContext::new();
        cx.resolve(items);
        cx.lower_signatures(items);
        cx.check(items);
        cx.freeze()
    }

    pub fn diagnostics(&self) -> &[Diagnostic] {
        &self.diagnostics
    }

    pub fn symbol_at(&self, offset: usize) -> Option<SymbolId> {
        self.positions.symbol_at(offset)
    }

    pub fn type_name_at(&self, offset: usize) -> Option<&'static str> {
        self.positions.type_name_at(offset)
    }

    pub fn render_symbol_type(&self, symbol: SymbolId) -> String {
        let sym = &self.symbols[&symbol];
        let rendered = sym.ty.render();
        let generics_rendered = generics_list(&sym.generics);
        if generics_rendered.is_empty() {
            rendered
        } else {
            format!("{generics_rendered} {rendered}")
        }
    }

    pub fn describe_symbol(&self, symbol: SymbolId, at: usize) -> String {
        let sym = &self.symbols[&symbol];
        match &sym.kind {
            FrozenSymbolKind::Fn { param_names } => describe_fn_item(sym, param_names),
            FrozenSymbolKind::Param => format!("{}: {}", sym.name, sym.ty.render()),
            FrozenSymbolKind::Local => format!("let {}: {}", sym.name, sym.ty.render()),
            FrozenSymbolKind::TyAlias => {
                let rendered = alias_name_with_generics(sym);
                if sym.declared_at.start <= at && at < sym.declared_at.end {
                    format!("type {rendered}")
                } else {
                    rendered
                }
            }
            FrozenSymbolKind::Other => self.render_symbol_type(symbol),
        }
    }
}

fn generics_list(generics: &[String]) -> String {
    if generics.is_empty() {
        return String::new();
    }
    format!("<{}>", generics.join(", "))
}

fn alias_name_with_generics(sym: &FrozenSymbol) -> String {
    format!("{}{}", sym.name, generics_list(&sym.generics))
}

fn describe_fn_item(sym: &FrozenSymbol, param_names: &[String]) -> String {
    let generics_rendered = generics_list(&sym.generics);
    let Type::Fn(params, output) = &sym.ty else {
        return sym.ty.render();
    };
    let params: Vec<String> = params
        .iter()
        .enumerate()
        .map(|(i, ty)| {
            let rendered = ty.render();
            match param_names.get(i) {
                Some(name) => format!("{name}: {rendered}"),
                None => rendered,
            }
        })
        .collect();
    format!(
        "fn {}{generics_rendered}({}) -> {}",
        sym.name,
        params.join(", "),
        output.render()
    )
}
