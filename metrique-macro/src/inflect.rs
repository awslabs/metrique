use darling::FromMeta;

use crate::{MetricsField, MetricsFieldKind, MetricsVariant, RootAttributes};

#[allow(clippy::enum_variant_names)] // "Case" is part of the name...
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, FromMeta)]
pub(crate) enum NameStyle {
    #[darling(rename = "PascalCase")]
    PascalCase,
    #[darling(rename = "snake_case")]
    SnakeCase,
    #[darling(rename = "kebab-case")]
    KebabCase,
    #[default]
    Preserve,
}

impl NameStyle {
    pub(crate) fn apply(self, name: &str) -> String {
        use inflector::Inflector;
        match self {
            NameStyle::PascalCase => name.to_pascal_case(),
            NameStyle::SnakeCase => name.to_snake_case(),
            NameStyle::Preserve => name.to_string(),
            NameStyle::KebabCase => name.to_kebab_case(),
        }
    }
}

pub fn metric_name(
    root_attrs: &RootAttributes,
    name_style: NameStyle,
    field: &impl HasInflectableName,
) -> String {
    let prefix = root_attrs.prefix.as_deref().unwrap_or_default();

    if let Some(name_override) = field.name_override() {
        return name_override.to_owned();
    };
    let base = &field.ident().to_string();
    let prefixed_base = format!("{prefix}{base}");

    name_style.apply(&prefixed_base)
}

pub trait HasInflectableName {
    fn name_override(&self) -> Option<&str>;
    fn ident(&self) -> &syn::Ident;
}

impl HasInflectableName for MetricsField {
    fn name_override(&self) -> Option<&str> {
        if let MetricsFieldKind::Field {
            name: Some(name), ..
        } = &self.attrs.kind
        {
            Some(name)
        } else {
            None
        }
    }

    fn ident(&self) -> &syn::Ident {
        &self.ident
    }
}

impl HasInflectableName for MetricsVariant {
    fn name_override(&self) -> Option<&str> {
        self.attrs.name.as_deref()
    }

    fn ident(&self) -> &syn::Ident {
        &self.ident
    }
}
