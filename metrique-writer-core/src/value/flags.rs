// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

use std::any::Any;
use std::fmt::Debug;

/// A trait to define options that can be passed to a metric. This
/// is basically a fancier `Any`, the formatter implementation should downcast
/// this to what it cares about.
pub trait MetricOptions: Any + Debug {
    /// Try and merge this with another MetricOptions. Return None if merging is not supported
    ///
    /// This is currently an unstable detail and might change in future versions.
    /// Code that wants to work on future versions of Metrique should avoid working with it.
    #[doc(hidden)]
    fn try_merge(&self, _other: &dyn MetricOptions) -> Option<MetricFlags<'static>> {
        None
    }
}

/// Contains a set of options that describe the implementation of a metric
///
/// This is a "semi-opaque" struct. The idea is that code that just uses `fn metric`
/// rather than manipulating flags does not need to see the representation,
/// and therefore will not be broken if there is a change of representation.
///
/// Code that manipulates metric flags itself - format implementations - will
/// be broken, but that is much less code.
#[derive(Copy, Clone, Debug)]
pub struct MetricFlags<'a>(Option<&'a dyn MetricOptions>);

impl<'a> MetricFlags<'a> {
    /// Create an empty set of [MetricFlags]
    pub const fn empty() -> Self {
        Self(None)
    }

    /// Create a new MetricFlags from a set of flags.
    pub const fn upcast<T: MetricOptions>(t: &'a T) -> Self {
        MetricFlags(Some(t))
    }

    /// Merge flags. Currently only 1 set of flags is supported so this panics
    /// if there are flags set.
    pub const fn merge_assert_none(&self, other: MetricFlags<'a>) -> Self {
        assert!(self.0.is_none(), "flags can only be set once");
        other
    }

    /// Merge this set of flags with another set of flags. Panics if the flags
    /// can't be merged.
    pub fn try_merge(&self, other: MetricFlags<'a>) -> Self {
        match (self.0, other.0) {
            (None, None) => Self::empty(),
            (Some(_), None) => *self,
            (None, Some(_)) => other,
            (Some(f1), Some(f2)) => match f1.try_merge(f2) {
                Some(merged) => merged,
                None => panic!("unable to merge"),
            },
        }
    }

    /// Downcast this set of flags to a particular flag type
    pub fn downcast<T: MetricOptions>(&self) -> Option<&'a T> {
        self.0.and_then(|x| (x as &dyn Any).downcast_ref())
    }
}

#[cfg(test)]
mod test {
    use std::any::Any;

    use crate::value::MetricFlags;

    #[test]
    #[should_panic]
    fn test_try_merge_panic() {
        #[derive(Debug)]
        struct MyOptions;
        impl super::MetricOptions for MyOptions {}
        let m = MetricFlags::upcast(&MyOptions);
        m.merge_assert_none(m); // panics
    }

    #[test]
    #[should_panic]
    fn test_try_merge_matrix() {
        #[derive(Debug)]
        struct MyOptions {
            order: u32,
        }
        impl super::MetricOptions for MyOptions {
            fn try_merge(&self, other: &dyn super::MetricOptions) -> Option<MetricFlags<'static>> {
                assert!(
                    self.order == 0
                        && (other as &dyn Any)
                            .downcast_ref::<MyOptions>()
                            .unwrap()
                            .order
                            == 1
                );
                Some(&MyOptions { order: 2 });
                None
            }
        }
        let zero = MetricFlags::upcast(&MyOptions { order: 0 });
        let one = MetricFlags::upcast(&MyOptions { order: 1 });
        assert_eq!(
            MetricFlags::empty()
                .merge_assert_none(one)
                .downcast::<MyOptions>()
                .unwrap()
                .order,
            1
        );
        assert_eq!(
            one.merge_assert_none(MetricFlags::empty())
                .downcast::<MyOptions>()
                .unwrap()
                .order,
            1
        );
        assert_eq!(
            zero.merge_assert_none(one)
                .downcast::<MyOptions>()
                .unwrap()
                .order,
            2
        );
    }
}
