// Aggregated<T> for keyless aggregation within regular metrics

use std::time::Duration;

// The wrapper type for aggregated fields (keyless only)
pub struct Aggregated<T: AggregatableEntry<Key = ()>> {
    // Single aggregated entry since Key = ()
    aggregated: Option<T::Aggregated>,
}

impl<T: AggregatableEntry<Key = ()>> Aggregated<T> {
    pub fn new() -> Self {
        Self { aggregated: None }
    }
    
    pub fn add(&mut self, entry: T) {
        match &mut self.aggregated {
            Some(agg) => agg.aggregate_into(&entry),
            None => {
                let mut agg = T::new_aggregated(());
                agg.aggregate_into(&entry);
                self.aggregated = Some(agg);
            }
        }
    }
}

// CloseValue implementation - flattens the aggregated entry
impl<T: AggregatableEntry<Key = ()>> CloseValue for Aggregated<T> 
where
    T::Aggregated: Entry,
{
    type Closed = Option<T::Aggregated>;
    
    fn close(self) -> Self::Closed {
        self.aggregated
    }
}

// Entry implementation for flattened aggregated fields
impl<T: Entry> Entry for Option<T> {
    fn write<'a>(&'a self, writer: &mut impl EntryWriter<'a>) {
        if let Some(entry) = self {
            entry.write(writer);  // Flatten directly into parent
        }
    }
    
    fn sample_group(&self) -> impl Iterator<Item = (Cow<'static, str>, Cow<'static, str>)> {
        std::iter::empty()  // Use parent's sample group
    }
}

// Usage example
#[metrics]
struct TaskResults {
    task_id: &'static str,
    
    #[metrics(flatten)]
    subtask_metrics: Aggregated<SubtaskMetrics>,
}

#[metrics(aggregate)]  // Keyless aggregation
struct SubtaskMetrics {
    // No #[metrics(key)] fields - all entries merge together
    
    #[metrics(aggregate = Counter)]
    processed_items: u64,
    
    #[metrics(aggregate = Histogram)]
    processing_time: Duration,
}

fn example_usage() {
    let mut task_results = TaskResults {
        task_id: "main_task",
        subtask_metrics: Aggregated::new(),
    };
    
    // Fan out to subtasks, all results merge together
    task_results.subtask_metrics.add(SubtaskMetrics {
        processed_items: 100,
        processing_time: Duration::from_millis(50),
    });
    
    task_results.subtask_metrics.add(SubtaskMetrics {
        processed_items: 150,  // Will sum: 100 + 150 = 250
        processing_time: Duration::from_millis(75),  // Will collect: [50ms, 75ms]
    });
    
    // When emitted (flattened):
    // {
    //   "task_id": "main_task",
    //   "processed_items": 250,
    //   "processing_time": {"Values": [50, 75], "Counts": [1, 1]}
    // }
}
