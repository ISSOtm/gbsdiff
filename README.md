# gbsdiff

This program compares two GBS files for significant differences in playback.

This project uses Rust and Cargo, so to get started you only need to [install Rust](https://www.rust-lang.org/tools/install).

## Caveats

- `rst` instructions are not supported.
  (They jump to $00xx instead of to an offset relative to the load address.)

## License

This project is available under the terms of the [MPL 2.0](https://mozilla.org/MPL/2.0/) or any later version.
