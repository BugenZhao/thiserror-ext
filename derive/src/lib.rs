#![allow(rustdoc::broken_intra_doc_links)]

//! Procedural macros for `thiserror_ext`.

use expand::{DeriveCtorType, DeriveNewType};
use proc_macro::TokenStream;
use syn::{parse_macro_input, DeriveInput};

mod expand;
mod thiserror;

/// Generates constructor functions for different variants of the error type.
///
/// The arguments of the constructor functions can be any types that implement
/// [`Into`] for the corresponding fields of the variant, enabling convenient
/// construction of the error type.
///
/// # Example
///
/// ```ignore
/// #[derive(Debug, thiserror::Error, thiserror_ext::Construct)]
/// enum Error {
///     #[error("unsupported feature: {0}")]
///     UnsupportedFeature { name: String },
///
///     #[error("internal error: {0}")]
///     #[construct(skip)] // to skip generating the constructor
///     InternalError(String),
/// }
///
/// // Any type that implements `Into<String>` is accepted as the argument.
/// let _: Error = Error::unsupported_feature("foo");
/// ```
///
/// # New type
///
/// If a new type is specified with `#[thiserror_ext(newtype(..))]`, the
/// constructors will be implemented on the new type instead.
///
/// See the documentation of [`thiserror_ext::Box`] or [`thiserror_ext::Arc`]
/// for more details.
///
/// [`thiserror_ext::Box`]: derive@Box
/// [`thiserror_ext::Arc`]: derive@Arc
///
/// # Location support
///
/// If you enable the `location` feature, and then create a parameter which has a
/// type of [`&'static std::panic::Location<'static>`], then the location of the
/// constructor call will be placed into that parameter.
///
/// [`&'static std::panic::Location<'static>`]: https://doc.rust-lang.org/stable/std/panic/struct.Location.html
#[proc_macro_derive(Construct, attributes(thiserror_ext, construct))]
pub fn derive_construct(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);

    expand::derive_ctor(&input, DeriveCtorType::Construct)
        .unwrap_or_else(|err| err.to_compile_error())
        .into()
}

/// Generates extension traits for converting the external error type into the
/// the provided one, with extra context.
///
/// This can be helpful when the external error type does not provide enough
/// context for the application. `thiserror` does not allow specifying `#[from]`
/// on the error field if there're extra fields in the variant.
///
/// The extension trait is only generated when there's a field named `source`
/// or marked with `#[source]` but not `#[from]`. The rest of the fields (except
/// the backtrace) are treated as the context. Both single and multiple context
/// fields are supported.
///
/// # Example
///
/// ```ignore
/// #[derive(Debug, thiserror::Error, thiserror_ext::ContextInto)]
/// enum Error {
///     #[error("cannot parse int from `{from}`")]
///     ParseInt {
///         source: std::num::ParseIntError,
///         from: String,
///     },
///
///     #[error("cannot parse float from `{from}`")]
///     #[context_into(skip)] // to skip generating the extension
///     ParseFloat {
///         source: std::num::ParseIntError,
///         from: String,
///     },
/// }
///
/// // Specify the `from` as "foo" and convert it into `Error::ParseInt`.
/// let _: Error = "foo".parse::<i32>().unwrap_err().into_parse_int("foo");
///
/// // Can also be called on `Result<T, ExternalError>`
/// let _: Result<i32, Error> = "foo".parse().into_parse_int("foo");
///
/// // Call `into_*_with` with a closure to lazily evaluate the context.
/// let _: Result<i32, Error> = "foo".parse().into_parse_int_with(|| format!("{}", 1 + 1));
/// ```
///
/// # New type
///
/// If a new type is specified with `#[thiserror_ext(newtype(..))]`, the
/// extensions will convert the errors into the new type instead.
///
/// See the documentation of [`thiserror_ext::Box`] or [`thiserror_ext::Arc`]
/// for more details.
///
/// [`thiserror_ext::Box`]: derive@Box
/// [`thiserror_ext::Arc`]: derive@Arc
#[proc_macro_derive(ContextInto, attributes(thiserror_ext, context_into))]
pub fn derive_context_into(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);

    expand::derive_ctor(&input, DeriveCtorType::ContextInto)
        .unwrap_or_else(|err| err.to_compile_error())
        .into()
}

