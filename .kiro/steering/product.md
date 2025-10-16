# Metrique - Unit of Work Metrics Library

Metrique is a Rust crate ecosystem for collecting and exporting structured unit-of-work metrics, primarily targeting Amazon CloudWatch via EMF (Embedded Metric Format).

## Core Concept

Unlike traditional counter/gauge metrics systems, Metrique focuses on **structured metric records** - essentially structured log entries that capture complete context about operations. This enables complex aggregations and problem-specific analysis rather than just symptom observation.

## Key Features

- **Unit-of-work metrics**: Metrics tied to request/response scopes or operation lifecycles
- **Structured records**: Rich context with dimensions, properties, and multiple metric values per record
- **EMF format support**: Native Amazon CloudWatch integration
- **Composable design**: Nested metrics, subfields, and flexible aggregation patterns
- **Testing utilities**: Built-in test infrastructure for validating metric emission
- **Timing primitives**: Comprehensive timing utilities (Timer, Stopwatch, Timestamp)
- **Concurrent patterns**: Support for atomic updates, slots, and background task coordination

## Primary Use Cases

- HTTP request/response metrics in web services
- Library operation instrumentation
- Background task and workflow metrics
- Performance monitoring with rich dimensional data
- CloudWatch-based observability and alerting