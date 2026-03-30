use metrique::CloseValue;
use metrique_macro::metrics;

#[derive(Debug)]
#[metrics(value(string))]
pub(crate) enum Operation {
    Flush,
    ProcessSegment,
}

#[metrics(rename_all = "PascalCase")]
#[derive(Debug)]
pub(crate) struct FlushMetrics {
    pub operation: Operation,
    pub count: usize,
}

fn main() {
    let metrics = FlushMetrics {
        operation: Operation::Flush,
        count: 10,
    };

    let _ = format!("{:?}", metrics.operation);
    let _ = format!("{:?}", Operation::Flush);
}
