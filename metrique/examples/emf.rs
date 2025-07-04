// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

use std::time::SystemTime;

use metrique::unit_of_work::metrics;
use metrique_writer::{
    AttachGlobalEntrySinkExt, Entry, EntryIoStreamExt, FormatExt, GlobalEntrySink,
    sink::global_entry_sink,
};
use metrique_writer_format_emf::Emf;

global_entry_sink! { ServiceMetrics }

#[metrics(
    emf::dimension_sets = [
        ["Status", "Operation"],
        ["Operation"]
    ]
)]
struct RequestMetrics {
    #[metrics(timestamp)]
    timestamp: SystemTime,
    operation: &'static str,
    status: &'static str,
    number_of_ducks: usize,
}

impl RequestMetrics {
    fn init(operation: &'static str) -> RequestMetricsGuard {
        RequestMetrics {
            timestamp: SystemTime::now(),
            operation,
            status: "INCOMPLETE",
            number_of_ducks: 0,
        }
        .append_on_drop(ServiceMetrics::sink())
    }
}

#[derive(Entry)]
#[entry(rename_all = "PascalCase")]
struct Globals {
    region: String,
}

fn main() {
    let globals = Globals {
        // Generally, this is usually sourced from CLI args or the environment
        region: "us-east-1".to_string(),
    };

    let _handle = ServiceMetrics::attach_to_stream(
        Emf::all_validations("MyApp".to_string(), vec![vec!["region".to_string()]])
            .output_to(std::io::stdout())
            // All entries will contain `region` as a dimension
            .merge_globals(globals),
    );
    let mut request = RequestMetrics::init("CountDucks");
    request.number_of_ducks += 10;
    request.status = "SUCCESS";

    /*
    {"_aws":{"CloudWatchMetrics":[{"Namespace":"MyApp","Dimensions":[["region","Status","Operation"],["region","Operation"]],"Metrics":[{"Name":"NumberOfDucks"}]}],"Timestamp":1744396038208},"NumberOfDucks":10,"Region":"us-east-1","Operation":"CountDucks","Status":"Ok"}
    */
}

mod rotation_file_destination {
    use std::path::PathBuf;

    use metrique_writer::{AttachGlobalEntrySinkExt, FormatExt, sink::AttachHandle};
    use metrique_writer_format_emf::Emf;
    use tracing_appender::rolling::{RollingFileAppender, Rotation};

    #[allow(dead_code)]
    fn initialize_metrics(service_log_dir: PathBuf) -> AttachHandle {
        super::ServiceMetrics::attach_to_stream(
            Emf::builder("Ns".to_string(), vec![vec![]])
                .build()
                .output_to_makewriter(RollingFileAppender::new(
                    Rotation::MINUTELY,
                    &service_log_dir,
                    "service_log.log",
                )),
        )
    }
}

mod tcp_destination {
    use std::net::SocketAddr;

    use metrique_writer::FormatExt;
    use metrique_writer_format_emf::Emf;

    #[allow(dead_code)]
    async fn initialize_metrics() {
        let emf_port = 1234;
        let addr = SocketAddr::from(([127, 0, 0, 1], emf_port));
        // Use tokio to establish the socket to avoid blocking the runtime, then convert it to std
        let tcp_connection = tokio::net::TcpStream::connect(addr)
            .await
            .expect("failed to connect to Firelens TCP port")
            .into_std()
            .unwrap();
        let _stream = Emf::all_validations("QPersonalizationService".to_string(), vec![vec![]])
            .output_to(tcp_connection);
    }
}
