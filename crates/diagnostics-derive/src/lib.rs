use proc_macro::TokenStream;
use proc_macro2::TokenStream as TokenStream2;
use quote::quote;
use syn::{Data, DeriveInput, Fields, Ident, LitInt, LitStr, Type, parse_macro_input};

enum FieldKind {
    Plain,
    Span,
    Related,
    Note,
    Skip,
}

fn kebab_case(name: &str) -> String {
    let mut result = String::new();
    for (i, c) in name.char_indices() {
        if c.is_uppercase() {
            if i != 0 {
                result.push('-');
            }
            result.extend(c.to_lowercase());
        } else {
            result.push(c);
        }
    }
    result
}

fn type_is_ident(ty: &Type, ident: &str) -> bool {
    match ty {
        Type::Path(path) => path
            .path
            .segments
            .last()
            .is_some_and(|segment| segment.ident == ident),
        _ => false,
    }
}

fn level_variant(level: &str) -> syn::Result<TokenStream2> {
    match level {
        "error" => Ok(quote! { Error }),
        "warning" => Ok(quote! { Warning }),
        "note" => Ok(quote! { Note }),
        "help" => Ok(quote! { Help }),
        other => Err(syn::Error::new(
            proc_macro2::Span::call_site(),
            format!("unknown diagnose level `{other}`"),
        )),
    }
}

fn expand(input: DeriveInput) -> syn::Result<TokenStream2> {
    let ident = &input.ident;

    let mut code: Option<u32> = None;
    let mut level: Option<String> = None;
    let mut message: Option<String> = None;

    for attr in &input.attrs {
        if !attr.path().is_ident("diagnose") {
            continue;
        }
        attr.parse_nested_meta(|meta| {
            if meta.path.is_ident("code") {
                let lit: LitInt = meta.value()?.parse()?;
                code = Some(lit.base10_parse()?);
                Ok(())
            } else if meta.path.is_ident("level") {
                let lit: LitStr = meta.value()?.parse()?;
                level = Some(lit.value());
                Ok(())
            } else if meta.path.is_ident("message") {
                let lit: LitStr = meta.value()?.parse()?;
                message = Some(lit.value());
                Ok(())
            } else {
                Err(meta.error("unknown diagnose attribute"))
            }
        })?;
    }

    let code =
        code.ok_or_else(|| syn::Error::new_spanned(&input, "missing #[diagnose(code = ...)]"))?;
    let level = level
        .ok_or_else(|| syn::Error::new_spanned(&input, "missing #[diagnose(level = \"...\")]"))?;
    let level_variant = level_variant(&level)?;
    let message_id = message.unwrap_or_else(|| kebab_case(&ident.to_string()));

    let Data::Struct(data) = &input.data else {
        return Err(syn::Error::new_spanned(
            &input,
            "Diagnose can only be derived for structs",
        ));
    };
    let Fields::Named(fields) = &data.fields else {
        return Err(syn::Error::new_spanned(
            &input,
            "Diagnose requires named fields",
        ));
    };

    let mut span_field: Option<Ident> = None;
    let mut arg_fields: Vec<Ident> = Vec::new();
    let mut emphasize_pairs: Vec<(Ident, String)> = Vec::new();
    let mut related_fields: Vec<(Ident, bool)> = Vec::new();
    let mut note_fields: Vec<(Ident, bool)> = Vec::new();

    for field in &fields.named {
        let field_ident = field.ident.clone().unwrap();
        let mut kind = FieldKind::Plain;
        let mut emphasize_target: Option<String> = None;

        for attr in &field.attrs {
            if !attr.path().is_ident("diagnose") {
                continue;
            }
            attr.parse_nested_meta(|meta| {
                if meta.path.is_ident("span") {
                    kind = FieldKind::Span;
                    Ok(())
                } else if meta.path.is_ident("related") {
                    kind = FieldKind::Related;
                    Ok(())
                } else if meta.path.is_ident("note") {
                    kind = FieldKind::Note;
                    Ok(())
                } else if meta.path.is_ident("skip") {
                    kind = FieldKind::Skip;
                    Ok(())
                } else if meta.path.is_ident("emphasize_in") {
                    let lit: LitStr = meta.value()?.parse()?;
                    emphasize_target = Some(lit.value());
                    Ok(())
                } else {
                    Err(meta.error("unknown diagnose field attribute"))
                }
            })?;
        }

        match kind {
            FieldKind::Span => span_field = Some(field_ident),
            FieldKind::Related => {
                let is_option = type_is_ident(&field.ty, "Option");
                related_fields.push((field_ident, is_option));
            }
            FieldKind::Note => {
                let is_option = type_is_ident(&field.ty, "Option");
                note_fields.push((field_ident, is_option));
            }
            FieldKind::Skip => {}
            FieldKind::Plain => {
                if let Some(target) = emphasize_target {
                    emphasize_pairs.push((field_ident.clone(), target));
                }
                arg_fields.push(field_ident);
            }
        }
    }

    let span_field = span_field
        .ok_or_else(|| syn::Error::new_spanned(&input, "missing #[diagnose(span)] field"))?;

    let arg_pushes = arg_fields.iter().map(|field| {
        let name = field.to_string();
        quote! {
            (#name, ::diagnostics::ToArgValue::to_arg_value(&self.#field))
        }
    });

    let emphasize_entries = emphasize_pairs.iter().map(|(field, target)| {
        let field_name = field.to_string();
        quote! { (#target, #field_name) }
    });

    let related_pushes = related_fields.iter().map(|(field, is_option)| {
        if *is_option {
            quote! {
                if let Some(value) = &self.#field {
                    related.push(::std::clone::Clone::clone(value));
                }
            }
        } else {
            quote! {
                related.extend(self.#field.iter().cloned());
            }
        }
    });

    let note_pushes = note_fields.iter().map(|(field, is_option)| {
        if *is_option {
            quote! {
                if let Some(value) = &self.#field {
                    notes.push(::std::clone::Clone::clone(value));
                }
            }
        } else {
            quote! {
                notes.extend(self.#field.iter().cloned());
            }
        }
    });

    Ok(quote! {
        impl ::diagnostics::Diagnose for #ident {
            const CODE: ::diagnostics::ErrorCode = ::diagnostics::ErrorCode(#code);
            const LEVEL: ::diagnostics::Level = ::diagnostics::Level::#level_variant;

            fn span(&self) -> ::ast::Span {
                self.#span_field
            }

            fn message_id(&self) -> &'static str {
                #message_id
            }

            fn args(&self) -> ::std::vec::Vec<(&'static str, ::diagnostics::ArgValue)> {
                ::std::vec![#(#arg_pushes),*]
            }

            fn emphasize(&self) -> ::std::vec::Vec<(&'static str, &'static str)> {
                ::std::vec![#(#emphasize_entries),*]
            }

            fn related(&self) -> ::std::vec::Vec<::diagnostics::Related> {
                let mut related = ::std::vec::Vec::new();
                #(#related_pushes)*
                related
            }

            fn notes(&self) -> ::std::vec::Vec<::diagnostics::Note> {
                let mut notes = ::std::vec::Vec::new();
                #(#note_pushes)*
                notes
            }
        }
    })
}

#[proc_macro_derive(Diagnose, attributes(diagnose))]
pub fn derive_diagnose(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    expand(input)
        .unwrap_or_else(|err| err.to_compile_error())
        .into()
}
