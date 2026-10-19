# Fuzzing

Coverage-guided fuzzing with [`cargo-fuzz`](https://rust-fuzz.github.io/book/cargo-fuzz.html) (needs a
nightly toolchain):

```sh
cargo install cargo-fuzz
cargo +nightly fuzz run svg          # or: dxf project controller_line gcode_program mcp_message text_layout glyph_font inkstitch_font font_index font_listing ai_reply command
```

Every target calls a function in `crates/fuzzkit`, which are also driven by deterministic mutation
tests that run on stable with `cargo test -p laserfoundry-fuzzkit` (part of `cargo xtask ci`). Those
tests already found two bugs: a panic in the simulated controller on a non-ASCII G-code word, and
G-code lines containing GRBL real-time bytes (soft reset, `?`, `!`, `~`), which the streamer now
refuses before sending anything.
