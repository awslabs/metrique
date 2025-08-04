use metrique::unit_of_work::metrics;

#[metrics(value)]
enum Foo {
}

#[metrics(value(string))]
struct Bar {
}

#[metrics(value)]
struct Baz {
}

#[metrics(value(string))]
enum Bad {
    #[metrics(p="q")]
    Bad,
}

#[metrics(value(string), subfield)]
enum Multi {
    X
}

#[metrics(value(string), emf::dimension_sets = [["X"]])]
enum Multi2 {
    X
}

fn main() {
    let _ = Bad::Bad; // check that the enum is still generated
}