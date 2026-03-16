A `metrique` [Format] for formatting `metrique` metrics as plain JSON objects.

## Usage

```no_run
use metrique_writer_format_json::Json;

let format = Json::new();
```

Each entry is serialized as a single JSON line:
```json
{
  "timestamp": 1705312800000,
  "metrics": {
    "Latency": { "value": 42.5, "unit": "Milliseconds" },
    "Count": { "value": 10 },
    "BackendLatency": { "value": { "total": 150, "count": 3 }, "unit": "Milliseconds" },
    "ResponseTimes": { "values": [1, 2, 3], "unit": "Milliseconds" }
  },
  "properties": {
    "Operation": "GetItem"
  }
}
```

Single observations are emitted as `"value": X`, multiple as `"values": [...]`.
Repeated observations (e.g. from histogram buckets) are emitted as
`{"total": ..., "count": ...}`.

[Format]: https://docs.rs/metrique-writer/latest/metrique_writer/format/trait.Format.html
