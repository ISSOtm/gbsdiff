# gbsdiff

This program compares two GBS files for significant differences in playback.
It requires **[`gbsplay`](https://github.com/mmitch/gbsplay) 0.0.94** or later configured with the **`iodumper` plugout**.
(If a suitable `gbsplay` is not installed on your system, use `gbsdiff --gbsplay-path path/to/custom_gbsplay`.)

This project uses Rust and Cargo, so to get started you only need to [install Rust](https://www.rust-lang.org/tools/install).
