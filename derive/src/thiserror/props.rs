use super::ast::{Enum, Field, Struct, Variant};
use syn::{Member, Type};

impl Struct<'_> {
    pub(crate) fn from_field(&self) -> Option<&Field> {
        from_field(&self.fields)
    }

    pub(crate) fn source_field(&self) -> Option<&Field> {
        source_field(&self.fields)
    }

    pub(crate) fn backtrace_field(&self) -> Option<&Field> {
        backtrace_field(&self.fields)
    }

    pub(crate) fn message_field(&self) -> Option<&Field> {
        message_field(&self.fields)
    }

    pub(crate) fn distinct_backtrace_field(&self) -> Option<&Field> {
        let backtrace_field = self.backtrace_field()?;
        distinct_backtrace_field(backtrace_field, self.from_field())
    }
}

impl Enum<'_> {
    pub(crate) fn has_source(&self) -> bool {
        self.variants
            .iter()
            .any(|variant| variant.source_field().is_some() || variant.attrs.transparent.is_some())
    }

    pub(crate) fn has_backtrace(&self) -> bool {
        self.variants
            .iter()
            .any(|variant| variant.backtrace_field().is_some())
    }

    pub(crate) fn has_display(&self) -> bool {
        self.attrs.display.is_some()
            || self.attrs.transparent.is_some()
            || self
                .variants
                .iter()
                .any(|variant| variant.attrs.display.is_some())
            || self
                .variants
                .iter()
                .all(|variant| variant.attrs.transparent.is_some())
    }
}

impl Variant<'_> {
    pub(crate) fn from_field(&self) -> Option<&Field> {
        from_field(&self.fields)
    }

    pub(crate) fn source_field(&self) -> Option<&Field> {
        source_field(&self.fields)
    }

    pub(crate) fn backtrace_field(&self) -> Option<&Field> {
        backtrace_field(&self.fields)
    }

    pub(crate) fn message_field(&self) -> Option<&Field> {
        message_field(&self.fields)
    }

    pub(crate) fn distinct_backtrace_field(&self) -> Option<&Field> {
        let backtrace_field = self.backtrace_field()?;
        distinct_backtrace_field(backtrace_field, self.from_field())
    }
}

impl Field<'_> {
    pub(crate) fn is_backtrace(&self) -> bool {
        type_is_backtrace(self.ty)
    }

    pub(crate) fn is_location(&self) -> bool {
        type_is_location(self.ty)
    }

    /// Whether this field is the `source` field but not the `from` field.
    ///
    /// See [`Variant::source_field`].
    pub(crate) fn is_non_from_source(&self) -> bool {
        if self.attrs.from.is_some() {
            false
        } else if self.attrs.source.is_some() {
            true
        } else if matches!(&self.member, Member::Named(ident) if ident == "source") {
            true
        } else {
            false
        }
    }

    /// WHether this field is the `message` field.
    pub(crate) fn is_message(&self) -> bool {
        if self.attrs.message.is_some() {
            true
        } else if matches!(
            &self.member,
            Member::Named(ident) if ident == "message"
        ) {
            true
        } else {
            false
        }
    }
}

fn from_field<'a, 'b>(fields: &'a [Field<'b>]) -> Option<&'a Field<'b>> {
    for field in fields {
        if field.attrs.from.is_some() {
            return Some(field);
        }
    }
    None
}

fn source_field<'a, 'b>(fields: &'a [Field<'b>]) -> Option<&'a Field<'b>> {
    for field in fields {
        if field.attrs.from.is_some() || field.attrs.source.is_some() {
            return Some(field);
        }
    }
    for field in fields {
        match &field.member {
            Member::Named(ident) if ident == "source" => return Some(field),
            _ => {}
        }
    }
    None
}

fn backtrace_field<'a, 'b>(fields: &'a [Field<'b>]) -> Option<&'a Field<'b>> {
    for field in fields {
        if field.attrs.backtrace.is_some() {
            return Some(field);
        }
    }
    for field in fields {
        if field.is_backtrace() {
            return Some(field);
        }
    }
    None
}

// The #[backtrace] field, if it is not the same as the #[from] field.
fn distinct_backtrace_field<'a, 'b>(
    backtrace_field: &'a Field<'b>,
    from_field: Option<&Field>,
) -> Option<&'a Field<'b>> {
    if from_field.map_or(false, |from_field| {
        from_field.member == backtrace_field.member
    }) {
        None
    } else {
        Some(backtrace_field)
    }
}

fn message_field<'a, 'b>(fields: &'a [Field<'b>]) -> Option<&'a Field<'b>> {
    for field in fields {
        if field.is_message() {
            return Some(field);
        }
    }
    None
}

fn type_is_backtrace(ty: &Type) -> bool {
    let path = match ty {
        Type::Path(ty) => &ty.path,
        _ => return false,
    };

    let last = path.segments.last().unwrap();
    last.ident == "Backtrace" && last.arguments.is_empty()
}

fn type_is_location(ty: &Type) -> bool {
    let (lifetime, elem) = match ty {
        Type::Reference(ty) => (&ty.lifetime, &ty.elem),
        _ => return false,
    };

    if let Some(lifetime) = lifetime {
        if lifetime.ident.to_string() != "static" {
            return false;
        }
    } else {
        return false;
    }

    if let Type::Path(path) = elem.as_ref() {
        let last = path.path.segments.last().unwrap();
        last.ident == "Location"
    } else {
        false
    }
}
