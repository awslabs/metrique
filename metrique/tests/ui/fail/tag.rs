use metrique::unit_of_work::metrics;

// Tag not allowed on structs
#[metrics(tag(name = "operation"))]
struct Operation {
    bytes: usize,
}

// Tag not allowed on value enums
#[metrics(value(string), tag(name = "operation"))]
enum Status {
    Active,
    Inactive,
}

// Tag requires name or name_exact parameter
#[metrics(tag)]
enum MissingName {
    Read { bytes: usize },
}

// Tag cannot have both name and name_exact
#[metrics(tag(name = "op", name_exact = "op"))]
enum BothNames {
    Read { bytes: usize },
}

// Tag name cannot be empty
#[metrics(tag(name = ""))]
enum EmptyName {
    Read { bytes: usize },
}

// Tag name cannot contain spaces
#[metrics(tag(name = "my operation"))]
enum NameWithSpaces {
    Read { bytes: usize },
}

fn main() {}
