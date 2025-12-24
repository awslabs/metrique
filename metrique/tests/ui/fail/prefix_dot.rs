use metrique::unit_of_work::metrics;

#[metrics(subfield)]
struct Subfield {
    x: u32
}

#[metrics]
struct PrefixDot {
    #[metrics(flatten, prefix = "foo.")]
    prefix_dot: Subfield,
    #[metrics(flatten, prefix = "bar.")]
    prefix_dot2: Subfield,
}

#[metrics(prefix = "baz.")]
struct PrefixDot2 {
    inner: u32
}

fn main() {}