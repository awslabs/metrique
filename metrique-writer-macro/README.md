This crate defines the `#[derive(Entry)]` macro. You should be using the
`metrique-writer` crate, rather than referring to this crate directly.

The difference between the [`Entry`] and the [`MetriqueEntry`] macros,
is that the [`Entry`] macro generates paths that depend on `metrique-writer`,
and the [`MetriqueEntry`] macro generates paths that depend on `metrique`. The
corresponding crates re-export the right macro to ensure you only need
to have a single dependency in your `Cargo.toml`.