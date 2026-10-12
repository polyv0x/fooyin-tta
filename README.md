# True Audio for fooyin

A fooyin plugin for lossless True Audio (`.tta`) playback. All audio decoding
runs in Rust; a small C++/Qt adapter connects it to fooyin.

Supports unencrypted TTA1 with 8/16/24-bit PCM, 1–32 channels, sample rates up to
768 kHz, sample-accurate seeking, and CRC checks. Input must be seekable.
No encoding, tag editing, or artwork support.

## Build

Requires Linux, fooyin 0.13.1+ with development headers and its CMake package,
Qt 6 development files, a C++20 compiler, CMake 3.25+, Ninja, and Rust 1.74+.

Build against your installed fooyin version. If building fooyin from source,
enable its `INSTALL_HEADERS` CMake option.

```sh
cmake -S . -B build -G Ninja -DCMAKE_BUILD_TYPE=Release
cmake --build build
```

Output: **`build/fyplugin_tta.so`**. If CMake cannot find fooyin, add
`-DFooyin_DIR=/path/to/lib/cmake/fooyin` to the first command.

## Install

Download the archive for your Linux architecture from
[Releases](https://github.com/polyv0x/fooyin-tta/releases) and extract it,
or build the plugin above. Release binaries target fooyin 0.13.1 and use
Debian 12 / Qt 6.4 as the build baseline.

1. In fooyin's plugin settings, choose **Install Plugin** and select
   the extracted `fyplugin_tta.so` (or `build/fyplugin_tta.so` after building).
2. Restart fooyin and enable **True Audio** in the input settings.

For CUE duration and waveform support, enable **True Audio** in the metadata
reader list too. After upgrading from an earlier version, reload or rescan
affected CUE files and regenerate cached waveforms.

For a system-wide installation, use `sudo cmake --install build`.

## Development

Run the Rust and plugin tests after building:

```sh
ctest --test-dir build --output-on-failure
```

The [standalone decoder](examples/decode.rs) writes raw signed 32-bit
little-endian PCM. An optional second argument sets the seek position in milliseconds.

```sh
cargo run --release --offline --example decode -- album.tta > album.s32le
```

## License

By **polyv0x**, licensed under [GPL-3.0-or-later](LICENSE).

Based on the TTA reference decoder by Alexander Djourik and Pavel Zhilin,
preserved in [Rockbox's libtta](https://github.com/Rockbox/rockbox/tree/master/lib/rbcodec/codecs/libtta).
Its [BSD 3-clause notice](LICENSES/tta-reference.txt) is retained. Include both
license files when distributing the plugin.