/// Generates macros for different variants of the error type to construct
/// it or directly bail out.
///
/// # Inline formatting
///
/// It's common to put a message string in the error variant. With this macro,
/// one can directly format the message in the macro call, instead of calling
/// [`format!`].
///
/// To mark a field as the message to be formatted, name it `message` or mark
/// it with `#[message]`. The message field can be any type that implements
/// `From<String>`.
///
/// ## Example
///
/// ```ignore
/// #[derive(Debug, thiserror::Error, thiserror_ext::Macro)]
/// enum Error {
///     #[error("internal error: {msg}")]
///     Internal { #[message] msg: Box<str> },
/// }
///
/// // Equivalent to `Error::Internal { msg: format!(..).into() }`.
/// let _: Error = internal!("{} is a bad number", 42);
///
/// // Equivalent to `return Err(Error::Internal { msg: format!(..).into() }.into())`.
/// bail_internal!("{} is a bad number", 42);
/// ```
///
/// # Extra fields
///
/// If there're extra fields along with the message field, one can specify
/// the values of them with `field = value` syntax before the message. The
/// values can be any types that implement [`Into`] for the corresponding
/// fields.
///
/// Fields can be omitted, in which case [`Default::default()`] will be used.
///
/// ## Example
///
/// ```ignore
/// #[derive(Debug, thiserror::Error, thiserror_ext::Macro)]
/// #[error("not yet implemented: {message}")]
/// struct NotYetImplemented {
///     issue: Option<i32>,
///     pr: Option<i32>,
///     message: String,
/// }
///
/// let _: Error = not_yet_implemented!(issue = 42, pr = 88, "foo");
/// let _: Error = not_yet_implemented!(issue = 42, "foo"); // pr = None
/// let _: Error = not_yet_implemented!(pr = 88, "foo");    // issue = None
/// let _: Error = not_yet_implemented!("foo");             // issue = None, pr = None
/// ```
///
/// # Visibility
///
/// There's a different rule set for the visibility of the macros. The macros
/// generated by this proc-macro are marked with `#[macro_export]` only if the
/// visibility of the error type is `pub`, otherwise they're just re-exported
/// with the same visibility as the error type and only work in the same crate.
///
/// There're some extra configurations to help to better handle the visibility,
/// specified in `#[thiserror_ext(macro(..))]`:
///
/// - `vis = ..`: use a different visibility for the macro re-export.
/// - `mangle`: mangle the macro names so that they don't conflict with other
///   macros with the same name in the crate root.
/// - `path = "crate::.."`: the path to the current module. When specified,
///   types in the generated macros will use the qualified path like
///   `$crate::foo::bar::Error`, enabling the callers to use the macros without
///   importing the error type.
///
/// # New type
///
/// If a new type is specified with `#[thiserror_ext(newtype(..))]`, the macros
/// will generate the new type instead.
///
/// See the documentation of [`thiserror_ext::Box`] or [`thiserror_ext::Arc`]
/// for more details.
///
/// [`thiserror_ext::Box`]: derive@Box
/// [`thiserror_ext::Arc`]: derive@Arc
#[proc_macro_derive(Macro, attributes(thiserror_ext, message))]
pub fn derive_macro(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);

    expand::derive_macro(&input)
        .unwrap_or_else(|err| err.to_compile_error())
        .into()
}

