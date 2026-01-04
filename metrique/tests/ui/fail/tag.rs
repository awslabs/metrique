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

// Tag requires name parameter
#[metrics(tag)]
enum MissingName {
    Read { bytes: usize },
}

fn main() {}
