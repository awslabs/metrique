use metrique::unit_of_work::metrics;

#[metrics(value(string))]
enum ValueEnumWithGeneric<T> {
    Foo,
    Bar,
}

fn main() {}