/// Generates a new type that wraps the original error type in a [`struct@Box`].
///
/// Specify the name of the new type with `#[thiserror_ext(newtype(name = ..))]`.
///
/// # Reduce size
///
/// The most common motivation for using this macro is to reduce the size of
/// the original error type. As a sum-type, a [`Result`] is at least as large
/// as its largest variant. Large error type may hurt the performance of a
/// function call returning a [`Result`]. With this macro, the new type always
/// has the same size as a [`struct@Box`].
///
/// On the other hand, returning an error should be an exceptional case in most
/// cases. Therefore, even though boxing the error type may lead to extra
/// allocation, it's usually acceptable.
///
/// ## Example
///
/// ```ignore
/// #[derive(Debug, thiserror::Error, thiserror_ext::Box)]
/// #[thiserror_ext(newtype(name = Error))]
/// enum ErrorKind {
///     #[error("foo")]
///     Foo,
///     #[error("io")]
///     Io(#[from] std::io::Error),
/// }
///
/// // The size of `Error` is one pointer.
/// assert_eq!(std::mem::size_of::<Error>(), std::mem::size_of::<usize>());
///
/// // Convert to `Error`, from `ErrorKind` or other types that can be converted
/// // to `ErrorKind`.
/// let error: Error = ErrorKind::Foo.into();
/// let error: Error = io_error().into();
///
/// // Get the reference or the value of the inner error.
/// let _: &ErrorKind = error.inner();
/// let _: ErrorKind = error.into_inner();
/// ```
///
/// # Backtrace
///
/// Another use case is to capture backtrace when the error is created. Without
/// a new type, one has to manually add a [`Backtrace`] field to each variant
/// of the error type. The new type allows one to capture backtrace in a single
/// place.
///
/// Specify `#[thiserror_ext(newtype(.., backtrace))]` to enable capturing
/// backtrace. The extra backtrace is captured **only if** the original error
/// type does not [`provide`] one. Typically, this should be maintained by the
/// `#[backtrace]` attribute from `thiserror`.
///
/// ## Example
///
/// ```ignore
/// # use std::backtrace::Backtrace;
/// #[derive(Debug, thiserror::Error, thiserror_ext::Box)]
/// #[thiserror_ext(newtype(name = Error, backtrace))]
/// enum ErrorKind {
///     #[error("foo")]
///     Foo,
/// }
///
/// let error: Error = ErrorKind::Foo.into();
/// let backtrace: &Backtrace = std::error::request_ref(&error).unwrap();
/// ```
///
/// [`Backtrace`]: std::backtrace::Backtrace
/// [`provide`]: std::error::Error::provide
#[proc_macro_derive(Box, attributes(thiserror_ext))]
pub fn derive_box(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);

    expand::derive_new_type(&input, DeriveNewType::Box)
        .unwrap_or_else(|err| err.to_compile_error())
        .into()
}

/// Generates a new type that wraps the original error type in an [`Arc`].
///
/// Specify the name of the new type with `#[thiserror_ext(newtype(name = ..))]`.
///
/// This is similar to [`thiserror_ext::Box`] but wraps the original error type
/// in an [`Arc`], so that it can always be cloned and shared across threads.
/// See [`thiserror_ext::Box`] for the explanation and examples.
///
/// [`Arc`]: std::sync::Arc
/// [`thiserror_ext::Box`]: derive@Box
#[proc_macro_derive(Arc, attributes(thiserror_ext))]
pub fn derive_arc(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);

    expand::derive_new_type(&input, DeriveNewType::Arc)
        .unwrap_or_else(|err| err.to_compile_error())
        .into()
}

/// Generates the [`Debug`] implementation that delegates to the [`Report`] of
/// an error.
///
/// Generally, the [`Debug`] representation of an error should not be used in
/// user-facing scenarios. However, if [`Result::unwrap`] or [`Result::expect`]
/// is called, or an error is used as [`Termination`], the standard library
/// will format the error with [`Debug`]. By delegating to [`Report`], we ensure
/// that the error is still formatted in a user-friendly way and the source
/// chain can be kept in these cases.
///
/// # Example
/// ```ignore
/// #[derive(thiserror::Error, thiserror_ext::ReportDebug)]
/// #[error("inner")]
/// struct Inner;
///
/// #[derive(thiserror::Error, thiserror_ext::ReportDebug)]
/// #[error("outer")]
/// struct Outer {
///     #[source]
///     inner: Inner,
/// }
///
/// let error = Outer { inner: Inner };
/// println!("{:?}", error);
/// ```
///
/// [`Report`]: thiserror_ext::Report
/// [`Termination`]: std::process::Termination
///
/// # New type
///
/// Since the new type delegates its [`Debug`] implementation to the original
/// error type, if the original error type derives [`ReportDebug`], the new type
/// will also behave the same.
#[proc_macro_derive(ReportDebug)]
pub fn derive_report_debug(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);

    expand::derive_report_debug(&input)
        .unwrap_or_else(|err| err.to_compile_error())
        .into()
}
