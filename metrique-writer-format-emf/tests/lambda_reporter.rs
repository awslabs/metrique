// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

use metrique_writer_core::test_stream::TestSink;
use metrique_writer_format_emf::Emf;

mod lambda_reporter_util;

#[tokio::test]
async fn test_lambda_reporter() {
    let sink = TestSink::default();
    let sink_ = sink.clone();
    metrique_metricsrs::lambda_reporter::install_reporter_to_writer::<
        dyn metrics_024::Recorder,
        _,
        _,
        _,
    >(
        Emf::no_validations("MyNS".to_string(), vec![vec![]]),
        move || sink_.clone(),
    );
    lambda_reporter_util::perform_test(|| sink.dump()).await;
}
