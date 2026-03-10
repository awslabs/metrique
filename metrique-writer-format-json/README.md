A `metrique` [Format] for formatting `metrique` metrics as plain JSON objects.

For observation output shape, see [ObservationFormat].

## Usage

```no_run
use metrique_writer_format_json::{Json, ObservationFormat, RepeatedFormat};

let _format = Json::new().with_observation_format(ObservationFormat::Scalar(
    RepeatedFormat::TotalAndCount,
));
```

## Notes

- Per-metric dimensions and metric flags are currently ignored by this format.
- When sampling is used, prefer `ObservationFormat::Histogram` if downstream
  consumers need explicit multiplicity from JSON alone.

[Format]: https://docs.rs/metrique-writer/0.1/metrique_writer/format/trait.Format.html
[ObservationFormat]: crate::ObservationFormat

