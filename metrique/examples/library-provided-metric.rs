// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

//! This example shows a "library" (in the `library` module) and
//! an example usage of said library (in the main function)

mod library {
    use std::{error::Error, fmt::Display};

    use metrique::{
        instrument::{self, Instrumented},
        timers::Timer,
        unit_of_work::metrics,
    };

    #[metrics(subfield)]
    #[derive(Default)]
    pub struct LookupBookMetrics {
        lookup_book_time: Timer,
        books_considered: usize,
        error: bool,
    }

    #[metrics(subfield)]
    #[derive(Default)]
    pub struct NumberOfBooksMetrics {
        state_length: Option<usize>,
        count_books_time: Timer,
    }

    impl Display for LookupBookError {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            match self {
                LookupBookError::NoBookForName(name) => write!(f, "NoBook for {name}"),
                LookupBookError::InvalidFormat => write!(f, "InvalidFormat"),
            }
        }
    }
    impl Error for LookupBookError {}

    #[derive(Debug)]
    pub enum LookupBookError {
        NoBookForName(String),
        InvalidFormat,
    }

    pub struct MyLib {
        state: String,
    }

    impl MyLib {
        pub fn new() -> Self {
            Self {
                state: "Book 1: Hello ....\nBook 2: Goodbye ....".to_string(),
            }
        }

        pub fn number_of_books(&self) -> Instrumented<usize, NumberOfBooksMetrics> {
            Instrumented::instrument(NumberOfBooksMetrics::default(), |metrics| {
                metrics.state_length = Some(self.state.len());
                self.state.lines().count()
            })
        }

        pub async fn lookup_book(
            &self,
            title: &str,
        ) -> instrument::Result<&str, LookupBookError, LookupBookMetrics> {
            Instrumented::instrument_async(LookupBookMetrics::default(), async |metrics| {
                let book = self
                    .state
                    .lines()
                    .flat_map(|l| {
                        metrics.books_considered += 1;
                        l.strip_prefix(title)
                    })
                    .next()
                    .ok_or_else(|| LookupBookError::NoBookForName(title.to_string()))?;
                let book = book
                    .strip_prefix(":")
                    .ok_or(LookupBookError::InvalidFormat)?;
                Ok(book)
            })
            .await
            .on_error(|_res, metrics| metrics.error = true)
        }
    }
}

use metrique::emf::Emf;
use metrique::writer::{FormatExt, sink::FlushImmediately};
use metrique::{DefaultSink, timers::Timer, unit_of_work::metrics};

#[metrics]
#[derive(Default)]
struct RequestMetrics {
    time: Timer,
    #[metrics(flatten)]
    checkout_book: Option<library::LookupBookMetrics>,
}

impl RequestMetrics {
    fn init(sink: DefaultSink) -> RequestMetricsGuard {
        Self::default().append_on_drop(sink)
    }
}

#[tokio::main]
async fn main() -> Result<(), anyhow::Error> {
    let sink = FlushImmediately::new_boxed(
        Emf::all_validations("MyApp".into(), vec![vec![]])
            .output_to_makewriter(|| std::io::stdout().lock()),
    );
    let mut request_metrics = RequestMetrics::init(sink);
    let lib = library::MyLib::new();
    // returns the result, but sets the metrics on metrics object.

    // Two patterns for dealing with `Instrumented`:

    // 1: handle downstream metrics explicitly with `into_parts`
    let (book_contents, metrics) = lib.lookup_book("Book 1").await.into_parts();
    request_metrics.checkout_book = Some(metrics);
    let book_contents = book_contents?;
    eprintln!("the book contents are {book_contents}");

    // 2: handle downstream metrics in the same statement with `write_metrics_to`
    let book_contents = lib
        .lookup_book("Book 2")
        .await
        .split_metrics_to(&mut request_metrics.checkout_book)?;
    eprintln!("the book contents are {book_contents}");

    // you can also discard the metrics if you don't care
    eprintln!(
        "total number of books: {}",
        lib.number_of_books().discard_metrics()
    );

    Ok(())
}
