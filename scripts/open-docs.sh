#!/usr/bin/env bash
set -e

RUSTDOCFLAGS="--cfg docsrs" cargo +nightly doc --no-deps --all-features -Zunstable-options -Zrustdoc-scrape-examples --open
