use metrique_writer::{AttachGlobalEntrySinkExt, FormatExt, GlobalEntrySink};
use metrique_writer_core::global_entry_sink;
use metrique_writer_core::test_stream::TestSink;
use metrique_writer_format_emf::Emf;

mod lambda_reporter_util;

#[tokio::test]
async fn test_lambda_reporter_global_sink() {
    global_entry_sink! { MySink }

    let sink = TestSink::default();
    let sink_ = sink.clone();
    let handle = MySink::attach_to_stream(
        Emf::no_validations("MyNS".to_string(), vec![vec![]]).output_to(sink_),
    );
    metrique_metricsrs::lambda_reporter::install_reporter_to_sink::<dyn metrics_024::Recorder, _, _>(
        MySink::sink(),
        handle,
    );
    lambda_reporter_util::perform_test(|| sink.dump()).await;
}
