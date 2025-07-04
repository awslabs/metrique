// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

use std::fmt;

/// An error type that describes why an [`crate::Entry`] isn't valid. This can be because it violated general contracts
/// (e.g. writing multiple values with the same name) or because it violated a format-specific contract (e.g. using a
/// reserved property name).
///
/// Unlike the happy-case path, errors are free to allocate. We won't bend over backwards to ensure fast performance in
/// reporting why entries are invalid!
#[derive(Clone)]
pub struct ValidationError(Vec<String>);

impl ValidationError {
    /// Create a build that can be used to compose multiple validation failures into a single [`ValidationError`]. Note
    /// that if no validation failures are added to the builder, [`ValidationErrorBuilder::build()`] will return
    /// [`Ok`], which is useful to track if a side-effect produced any errors.
    pub fn builder() -> ValidationErrorBuilder {
        ValidationErrorBuilder::default()
    }

    /// Extend this error with all of the validation failures recorded in `other`.
    pub fn extend(&mut self, other: Self) {
        self.0.extend(other.0);
    }

    /// Add the field `name` context for all of the validation failures reported in `self`.
    pub fn for_field(mut self, name: &str) -> Self {
        for err in self.0.iter_mut() {
            *err = format!("for `{name}`: {err}");
        }
        self
    }

    /// Record a generic validation failure with a reason string.
    pub fn invalid(reason: impl Into<String>) -> Self {
        Self(vec![reason.into()])
    }
}

impl fmt::Debug for ValidationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_list().entries(&self.0).finish()
    }
}

impl fmt::Display for ValidationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0.join(", "))
    }
}

impl std::error::Error for ValidationError {}

/// Builder to record validation failures over time and bundle them into a single [`ValidationError`] Note that if no
/// validation failures are added to the builder, [`ValidationErrorBuilder::build()`] will return [`None`], which is
/// useful to track if a side-effect produced any errors.
#[derive(Debug, Clone, Default)]
pub struct ValidationErrorBuilder(Vec<String>);

impl ValidationErrorBuilder {
    /// Returns [`Ok`] if no validation failures were recorded, otherwise [`Err`] [`ValidationError`] containing all of
    /// the recorded validation falures.
    pub fn build(self) -> Result<(), ValidationError> {
        if self.0.is_empty() {
            Ok(())
        } else {
            Err(ValidationError(self.0))
        }
    }

    // We use a $method(), $method_mut() pattern to allow for both chained builder use and for recording on a builder
    // field of a struct.

    /// Record a generic validation failure with a reason string.
    pub fn invalid(mut self, reason: impl Into<String>) -> Self {
        self.invalid_mut(reason);
        self
    }

    /// Record a generic validation failure with a reason string, but only require `&mut Self`.
    pub fn invalid_mut(&mut self, reason: impl Into<String>) -> &mut Self {
        self.0.push(reason.into());
        self
    }

    /// Extend this error with all of the validation failures recorded in `error`.
    pub fn extend(mut self, error: ValidationError) -> Self {
        self.extend_mut(error);
        self
    }

    /// Extend this error with all of the validation failures recorded in `error`, but only require `&mut Self`.
    pub fn extend_mut(&mut self, error: ValidationError) -> &mut Self {
        self.0.extend(error.0);
        self
    }
}

#[cfg(test)]
mod tests {
    use crate::validate::ValidationError;

    #[test]
    fn record_invalid() {
        assert_contains(
            &ValidationError::invalid("custom message"),
            "custom message",
        );
        assert_contains(
            &ValidationError::builder()
                .invalid("custom message")
                .build()
                .unwrap_err(),
            "custom message",
        );

        let multiple = ValidationError::builder()
            .invalid("first")
            .invalid("second")
            .build()
            .unwrap_err();
        assert_contains(&multiple, "first");
        assert_contains(&multiple, "second");
    }

    #[test]
    fn extendable() {
        let mut extended = ValidationError::invalid("first");
        extended.extend(ValidationError::invalid("second"));
        assert_contains(&extended, "first");
        assert_contains(&extended, "second");

        let extended = ValidationError::builder()
            .invalid("first")
            .extend(ValidationError::invalid("second"))
            .build()
            .unwrap_err();
        assert_contains(&extended, "first");
        assert_contains(&extended, "second");
    }

    #[test]
    fn add_field_context() {
        let contextualized = ValidationError::invalid("custom message").for_field("my_field");
        assert_contains(&contextualized, "custom message");
        assert_contains(&contextualized, "my_field");
    }

    #[test]
    fn build_returns_ok_if_no_errors() {
        assert!(ValidationError::builder().build().is_ok());
    }

    fn assert_contains(error: &ValidationError, s: &str) {
        assert!(format!("{}", error).contains(s));
        assert!(format!("{:?}", error).contains(s));
    }
}
