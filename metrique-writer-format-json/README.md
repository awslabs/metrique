A `metrique` [Format] for formatting `metrique` metrics as plain JSON objects.

## Usage

With default settings:
```no_run
use metrique_writer_format_json::Json;

let format = Json::new();
```
```json
{
  "timestamp": 1705312800000,
  "metrics": {
    "Latency": { "value": 42.5, "unit": "Milliseconds" },
    "Count": { "value": 10 }
  },
  "properties": {
    "Operation": "GetItem"
  }
}
```

The observation format can be customized. For example, [`Histogram`](ObservationFormat::Histogram)
mode emits parallel `values`/`counts` arrays, preserving per-observation multiplicity:
```no_run
use metrique_writer_format_json::{Json, ObservationFormat};

let format = Json::new().with_observation_format(ObservationFormat::Histogram);
```
```json
{
  "timestamp": 1705312800000,
  "metrics": {
    "Latency": { "values": [42.5], "counts": [1], "unit": "Milliseconds" },
    "Count": { "values": [10], "counts": [1] }
  },
  "properties": {
    "Operation": "GetItem"
  }
}
```

For more details on observation output shapes, see [ObservationFormat].

[Format]: https://docs.rs/metrique-writer/0.1/metrique_writer/format/trait.Format.html
[ObservationFormat]: crate::ObservationFormat

