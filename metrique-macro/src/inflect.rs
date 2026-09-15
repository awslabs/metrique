use darling::FromMeta;
use metrique_core::case_convert::CaseStyle;

use crate::{MetricsField, MetricsFieldKind, RootAttributes, enums::MetricsVariant};

// Case-conversion primitives live in `metrique_core::case_convert`; re-export
// the validation helpers so existing call sites keep working.
pub(crate) use metrique_core::case_convert::{
    name_contains_dot, name_contains_uninflectables, name_ends_with_delimiter,
};

#[allow(clippy::enum_variant_names)] // "Case" is part of the name...
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, FromMeta)]
pub(crate) enum NameStyle {
    #[darling(rename = "PascalCase")]
    PascalCase,
    #[darling(rename = "snake_case")]
    SnakeCase,
    #[darling(rename = "SCREAMING_SNAKE_CASE")]
    ScreamingSnakeCase,
    #[darling(rename = "kebab-case")]
    KebabCase,
    #[default]
    Preserve,
}

impl NameStyle {
    /// All styles in index order (matching `Styles::ALL`).
    pub(crate) const ALL: [NameStyle; metrique_core::Styles::COUNT] = {
        let mut arr = [NameStyle::Preserve; metrique_core::Styles::COUNT];
        arr[metrique_core::Styles::PRESERVE.index as usize] = NameStyle::Preserve;
        arr[metrique_core::Styles::PASCAL.index as usize] = NameStyle::PascalCase;
        arr[metrique_core::Styles::SNAKE.index as usize] = NameStyle::SnakeCase;
        arr[metrique_core::Styles::KEBAB.index as usize] = NameStyle::KebabCase;
        arr[metrique_core::Styles::SCREAMING_SNAKE.index as usize] = NameStyle::ScreamingSnakeCase;
        arr
    };

    /// Map this macro-level style onto the shared [`CaseStyle`] primitive.
    fn to_case_style(self) -> CaseStyle {
        match self {
            NameStyle::PascalCase => CaseStyle::Pascal,
            NameStyle::SnakeCase => CaseStyle::Snake,
            NameStyle::ScreamingSnakeCase => CaseStyle::ScreamingSnake,
            NameStyle::KebabCase => CaseStyle::Kebab,
            NameStyle::Preserve => CaseStyle::Preserve,
        }
    }

    pub(crate) fn apply(self, name: &str) -> String {
        self.to_case_style().apply(name)
    }

    pub(crate) fn apply_prefix(self, name: &str) -> String {
        self.to_case_style().apply_prefix(name)
    }

    pub(crate) fn to_word(self) -> &'static str {
        match self {
            NameStyle::PascalCase => "Pascal",
            NameStyle::SnakeCase => "Snake",
            NameStyle::ScreamingSnakeCase => "ScreamingSnake",
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

    root_attrs
        .prefix
        .as_ref()
        .map(|p| p.apply(&base, name_style))
        .unwrap_or_else(|| name_style.apply(&base))
}

/// Inflect a field or variant name, respecting container and field attributes
/// BESIDES prefix and prefix_exact
pub fn inflect_no_prefix(root_attrs: &RootAttributes, field: &impl HasInflectableName) -> String {
    if let Some(name_override) = field.name_override() {
        return name_override.to_string();
    };

    let base = field.name();
    root_attrs.rename_all.apply(&base)
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
    fn descriptor_styles_exhaustive() {
        // Exhaustive match: adding a new NameStyle variant causes a compile error here,
        // forcing you to update NameStyle::ALL and descriptor_index().
        fn _assert_exhaustive(s: NameStyle) -> usize {
            match s {
                NameStyle::Preserve => 0,
                NameStyle::PascalCase => 1,
                NameStyle::SnakeCase => 2,
                NameStyle::KebabCase => 3,
                NameStyle::ScreamingSnakeCase => 4,
            }
        }

        // Verify that every variant has the correct index in ALL.
        for (i, &style) in NameStyle::ALL.iter().enumerate() {
            assert_eq!(
                _assert_exhaustive(style),
                i,
                "NameStyle::ALL[{i}] does not match _assert_exhaustive index"
            );
        }
    }

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
    fn test_inflect_digits_and_acronyms() {
        use std::fmt::Write as _;

        // Names containing digits or runs of uppercase letters exercise the
        // inflection library's word-boundary rules, where behavior is most
        // likely to shift if the underlying library changes. Snapshot the full
        // grid (every name in every style) so any such shift is visible in
        // review.
        let inputs = [
            "latency_p99",
            "s3_requests",
            "utf8_string",
            "XMLHttpRequest",
            "status_5xx",
            "concurrency_avg_10s",
            "e2e_us",
            "FooBARBaz",
            "request_count_10sec",
        ];
        let mut grid = String::new();
        for input in inputs {
            writeln!(grid, "{input}").unwrap();
            for style in NameStyle::ALL {
                writeln!(grid, "  {:<14} {}", style.to_word(), style.apply(input)).unwrap();
            }
            grid.push('\n');
        }
        insta::assert_snapshot!("inflect_digits_and_acronyms", grid);
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
