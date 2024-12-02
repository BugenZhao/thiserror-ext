use either::{for_both, Either};
use proc_macro2::TokenStream;
use quote::{format_ident, quote, ToTokens};
use syn::{
    DeriveInput, Error, GenericArgument, Ident, LitStr, Member, PathArguments, Result, Type,
    Visibility,
};

use crate::thiserror::ast::{Field, Input, Variant};

struct Args {
    other_args: Vec<TokenStream>,
    other_names: Vec<Ident>,
    other_tys: Vec<Type>,
    source_arg: Option<TokenStream>,
    ctor_args: Vec<TokenStream>,
    track_caller_attr: TokenStream,
}

enum SourceInto {
    Yes,
    No,
}

fn resolve_variant_args(variant: &Variant<'_>, source_into: SourceInto) -> Args {
    let mut other_args = Vec::new();
    let mut other_names = Vec::new();
    let mut other_tys = Vec::new();
    let mut source_arg = None;
    let mut ctor_args = Vec::new();
    let mut track_caller_attr = TokenStream::new();

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
        } else if field.is_location() {
            track_caller_attr = quote!(#[track_caller]);
            ctor_args.push(quote!(#member: std::panic::Location::caller(),));
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
            other_args.push(quote!(#name: impl Into<#ty>,));
            other_names.push(name.clone());
            other_tys.push((**ty).clone());
            ctor_args.push(quote!(#member: #name.into(),));
        }
    }

    Args {
        other_args,
        other_names,
        other_tys,
        source_arg,
        ctor_args,
        track_caller_attr,
    }
}

struct MacroArgs {
    other_args: Vec<TokenStream>,
    other_call_args: Vec<TokenStream>,
    ctor_args: Vec<TokenStream>,
}

fn resolve_args_for_macro(fields: &[Field<'_>]) -> MacroArgs {
    let mut other_args = Vec::new();
    let mut other_call_args = Vec::new();
    let mut ctor_args = Vec::new();

    for (i, field) in fields.iter().enumerate() {
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
        } else if field.is_location() {
            ctor_args.push(quote!(#member: std::panic::Location::caller(),))
        } else if field.is_message() {
            ctor_args.push(quote!(#member: ::std::format!($($fmt_arg)*).into(),));
        } else {
            other_args.push(quote!(#name = $#name:expr,));
            other_call_args.push(quote!(#name));
            ctor_args.push(quote!(#member: $#name,));
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
    nt_backtrace: bool,
    macro_mangle: bool,
    macro_path: Option<TokenStream>,
    macro_vis: Option<Visibility>,
}

fn resolve_meta(input: &DeriveInput) -> Result<DeriveMeta> {
    let mut new_type = None;
    let mut nt_backtrace = false;
    let mut macro_mangle = false;
    let mut macro_path = None;
    let mut macro_vis = None;

    for attr in &input.attrs {
        if attr.path().is_ident("thiserror_ext") {
            attr.parse_nested_meta(|meta| {
                if meta.path.is_ident("newtype") {
                    meta.parse_nested_meta(|meta| {
                        if meta.path.is_ident("name") {
                            let value = meta.value()?;
                            new_type = Some(value.parse()?);
                        } else if meta.path.is_ident("backtrace") {
                            if cfg!(feature = "backtrace") {
                                nt_backtrace = true;
                            } else {
                                return Err(Error::new_spanned(
                                    meta.path,
                                    "enable the `backtrace` feature to use `backtrace` attribute",
                                ));
                            }
                        } else {
                            return Err(Error::new_spanned(meta.path, "unknown attribute"));
                        }
                        Ok(())
                    })?;
                } else if meta.path.is_ident("macro") {
                    meta.parse_nested_meta(|meta| {
                        if meta.path.is_ident("mangle") {
                            macro_mangle = true;
                        } else if meta.path.is_ident("path") {
                            let value = meta.value()?;
                            let path: LitStr = value.parse()?;
                            let mut path = path.value();

                            if path.starts_with("crate") {
                                path.insert(0, '$');
                                if !path.ends_with("::") {
                                    path.push_str("::");
                                }
                                macro_path = Some(path.parse()?);
                            } else {
                                return Err(Error::new_spanned(
                                    meta.path,
                                    "macro path should start with `crate`",
                                ));
                            }
                        } else if meta.path.is_ident("vis") {
                            let value = meta.value()?;
                            macro_vis = Some(if let Ok(lit_str) = value.parse::<LitStr>() {
                                lit_str.parse()?
                            } else {
                                value.parse()?
                            })
                        } else {
                            return Err(Error::new_spanned(meta.path, "unknown attribute"));
                        }
                        Ok(())
                    })?;
                } else {
                    return Err(Error::new_spanned(meta.path, "unknown attribute"));
                }
                Ok(())
            })?;
        }
    }
    let impl_type = new_type.unwrap_or_else(|| input.ident.clone());

    Ok(DeriveMeta {
        impl_type,
        nt_backtrace,
        macro_mangle,
        macro_path,
        macro_vis,
    })
}

pub enum DeriveCtorType {
    Construct,
    ContextInto,
}

pub enum DeriveNewType {
    Box,
    Arc,
}

impl DeriveNewType {
    fn name(&self) -> &'static str {
        match self {
            DeriveNewType::Box => "Box",
            DeriveNewType::Arc => "Arc",
        }
    }

    fn ty_ident(&self) -> Ident {
        match self {
            DeriveNewType::Box => format_ident!("ErrorBox"),
            DeriveNewType::Arc => format_ident!("ErrorArc"),
        }
    }
}

pub fn derive_new_type(input: &DeriveInput, ty: DeriveNewType) -> Result<TokenStream> {
    let input_type = input.ident.clone();
    let vis = &input.vis;

    let DeriveMeta {
        impl_type,
        nt_backtrace: backtrace,
        ..
    } = resolve_meta(input)?;

    if impl_type == input_type {
        return Err(Error::new_spanned(
            input,
            format!("should specify a different type for `{}` derive with `#[thiserror_ext(newtype(name = <type>))]`", ty.name()),
        ));
    }

    let backtrace_type_param = if backtrace {
        quote!(thiserror_ext::__private::MaybeBacktrace)
    } else {
        quote!(thiserror_ext::__private::NoExtraBacktrace)
    };

    let doc = format!(
        "The `{}`-wrapped type of [`{}`].{}",
        ty.name(),
        input_type,
        if backtrace {
            "\n\nA backtrace is captured when the inner error doesn't provide one."
        } else {
            ""
        }
    );
    let new_type = ty.ty_ident();
    let extra_derive = match ty {
        DeriveNewType::Box => quote!(),
        DeriveNewType::Arc => quote!(Clone),
    };
    let backtrace_attr = if cfg!(feature = "backtrace") {
        quote!(#[backtrace])
    } else {
        quote!()
    };

    let into_inner = match ty {
        DeriveNewType::Box => quote!(
            #[doc = "Consumes `self` and returns the inner error."]
            #vis fn into_inner(self) -> #input_type {
                self.0.into_inner()
            }
        ),
        DeriveNewType::Arc => quote!(),
    };

    let generated = quote!(
        #[doc = #doc]
        #[derive(thiserror_ext::__private::thiserror::Error, #extra_derive)]
        #[error(transparent)]
        #vis struct #impl_type(
            #[from]
            #backtrace_attr
            thiserror_ext::__private::#new_type<
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
                Self(thiserror_ext::__private::#new_type::new(error.into()))
            }
        }

        impl std::fmt::Debug for #impl_type {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                std::fmt::Debug::fmt(&self.0, f)
            }
        }

        impl #impl_type {
            #[doc = "Returns the reference to the inner error."]
            #vis fn inner(&self) -> &#input_type {
                self.0.inner()
            }

            #into_inner
        }
    );

    Ok(generated)
}

pub fn derive_ctor(input: &DeriveInput, t: DeriveCtorType) -> Result<TokenStream> {
    let input_type = input.ident.clone();
    let vis = &input.vis;

    let DeriveMeta { impl_type, .. } = resolve_meta(input)?;

    let input = Input::from_syn(input)?;

    let input = match input {
        Input::Struct(input) => {
            return Err(Error::new_spanned(
                input.original,
                "only `enum` is supported for `Construct` and `ContextInto`",
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

        let skipped = match t {
            DeriveCtorType::Construct => variant.attrs.construct_skip.is_some(),
            DeriveCtorType::ContextInto => variant.attrs.context_into_skip.is_some(),
        };
        if skipped {
            continue;
        }

        let variant_name = &variant.ident;

        let Args {
            other_args,
            other_names,
            other_tys,
            source_arg,
            ctor_args,
            track_caller_attr,
        } = resolve_variant_args(
            &variant,
            match t {
                DeriveCtorType::Construct => SourceInto::Yes,
                DeriveCtorType::ContextInto => SourceInto::No,
            },
        );

        let ctor_expr = quote!(#input_type::#variant_name {
            #(#ctor_args)*
        });

        let item = match t {
            DeriveCtorType::Construct => {
                let ctor_name = format_ident!(
                    "{}",
                    big_camel_case_to_snake_case(&variant_name.to_string()),
                    span = variant_name.span()
                );
                let doc = format!("Constructs a [`{input_type}::{variant_name}`] variant.");

                quote!(
                    #[doc = #doc]
                    #track_caller_attr
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

                let ext_name = format_ident!("Into{}", variant_name, span = variant_name.span());

                let doc_trait = format!(
                    "Extension trait for converting [`{source_ty_name}`] \
                     into [`{input_type}::{variant_name}`] with the given context.",
                );

                let method_sig = {
                    let name = format_ident!(
                        "into_{}",
                        big_camel_case_to_snake_case(&variant_name.to_string()),
                        span = variant_name.span()
                    );
                    let doc = format!(
                        "Converts [`{source_ty_name}`] \
                         into [`{input_type}::{variant_name}`] with the given context.",
                    );

                    quote!(
                        #[doc = #doc]
                        fn #name(self, #(#other_args)*) -> Self::Ret
                    )
                };

                let method_with_sig = {
                    let name = format_ident!(
                        "into_{}_with",
                        big_camel_case_to_snake_case(&variant_name.to_string()),
                        span = variant_name.span()
                    );
                    let doc = format!(
                        "Converts [`{source_ty_name}`] \
                         into [`{input_type}::{variant_name}`] with the context returned by the given function.",
                    );

                    let ret_tys: Vec<_> = other_names
                        .iter()
                        .map(|name| format_ident!("__{}", name.to_string().to_uppercase()))
                        .collect();
                    let ret_ty_bounds: Vec<_> = ret_tys
                        .iter()
                        .zip(other_tys.iter())
                        .map(|(ret_ty, ty)| quote!(#ret_ty: Into<#ty>))
                        .collect();

                    quote!(
                        #[doc = #doc]
                        fn #name<__F, #( #ret_tys, )*>(
                            self,
                            f: __F,
                        ) -> Self::Ret
                        where
                            __F: FnOnce() -> (#( #ret_tys ),*),
                            #( #ret_ty_bounds, )*
                    )
                };

                quote!(
                    #[doc = #doc_trait]
                    #vis trait #ext_name {
                        type Ret;
                        #method_sig;
                        #method_with_sig;
                    }
                    impl #ext_name for #source_ty {
                        type Ret = #impl_type;
                        #method_sig {
                            (move |#source_arg| #ctor_expr.into())(self)
                        }
                        #method_with_sig {
                            let (#( #other_names ),*) = f();
                            (move |#source_arg| #ctor_expr.into())(self)
                        }
                    }
                    impl<__T> #ext_name for std::result::Result<__T, #source_ty> {
                        type Ret = std::result::Result<__T, #impl_type>;
                        #method_sig {
                            self.map_err(move |#source_arg| #ctor_expr.into())
                        }
                        #method_with_sig {
                            self.map_err(move |#source_arg| {
                                let (#( #other_names ),*) = f();
                                #ctor_expr.into()
                            })
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

pub fn derive_macro_inner(input: &DeriveInput, bail: bool) -> Result<TokenStream> {
    let DeriveMeta {
        impl_type,
        macro_mangle,
        macro_path,
        macro_vis,
        ..
    } = resolve_meta(input)?;

    let input_type = input.ident.clone();
    let vis = macro_vis.unwrap_or_else(|| input.vis.clone());
    let input = Input::from_syn(input)?;

    let variants = match input {
        Input::Struct(input) => vec![Either::Left(input)],
        Input::Enum(input) => input.variants.into_iter().map(Either::Right).collect(),
    };

    let mut items = Vec::new();

    for variant in variants {
        // We only care about variants with `message` field.
        if for_both!(&variant, v => v.message_field()).is_none() {
            continue;
        }

        let variant_name = match &variant {
            Either::Left(_s) => quote!(#impl_type), // newtype name
            Either::Right(v) => v.ident.to_token_stream(),
        };
        let ctor_path = match &variant {
            Either::Left(_s) => quote!(#macro_path #input_type),
            Either::Right(_v) => quote!(#macro_path #input_type::#variant_name),
        };

        let fields = for_both!(&variant, v => &v.fields);

        let MacroArgs {
            other_args,
            other_call_args,
            ctor_args,
        } = resolve_args_for_macro(fields);

        let ctor_expr = quote!(#ctor_path {
            #(#ctor_args)*
        });

        let bail_prefix = if bail { "bail_" } else { "" };
        let bail_suffix = if bail { "__bail" } else { "" };

        let ctor_span = for_both!(&variant, v => v.ident.span());

        let export_name = format_ident!(
            "{}{}",
            bail_prefix,
            big_camel_case_to_snake_case(&variant_name.to_string()),
            span = ctor_span,
        );
        let mangled_name = if macro_mangle {
            format_ident!(
                "__thiserror_ext_macro__{}__{}{}",
                big_camel_case_to_snake_case(&input_type.to_string()),
                big_camel_case_to_snake_case(&variant_name.to_string()),
                bail_suffix,
                span = ctor_span,
            )
        } else {
            export_name.clone()
        };

        let bail_doc = if bail { " and bails out" } else { "" };
        let doc = match &variant {
            Either::Left(_s) => {
                format!("Constructs a [`{input_type}`]{bail_doc}.")
            }
            Either::Right(_v) => {
                format!("Constructs a [`{input_type}::{variant_name}`] variant{bail_doc}.")
            }
        };

        let mut arms = Vec::new();

        let len = other_args.len();

        let message_arg = quote!($($fmt_arg:tt)*);
        let message_call_arg = quote!($($fmt_arg)*);

        for bitset in (0..(1 << len)).rev() {
            let mut args = Vec::new();
            let mut call_args = Vec::new();
            for (i, (arg, call_arg)) in (other_args.iter()).zip(other_call_args.iter()).enumerate()
            {
                if bitset & (1 << i) != 0 {
                    args.push(arg);
                    call_args.push(quote!(#call_arg = $#call_arg.into(),));
                } else {
                    call_args.push(quote!(#call_arg = ::std::default::Default::default(),));
                }
            }

            let arm = quote!(
                (#(#args)* #message_arg) => {
                    #export_name!(@ #(#call_args)* #message_call_arg)
                };
            );
            arms.push(arm);
        }

        let full_inner = if bail {
            quote!({
                let res: #macro_path #impl_type = (#ctor_expr).into();
                return ::std::result::Result::Err(res.into());
            })
        } else {
            quote!({
                let res: #macro_path #impl_type = (#ctor_expr).into();
                res
            })
        };

        let full = quote!(
            () => { // empty macro call
                #export_name!("")
            };
            (@ #(#other_args)* #message_arg) => {
                #full_inner
            };
        );

        let macro_export = if let Visibility::Public(_) = &vis {
            quote!(#[macro_export])
        } else {
            quote!()
        };

        let item = quote!(
            #[doc = #doc]
            #[allow(unused_macros)]
            #macro_export
            macro_rules! #mangled_name {
                #full
                #(#arms)*
            }

            #[allow(unused_imports)]
            #vis use #mangled_name as #export_name;
        );

        items.push(item);
    }

    let generated = quote!(
        #( #items )*
    );

    Ok(generated)
}

pub fn derive_macro(input: &DeriveInput) -> Result<TokenStream> {
    let ctor = derive_macro_inner(input, false)?;
    let bail = derive_macro_inner(input, true)?;

    let generated = quote!(
        #ctor
        #bail
    );

    Ok(generated)
}

pub fn derive_report_debug(input: &DeriveInput) -> Result<TokenStream> {
    let input_type = input.ident.clone();

    // 1. Delegate to `Debug` impl as the backtrace provided by the error
    //    could be different than where panic happens.
    // 2. Passthrough the `alternate` flag.
    let generated = quote!(
        impl ::std::fmt::Debug for #input_type {
            fn fmt(&self, f: &mut ::std::fmt::Formatter) -> ::std::fmt::Result {
                use ::thiserror_ext::AsReport;
                ::std::fmt::Debug::fmt(&self.as_report(), f)
            }
        }
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
