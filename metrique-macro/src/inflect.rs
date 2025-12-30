use darling::FromMeta;

use crate::{MetricsField, MetricsFieldKind, Prefix, RootAttributes, enums::MetricsVariant};

pub(crate) fn name_contains_uninflectables(name: &str) -> Option<char> {
    name.chars()
        .find(|&c| !c.is_alphanumeric() && c != '_' && c != '-')
}

pub(crate) fn name_ends_with_delimiter(name: &str) -> bool {
    let last = name.chars().last();
    last == Some('_') || last == Some('-')
}

// `.` is currently used in production, make it a warning instead of an error
pub(crate) fn name_contains_dot(name: &str) -> bool {
    name.contains('.')
}

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

    pub(crate) fn apply_prefix(self, name: &str) -> String {
        use inflector::Inflector;
        match self {
            NameStyle::PascalCase => name.to_pascal_case(),
            NameStyle::SnakeCase => {
                let mut res = name.to_snake_case();
                if !res.ends_with("_") {
                    res.push('_');
                }
                res
            }
            NameStyle::Preserve => name.to_string(),
            NameStyle::KebabCase => {
                let mut res = name.to_kebab_case();
                if !res.ends_with("-") {
                    res.push('-');
                }
                res
            }
        }
    }

    pub(crate) fn to_word(self) -> &'static str {
        match self {
            NameStyle::PascalCase => "Pascal",
            NameStyle::SnakeCase => "Snake",
            NameStyle::Preserve => "Preserve",
            NameStyle::KebabCase => "Kebab",
        }
    }
}

pub fn metric_name(
    root_attrs: &RootAttributes,
    name_style: NameStyle,
    field: &impl HasInflectableName,
) -> String {
    if let Some(name_override) = field.name_override() {
        return name_override.to_owned();
    };

    let base = field.name();

    match &root_attrs.prefix {
        Some(Prefix::Exact(exact_prefix)) => {
            // If the prefix is exact, do not inflect the prefix
            format!("{exact_prefix}{}", name_style.apply(&base))
        }
        Some(Prefix::Inflectable { prefix, .. }) => {
            let prefixed_base = format!("{prefix}{base}");
            name_style.apply(&prefixed_base)
        }
        None => name_style.apply(&base),
    }
}

pub trait HasInflectableName {
    fn name_override(&self) -> Option<&str>;
    fn name(&self) -> String;
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

    fn name(&self) -> String {
        self.name.clone().expect("name must be set here")
    }
}

impl HasInflectableName for MetricsVariant {
    fn name_override(&self) -> Option<&str> {
        self.attrs.name.as_deref()
    }

    fn name(&self) -> String {
        self.ident.to_string()
    }
}

#[cfg(test)]
mod test {
    use super::name_contains_uninflectables;
    use crate::{NameStyle, inflect::name_ends_with_delimiter};

    #[test]
    fn test_inflect_prefix() {
        let kebab = NameStyle::KebabCase;
        let snake = NameStyle::SnakeCase;
        let pascal = NameStyle::PascalCase;

        assert_eq!(kebab.apply_prefix("Foo"), "foo-");
        assert_eq!(kebab.apply_prefix("foo"), "foo-");
        assert_eq!(kebab.apply_prefix("foo_"), "foo-");
        assert_eq!(kebab.apply_prefix("foo-"), "foo-");
        assert_eq!(kebab.apply_prefix("foo."), "foo-");

        assert_eq!(snake.apply_prefix("Foo"), "foo_");
        assert_eq!(snake.apply_prefix("foo"), "foo_");
        assert_eq!(snake.apply_prefix("foo_"), "foo_");
        assert_eq!(snake.apply_prefix("foo-"), "foo_");
        assert_eq!(snake.apply_prefix("foo."), "foo_");

        assert_eq!(pascal.apply_prefix("Foo"), "Foo");
        assert_eq!(pascal.apply_prefix("foo"), "Foo");
        assert_eq!(pascal.apply_prefix("foo_"), "Foo");
        assert_eq!(pascal.apply_prefix("foo-"), "Foo");
        assert_eq!(pascal.apply_prefix("foo."), "Foo");
    }

    #[test]
    fn test_uninflectables() {
        assert_eq!(name_contains_uninflectables("foo-bar_baz"), None);
        assert_eq!(name_contains_uninflectables("foo:bar"), Some(':'));
        assert_eq!(name_contains_uninflectables("foo.bar"), Some('.'));
    }

    #[test]
    fn test_delimiter() {
        assert!(name_ends_with_delimiter("foo-"));
        assert!(name_ends_with_delimiter("foo_"));
        assert!(!name_ends_with_delimiter("foo."));
        assert!(!name_ends_with_delimiter("foo"));
    }
}
