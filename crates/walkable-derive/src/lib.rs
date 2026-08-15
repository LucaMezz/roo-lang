use proc_macro::TokenStream;
use quote::quote;
use syn::{Data, DeriveInput, Fields, Index, parse_macro_input};

fn is_hookable(input: &DeriveInput) -> bool {
    input.attrs.iter().any(|attr| {
        attr.path().is_ident("walkable")
            && attr
                .parse_args::<syn::Ident>()
                .is_ok_and(|ident| ident == "hookable")
    })
}

fn struct_body(fields: &Fields) -> proc_macro2::TokenStream {
    match fields {
        Fields::Named(fields) => {
            let visits = fields.named.iter().map(|f| {
                let ident = f.ident.as_ref().unwrap();
                quote! { crate::visit::Visitable::visit(&self.#ident, visitor); }
            });
            quote! { #(#visits)* }
        }
        Fields::Unnamed(fields) => {
            let visits = (0..fields.unnamed.len()).map(|i| {
                let index = Index::from(i);
                quote! { crate::visit::Visitable::visit(&self.#index, visitor); }
            });
            quote! { #(#visits)* }
        }
        Fields::Unit => quote! {},
    }
}

fn enum_arm(variant_ident: &syn::Ident, fields: &Fields) -> proc_macro2::TokenStream {
    match fields {
        Fields::Named(fields) => {
            let idents: Vec<_> = fields
                .named
                .iter()
                .map(|f| f.ident.as_ref().unwrap())
                .collect();
            let visits = idents
                .iter()
                .map(|ident| quote! { crate::visit::Visitable::visit(#ident, visitor); });
            quote! {
                Self::#variant_ident { #(#idents),* } => { #(#visits)* }
            }
        }
        Fields::Unnamed(fields) => {
            let bindings: Vec<_> = (0..fields.unnamed.len())
                .map(|i| quote::format_ident!("__{i}"))
                .collect();
            let visits = bindings
                .iter()
                .map(|binding| quote! { crate::visit::Visitable::visit(#binding, visitor); });
            quote! {
                Self::#variant_ident(#(#bindings),*) => { #(#visits)* }
            }
        }
        Fields::Unit => quote! { Self::#variant_ident => {} },
    }
}

#[proc_macro_derive(Walkable, attributes(walkable))]
pub fn derive_walkable(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    let ident = &input.ident;
    let hookable = is_hookable(&input);

    let walk_body = match &input.data {
        Data::Struct(data) => struct_body(&data.fields),
        Data::Enum(data) => {
            let arms = data
                .variants
                .iter()
                .map(|variant| enum_arm(&variant.ident, &variant.fields));
            quote! {
                match self {
                    #(#arms)*
                }
            }
        }
        Data::Union(_) => panic!("Walkable cannot be derived for unions"),
    };

    let walk_impl = quote! {
        impl<V: crate::visit::Visitor> crate::visit::Walkable<V> for #ident {
            fn walk(&self, visitor: &mut V) {
                #walk_body
            }
        }
    };

    let visitable_impl = if hookable {
        quote! {}
    } else {
        quote! {
            impl<V: crate::visit::Visitor> crate::visit::Visitable<V> for #ident {
                fn visit(&self, visitor: &mut V) {
                    crate::visit::Walkable::walk(self, visitor);
                }
            }
        }
    };

    quote! {
        #walk_impl
        #visitable_impl
    }
    .into()
}
