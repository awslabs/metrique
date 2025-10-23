use metrique::writer::Entry;
use metrique::{CloseValue, RootEntry, unit_of_work::metrics};
use metrique::{InflectableEntry, NameStyle, ServiceMetrics};
use metrique_core::CloseEntry;
use metrique_writer::{AttachGlobalEntrySinkExt, BoxEntry, FormatExt, GlobalEntrySink};
use metrique_writer_format_emf::Emf;

#[metrics(prefix = "handler_a_")]
#[derive(Default)]
struct HandlerAMetrics {
    request_count: usize,
}

#[metrics(prefix = "hanlder_b_")]
#[derive(Default)]
struct HandlerBMetrics {
    request_count: usize,
    number_of_rs_in_strawberry: usize,
}

#[derive(Default)]
struct MultiEntry {
    entries: Vec<Box<dyn FnOnce() -> BoxEntry + Send + Sync>>,
}

struct ClosedMultiEntry {
    entries: Vec<BoxEntry>,
}

impl<Ns: NameStyle> InflectableEntry<Ns> for ClosedMultiEntry {
    fn write<'a>(&'a self, w: &mut impl metrique_writer::EntryWriter<'a>) {
        for e in &self.entries {
            e.write(w)
        }
    }
}

impl CloseValue for MultiEntry {
    type Closed = ClosedMultiEntry;

    fn close(self) -> Self::Closed {
        ClosedMultiEntry {
            entries: self.entries.into_iter().map(|f| f()).collect(),
        }
    }
}

#[metrics]
struct RequestMetrics {
    #[metrics(flatten)]
    rows: MultiEntry,
}

impl MultiEntry {
    fn insert<T: Send + 'static, E: CloseEntry<Closed = T> + Send + Sync + 'static>(
        &mut self,
        e: E,
    ) {
        let f = Box::new(move || BoxEntry::new(RootEntry::new(e.close())));
        self.entries.push(f)
    }
}

fn handler_a() -> HandlerAMetrics {
    HandlerAMetrics { request_count: 1 }
}

fn handler_b() -> HandlerBMetrics {
    HandlerBMetrics {
        request_count: 4,
        number_of_rs_in_strawberry: 65,
    }
}

fn main() {
    let _handle = ServiceMetrics::attach_to_stream(
        Emf::all_validations("DynamicMetrics".to_string(), vec![vec![]])
            .output_to_makewriter(std::io::stdout),
    );
    let mut main_entry = RequestMetrics {
        rows: Default::default(),
    }
    .append_on_drop(ServiceMetrics::sink());
    main_entry.rows.insert(handler_a());
    main_entry.rows.insert(handler_b());
    drop(main_entry)
}
