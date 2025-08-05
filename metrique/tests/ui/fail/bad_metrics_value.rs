use metrique::unit_of_work::metrics;

#[metrics(value)]
enum Foo {
}

#[metrics(value(string))]
struct Bar {
}

#[metrics(value)]
struct Baz {
    x: u32,
    y: u32,
}

#[metrics(value)]
struct Baz2 {
    #[metrics(unit = Second)]
    x: u32,
}

#[metrics(value)]
struct Baz3 {
    #[metrics(format = Foo)]
    x: u32,
}


#[metrics(value)]
struct Baz4 (
    #[metrics(name = "Bar")]
    u32,
);

#[metrics(value)]
struct Baz5 {
    #[metrics(p = q)]
    x: u32,
}

#[metrics(value)]
struct Unit; /* not supported right now */

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