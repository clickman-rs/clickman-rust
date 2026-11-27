//! Derive macros of `clickman-event`; see that crate for their documentation.

use proc_macro::TokenStream;
use proc_macro2::{Span, TokenStream as Tokens};
use quote::quote;
use syn::spanned::Spanned;
use syn::{
    Attribute, Data, DeriveInput, Error, Expr, ExprLit, Fields, GenericArgument, Lit, LitStr, Meta,
    PathArguments, Result, Type, parse_macro_input,
};

const MAX_EVENT_NAME: usize = 200;
const MAX_PROPERTY_NAME: usize = 128;
const MAX_DESCRIPTION: usize = 1000;
const MAX_VALUE: usize = 1024;

/// Implements `clickman_event::Event` for an enum whose variants are events.
#[proc_macro_derive(Event, attributes(clickman))]
pub fn derive_event(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    event(&input)
        .unwrap_or_else(Error::into_compile_error)
        .into()
}

/// Implements `clickman_event::Values` for an enum of unit variants.
#[proc_macro_derive(Values, attributes(clickman))]
pub fn derive_values(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    values(&input)
        .unwrap_or_else(Error::into_compile_error)
        .into()
}

fn event(input: &DeriveInput) -> Result<Tokens> {
    let Data::Enum(data) = &input.data else {
        return Err(Error::new(
            input.ident.span(),
            "derive `Event` on an enum whose variants are the events",
        ));
    };
    if data.variants.is_empty() {
        return Err(Error::new(input.ident.span(), "declare at least one event"));
    }
    let mut seen = Vec::new();
    let (mut names, mut properties, mut specs) = (Vec::new(), Vec::new(), Vec::new());
    for variant in &data.variants {
        let options = Options::of(&variant.attrs)?;
        if options.text {
            return Err(Error::new(variant.span(), "`text` applies to a property"));
        }
        let ident = &variant.ident;
        let name = options
            .name
            .unwrap_or_else(|| snake_case(&ident.to_string()));
        check_name(&name, MAX_EVENT_NAME, ident.span())?;
        if seen.contains(&name) {
            return Err(Error::new(
                ident.span(),
                format!("`{name}` is declared twice"),
            ));
        }
        seen.push(name.clone());
        let description = described(&variant.attrs, ident.span(), "an event")?;
        match &variant.fields {
            Fields::Unit => {
                names.push(quote!(Self::#ident => #name));
                properties.push(quote!(Self::#ident => {}));
                specs.push(quote! {
                    ::clickman_event::EventSpec {
                        name: #name,
                        description: #description,
                        properties: ::std::vec::Vec::new(),
                    }
                });
            }
            Fields::Named(fields) => {
                let mut property_names = Vec::new();
                let (mut bindings, mut inserts, mut property_specs) =
                    (Vec::new(), Vec::new(), Vec::new());
                for field in &fields.named {
                    let field_ident = field.ident.as_ref().expect("a named field");
                    let options = Options::of(&field.attrs)?;
                    let property = options.name.unwrap_or_else(|| {
                        field_ident.to_string().trim_start_matches("r#").to_owned()
                    });
                    check_name(&property, MAX_PROPERTY_NAME, field_ident.span())?;
                    if property_names.contains(&property) {
                        return Err(Error::new(
                            field_ident.span(),
                            format!("`{property}` is declared twice"),
                        ));
                    }
                    property_names.push(property.clone());
                    let description = described(&field.attrs, field_ident.span(), "a property")?;
                    let (inner, required) = match optional(&field.ty) {
                        Some(inner) => (inner, false),
                        None => (&field.ty, true),
                    };
                    match (is_string(inner), options.text) {
                        (true, false) => {
                            return Err(Error::new(
                                field.ty.span(),
                                "a string can carry personal data: declare it `#[clickman(text)]` \
                                 or take a `Values` enum",
                            ));
                        }
                        (false, true) => {
                            return Err(Error::new(
                                field.ty.span(),
                                "`text` applies to a `String` property",
                            ));
                        }
                        _ => {}
                    }
                    bindings.push(field_ident);
                    inserts.push(if required {
                        quote! {
                            properties.insert(
                                ::std::string::String::from(#property),
                                ::clickman_event::Property::to_value(#field_ident),
                            );
                        }
                    } else {
                        quote! {
                            if let ::std::option::Option::Some(value) = #field_ident {
                                properties.insert(
                                    ::std::string::String::from(#property),
                                    ::clickman_event::Property::to_value(value),
                                );
                            }
                        }
                    });
                    property_specs.push(quote! {
                        ::clickman_event::PropertySpec::of::<#inner>(#property, #description, #required)
                    });
                }
                names.push(quote!(Self::#ident { .. } => #name));
                properties.push(quote!(Self::#ident { #(#bindings),* } => { #(#inserts)* }));
                specs.push(quote! {
                    ::clickman_event::EventSpec {
                        name: #name,
                        description: #description,
                        properties: ::std::vec![#(#property_specs),*],
                    }
                });
            }
            Fields::Unnamed(_) => {
                return Err(Error::new(
                    variant.span(),
                    "an event names its properties: write `Variant { field: Type }`",
                ));
            }
        }
    }
    let ty = &input.ident;
    let (impl_generics, ty_generics, where_clause) = input.generics.split_for_impl();
    Ok(quote! {
        impl #impl_generics ::clickman_event::Event for #ty #ty_generics #where_clause {
            fn name(&self) -> &'static str {
                match self {
                    #(#names,)*
                }
            }

            fn properties(
                &self,
            ) -> ::clickman_event::serde_json::Map<
                ::std::string::String,
                ::clickman_event::serde_json::Value,
            > {
                #[allow(unused_mut)]
                let mut properties = ::clickman_event::serde_json::Map::new();
                match self {
                    #(#properties)*
                }
                properties
            }

            fn catalog() -> ::clickman_event::Catalog {
                ::clickman_event::Catalog {
                    events: ::std::vec![#(#specs),*],
                }
            }
        }
    })
}

fn values(input: &DeriveInput) -> Result<Tokens> {
    let Data::Enum(data) = &input.data else {
        return Err(Error::new(
            input.ident.span(),
            "derive `Values` on an enum of unit variants",
        ));
    };
    if data.variants.is_empty() {
        return Err(Error::new(input.ident.span(), "declare at least one value"));
    }
    let (mut values, mut arms) = (Vec::new(), Vec::new());
    for variant in &data.variants {
        if !matches!(variant.fields, Fields::Unit) {
            return Err(Error::new(variant.span(), "a value is a unit variant"));
        }
        let options = Options::of(&variant.attrs)?;
        let ident = &variant.ident;
        let value = options
            .name
            .unwrap_or_else(|| snake_case(&ident.to_string()));
        if value.is_empty()
            || value.chars().count() > MAX_VALUE
            || value.chars().any(char::is_control)
        {
            return Err(Error::new(
                ident.span(),
                "a value has 1 to 1024 characters without control characters",
            ));
        }
        if values.contains(&value) {
            return Err(Error::new(
                ident.span(),
                format!("`{value}` is declared twice"),
            ));
        }
        arms.push(quote!(Self::#ident => #value));
        values.push(value);
    }
    let ty = &input.ident;
    let (impl_generics, ty_generics, where_clause) = input.generics.split_for_impl();
    Ok(quote! {
        impl #impl_generics ::clickman_event::Values for #ty #ty_generics #where_clause {
            const VALUES: &'static [&'static str] = &[#(#values),*];

            fn value(&self) -> &'static str {
                match self {
                    #(#arms,)*
                }
            }
        }

        impl #impl_generics ::clickman_event::Property for #ty #ty_generics #where_clause {
            const KIND: ::clickman_event::Kind =
                ::clickman_event::Kind::Values(<Self as ::clickman_event::Values>::VALUES);

            fn to_value(&self) -> ::clickman_event::serde_json::Value {
                ::clickman_event::serde_json::Value::from(
                    ::clickman_event::Values::value(self),
                )
            }
        }
    })
}

/// `#[clickman(name = "…")]` and `#[clickman(text)]`.
#[derive(Default)]
struct Options {
    name: Option<String>,
    text: bool,
}

impl Options {
    fn of(attributes: &[Attribute]) -> Result<Self> {
        let mut options = Self::default();
        for attribute in attributes.iter().filter(|a| a.path().is_ident("clickman")) {
            attribute.parse_nested_meta(|meta| {
                if meta.path.is_ident("name") {
                    options.name = Some(meta.value()?.parse::<LitStr>()?.value());
                    Ok(())
                } else if meta.path.is_ident("text") {
                    options.text = true;
                    Ok(())
                } else {
                    Err(meta.error("expected `name = \"…\"` or `text`"))
                }
            })?;
        }
        Ok(options)
    }
}

/// The `///` lines, trimmed and joined with single spaces.
fn described(attributes: &[Attribute], span: Span, what: &str) -> Result<String> {
    let lines: Vec<String> = attributes
        .iter()
        .filter(|attribute| attribute.path().is_ident("doc"))
        .filter_map(|attribute| match &attribute.meta {
            Meta::NameValue(pair) => match &pair.value {
                Expr::Lit(ExprLit {
                    lit: Lit::Str(text),
                    ..
                }) => Some(text.value()),
                _ => None,
            },
            _ => None,
        })
        .collect();
    let description = lines
        .iter()
        .flat_map(|line| line.lines())
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>()
        .join(" ");
    if description.is_empty() {
        return Err(Error::new(
            span,
            format!("{what} needs a `///` description: the event catalog shows it"),
        ));
    }
    if description.chars().count() > MAX_DESCRIPTION {
        return Err(Error::new(
            span,
            "a description has at most 1000 characters",
        ));
    }
    if description
        .chars()
        .any(|c| c.is_control() || matches!(c, '\u{2028}' | '\u{2029}'))
    {
        return Err(Error::new(
            span,
            "a description is one line without control characters",
        ));
    }
    Ok(description)
}

fn check_name(name: &str, max: usize, span: Span) -> Result<()> {
    let mut chars = name.chars();
    let valid = chars.next().is_some_and(|first| first.is_ascii_lowercase())
        && chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
        && name.len() <= max;
    if valid {
        Ok(())
    } else {
        Err(Error::new(
            span,
            format!("`{name}` is not a snake case name of 1 to {max} characters"),
        ))
    }
}

/// `ExportCompleted` → `export_completed`, `HTTPError` → `http_error`.
fn snake_case(ident: &str) -> String {
    let chars: Vec<char> = ident.trim_start_matches("r#").chars().collect();
    let mut snake = String::new();
    for (index, &c) in chars.iter().enumerate() {
        if c.is_uppercase() {
            let previous = index.checked_sub(1).map(|i| chars[i]);
            let next = chars.get(index + 1);
            let after_word = previous.is_some_and(|p| p.is_lowercase() || p.is_ascii_digit());
            let ends_acronym =
                previous.is_some_and(char::is_uppercase) && next.is_some_and(|n| n.is_lowercase());
            if (after_word || ends_acronym) && !snake.ends_with('_') {
                snake.push('_');
            }
            snake.extend(c.to_lowercase());
        } else {
            snake.push(c);
        }
    }
    snake
}

/// `T` of a field of type `Option<T>`.
fn optional(ty: &Type) -> Option<&Type> {
    let Type::Path(path) = ty else { return None };
    let last = path.path.segments.last()?;
    if last.ident != "Option" {
        return None;
    }
    let PathArguments::AngleBracketed(arguments) = &last.arguments else {
        return None;
    };
    match arguments.args.first()? {
        GenericArgument::Type(inner) => Some(inner),
        _ => None,
    }
}

/// Whether a field is a `String` or a `&str`.
fn is_string(ty: &Type) -> bool {
    match ty {
        Type::Path(path) => path
            .path
            .segments
            .last()
            .is_some_and(|last| last.ident == "String" || last.ident == "str"),
        Type::Reference(reference) => is_string(&reference.elem),
        _ => false,
    }
}
