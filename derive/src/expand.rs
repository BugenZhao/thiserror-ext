use proc_macro2::TokenStream;
use quote::{format_ident, quote, ToTokens};
use syn::{
    spanned::Spanned, DeriveInput, GenericArgument, Ident, Member, PathArguments, Result, Type,
};

use crate::thiserror::ast::{Input, Variant};

struct Args {
    other_args: Vec<TokenStream>,
    source_arg: Option<TokenStream>,
    ctor_args: Vec<TokenStream>,
}

enum SourceInto {
    Yes,
    No,
}

fn resolve_variant_args(variant: &Variant<'_>, source_into: SourceInto, for_macro: bool) -> Args {
    let mut other_args = Vec::new();
    let mut source_arg = None;
    let mut ctor_args = Vec::new();

    for (i, field) in variant.fields.iter().enumerate() {
        let ty = &field.ty;
        let member = &field.member;

        let name = match &field.member {
            Member::Named(named) => named.clone(),
            Member::Unnamed(_) => {
                if field.is_non_from_source() {
                    format_ident!("source")
                } else {
                    format_ident!("arg_{}", i)
                }
            }
        };

        if field.is_backtrace() {
            let expr = if type_is_option(ty) {
                quote!(std::option::Option::Some(
                    std::backtrace::Backtrace::capture()
                ))
            } else {
                quote!(std::convert::From::from(
                    std::backtrace::Backtrace::capture()
                ))
            };
            ctor_args.push(quote!(#member: #expr,))
        } else if field.is_non_from_source() {
            match source_into {
                SourceInto::Yes => {
                    source_arg = Some(quote!(#name: impl Into<#ty>,));
                    ctor_args.push(quote!(#member: #name.into(),));
                }
                SourceInto::No => {
                    source_arg = Some(quote!(#name: #ty,));
                    ctor_args.push(quote!(#member: #name,));
                }
            }
        } else {
            #[allow(clippy::collapsible_else_if)]
            if for_macro {
                other_args.push(quote!(#name = $#name:expr,));
                ctor_args.push(quote!(#member: $#name.into(),));
            } else {
                other_args.push(quote!(#name: impl Into<#ty>,));
                ctor_args.push(quote!(#member: #name.into(),));
            }
        }
    }

    Args {
        other_args,
        source_arg,
        ctor_args,
    }
}

struct MacroArgs {
    other_args: Vec<TokenStream>,
    other_call_args: Vec<TokenStream>,
    ctor_args: Vec<TokenStream>,
}

fn resolve_variant_args_for_macro(variant: &Variant<'_>) -> MacroArgs {
    let mut other_args = Vec::new();
    let mut other_call_args = Vec::new();
    let mut ctor_args = Vec::new();

    for (i, field) in variant.fields.iter().enumerate() {
        let ty = &field.ty;
        let member = &field.member;

        let name = match &field.member {
            Member::Named(named) => named.clone(),
            Member::Unnamed(_) => format_ident!("arg_{}", i),
        };

        if field.is_backtrace() {
            let expr = if type_is_option(ty) {
                quote!(std::option::Option::Some(
                    std::backtrace::Backtrace::capture()
                ))
            } else {
                quote!(std::convert::From::from(
                    std::backtrace::Backtrace::capture()
                ))
            };
            ctor_args.push(quote!(#member: #expr,))
        } else if field.is_message() {
            ctor_args.push(quote!(#member: ::std::format!($($fmt_arg)*).into(),));
        } else {
            other_args.push(quote!(#name = $#name:expr,));
            other_call_args.push(quote!(#name));
            ctor_args.push(quote!(#member: $#name.into(),));
        }
    }

    MacroArgs {
        other_args,
        other_call_args,
        ctor_args,
    }
}

struct DeriveMeta {
    impl_type: Ident,
    backtrace: bool,
}

fn resolve_meta(input: &DeriveInput) -> Result<DeriveMeta> {
    let mut impl_type = None;
    let mut backtrace = false;

    for attr in &input.attrs {
        if attr.path().is_ident("thiserror_ext") {
            attr.parse_nested_meta(|meta| {
                if meta.path.is_ident("type") {
                    let value = meta.value()?;
                    impl_type = Some(value.parse()?);
                } else if meta.path.is_ident("backtrace") {
                    backtrace = true;
                }

                Ok(())
            })?;
        }
    }
    let impl_type = impl_type.unwrap_or_else(|| input.ident.clone());

    Ok(DeriveMeta {
        impl_type,
        backtrace,
    })
}

pub enum DeriveCtorType {
    Construct,
    ContextInto,
}

pub fn derive_box(input: &DeriveInput) -> Result<TokenStream> {
    let input_type = input.ident.clone();
    let vis = &input.vis;

    let DeriveMeta {
        impl_type,
        backtrace,
    } = resolve_meta(input)?;

    if impl_type == input_type {
        return Err(syn::Error::new_spanned(
            input,
            "should specify a different type for `Box` derive with `#[thiserror_ext(type = <type>)]`",
        ));
    }

    let backtrace_type_param = if backtrace {
        quote!(thiserror_ext::__private::MaybeBacktrace)
    } else {
        quote!(thiserror_ext::__private::NoBacktrace)
    };

    let doc = format!("The boxed type of [`{}`].", input_type);
    let generated = quote!(
        #[doc = #doc]
        #[derive(thiserror_ext::__private::thiserror::Error, Debug)]
        #[error(transparent)]
        #vis struct #impl_type(
            #[from]
            #[backtrace]
            thiserror_ext::__private::ErrorBox<
                #input_type,
                #backtrace_type_param,
            >,
        );

        // For `?` to work.
        impl<E> From<E> for #impl_type
        where
            E: Into<#input_type>,
        {
            fn from(error: E) -> Self {
                Self(thiserror_ext::__private::ErrorBox::new(error.into()))
            }
        }

        impl #impl_type {
            #[doc = "Returns the reference to the inner error."]
            #vis fn inner(&self) -> &#input_type {
                self.0.inner()
            }

            #[doc = "Consumes `self` and returns the inner error."]
            #vis fn into_inner(self) -> #input_type {
                self.0.into_inner()
            }
        }
    );

    Ok(generated)
}

pub fn derive_ctor(input: &DeriveInput, t: DeriveCtorType) -> Result<TokenStream> {
    let input_type = input.ident.clone();
    let vis = &input.vis;

    let DeriveMeta {
        impl_type,
        backtrace: _,
    } = resolve_meta(input)?;

    let input = Input::from_syn(input)?;

    let input = match input {
        Input::Struct(input) => {
            return Err(syn::Error::new_spanned(
                input.original,
                "only `enum` is supported for `thiserror_ext`",
            ))
        }
        Input::Enum(input) => input,
    };

    let mut items = Vec::new();

    for variant in input.variants {
        // Why not directly use `From`?
        if variant.from_field().is_some() {
            continue;
        }

        let variant_name = &variant.ident;

        let Args {
            other_args,
            source_arg,
            ctor_args,
        } = resolve_variant_args(
            &variant,
            match t {
                DeriveCtorType::Construct => SourceInto::Yes,
                DeriveCtorType::ContextInto => SourceInto::No,
            },
            false,
        );

        let ctor_expr = quote!(#input_type::#variant_name {
            #(#ctor_args)*
        });

        let item = match t {
            DeriveCtorType::Construct => {
                let ctor_name = format_ident!(
                    "{}",
                    big_camel_case_to_snake_case(&variant_name.to_string()),
                    span = variant.original.span()
                );
                let doc = format!("Constructs a [`{input_type}::{variant_name}`] variant.");

                quote!(
                    #[doc = #doc]
                    #vis fn #ctor_name(#source_arg #(#other_args)*) -> Self {
                        #ctor_expr.into()
                    }
                )
            }
            DeriveCtorType::ContextInto => {
                // It's implemented on `Result<T, SourceError>`, so there's must be the `source` field,
                // and we expect there's at least one argument.
                if source_arg.is_none() || other_args.is_empty() {
                    continue;
                }
                let source_ty = variant.source_field().unwrap().ty;
                let source_ty_name = get_type_string(source_ty);

                let ext_name =
                    format_ident!("Into{}", variant_name, span = variant.original.span());
                let method_name = format_ident!(
                    "into_{}",
                    big_camel_case_to_snake_case(&variant_name.to_string()),
                    span = variant.original.span()
                );
                let doc_trait = format!(
                    "Extension trait for converting [`{source_ty_name}`] \
                     into [`{input_type}::{variant_name}`] with the given context.",
                );
                let doc_method = format!(
                    "Converts [`{source_ty_name}`] \
                     into [`{input_type}::{variant_name}`] with the given context.",
                );

                quote!(
                    #[doc = #doc_trait]
                    #vis trait #ext_name {
                        type Ret;
                        #[doc = #doc_method]
                        fn #method_name(self, #(#other_args)*) -> Self::Ret;
                    }
                    impl<__T> #ext_name for std::result::Result<__T, #source_ty> {
                        type Ret = std::result::Result<__T, #impl_type>;
                        fn #method_name(self, #(#other_args)*) -> Self::Ret {
                            self.map_err(|#source_arg| #ctor_expr.into())
                        }
                    }
                    impl #ext_name for #source_ty {
                        type Ret = #impl_type;
                        fn #method_name(self, #(#other_args)*) -> Self::Ret {
                            (|#source_arg| #ctor_expr.into())(self)
                        }
                    }
                )
            }
        };

        items.push(item);
    }

    let generated = match t {
        DeriveCtorType::Construct => {
            quote!(
                #[automatically_derived]
                impl #impl_type {
                    #(#items)*
                }
            )
        }
        DeriveCtorType::ContextInto => {
            quote!(#(#items)*)
        }
    };

    Ok(generated)
}

pub fn derive_macro(input: &DeriveInput) -> Result<TokenStream> {
    let input_type = input.ident.clone();
    let vis = &input.vis;

    let DeriveMeta {
        impl_type,
        backtrace: _,
    } = resolve_meta(input)?;

    let input = Input::from_syn(input)?;

    let input = match input {
        Input::Struct(input) => {
            return Err(syn::Error::new_spanned(
                input.original,
                "only `enum` is supported for `thiserror_ext`",
            ))
        }
        Input::Enum(input) => input,
    };

    let mut items = Vec::new();

    for variant in input.variants {
        // We only care about variants with `message` field and no `source` or `from` field.
        if variant.message_field().is_none() || variant.source_field().is_some() {
            continue;
        }

        let variant_name = &variant.ident;

        let MacroArgs {
            other_args,
            other_call_args,
            ctor_args,
        } = resolve_variant_args_for_macro(&variant);

        let ctor_expr = quote!(#input_type::#variant_name {
            #(#ctor_args)*
        });

        let ctor_name = format_ident!(
            "{}",
            big_camel_case_to_snake_case(&variant_name.to_string()),
            span = variant.original.span()
        );
        let doc = format!("Constructs a [`{input_type}::{variant_name}`] variant.");

        let mut arms = Vec::new();

        let len = other_args.len();

        let message_arg = quote!($($fmt_arg:tt)*);
        let message_call_arg = quote!($($fmt_arg)*);

        for bitset in (0..((1 << len) - 1)).rev() {
            let mut args = Vec::new();
            let mut call_args = Vec::new();
            for (i, (arg, call_arg)) in (other_args.iter()).zip(other_call_args.iter()).enumerate()
            {
                if bitset & (1 << i) != 0 {
                    args.push(arg);
                    call_args.push(quote!(#call_arg = $#call_arg,));
                } else {
                    call_args.push(quote!(#call_arg = ::std::option::Option::None,));
                }
            }

            let arm = quote!(
                (#(#args)* #message_arg) => {
                    #ctor_name!(#(#call_args)* #message_call_arg)
                };
            );
            arms.push(arm);
        }

        let full = quote!(
            (#(#other_args)* #message_arg) => {{
                let res: #impl_type = (#ctor_expr).into();
                res
            }};
        );

        let item = quote!(
            #[doc = #doc]
            #[allow(unused_macros)]
            macro_rules! #ctor_name {
                #full
                #(#arms)*
            }
        );

        let mod_name = format_ident!(
            "__thiserror_ext_macros_{}_{}",
            big_camel_case_to_snake_case(&input_type.to_string()),
            ctor_name
        );

        let mod_item = quote!(
            mod #mod_name {
                #item
            }
            #vis use #mod_name::#ctor_name;
        );

        items.push(mod_item);
    }

    let generated = quote!(
        #( #items )*
    );

    Ok(generated)
}

fn big_camel_case_to_snake_case(input: &str) -> String {
    let mut output = String::new();

    for (i, c) in input.char_indices() {
        if i == 0 {
            output.push(c.to_ascii_lowercase());
        } else if c.is_uppercase() {
            output.push('_');
            output.push(c.to_ascii_lowercase());
        } else {
            output.push(c);
        }
    }

    output
}

fn type_is_option(ty: &Type) -> bool {
    type_parameter_of_option(ty).is_some()
}

fn type_parameter_of_option(ty: &Type) -> Option<&Type> {
    let path = match ty {
        Type::Path(ty) => &ty.path,
        _ => return None,
    };

    let last = path.segments.last().unwrap();
    if last.ident != "Option" {
        return None;
    }

    let bracketed = match &last.arguments {
        PathArguments::AngleBracketed(bracketed) => bracketed,
        _ => return None,
    };

    if bracketed.args.len() != 1 {
        return None;
    }

    match &bracketed.args[0] {
        GenericArgument::Type(arg) => Some(arg),
        _ => None,
    }
}

fn get_type_string(type_: &Type) -> String {
    let tokens = type_.to_token_stream();
    let mut type_string = String::new();

    for token in tokens {
        let stringified = token.to_string();
        type_string.push_str(&stringified);
    }

    type_string
}
