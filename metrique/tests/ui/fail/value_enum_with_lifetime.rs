use metrique::unit_of_work::metrics;

#[metrics(value(string))]
enum ValueEnumWithLifetime<'a> {
    Foo,
    Bar,
}

fn main() {}
