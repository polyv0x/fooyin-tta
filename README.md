# True Audio for fooyin

By **polyv0x**. Licensed under **GPL-3.0-or-later**.

A fooyin input plugin with a native Rust decoder for the lossless True Audio
(`.tta`) format. A small C++/Qt adapter implements fooyin's plugin interface;
header parsing, entropy decoding, prediction, channel reconstruction, CRCs,
and sample positioning run in Rust. The Rust crate has no external dependencies.
It does not call FFmpeg or libtta to decode audio (fooyin itself has its usual
library dependencies).

Supports unencrypted TTA1 PCM with 8, 16, or 24 bits per sample, 1–32 channels,
and sample rates up to 768 kHz. It provides exact sample seeking, gapless EOF,
leading ID3v2 tag skipping, and CRC verification of the header, seek table, and
every audio frame. Only one compressed frame and one decoded frame are buffered,
along with the seek table; the complete file is never loaded into memory.

## Build

Requirements: Linux, fooyin **0.13.1 or newer with its development headers and
CMake package**, Qt 6 development files, a C++20 compiler, CMake 3.25+, Ninja,
and Rust 1.74+. Tested here against fooyin 0.13.1 / Qt 6.11.2, with Rust 1.77.2 and 1.97.1.
Build against the same fooyin installation that will load the plugin: its C++
plugin ABI may change between releases. If building fooyin yourself, enable its
`INSTALL_HEADERS` CMake option.

```sh
cmake -S . -B build -G Ninja -DCMAKE_BUILD_TYPE=Release
cmake --build build
ctest --test-dir build --output-on-failure
```

The plugin is **`build/fyplugin_tta.so`**. Cargo is invoked with `--locked --offline`;
no network access or external Rust packages are needed. If CMake cannot find
fooyin, pass `-DFooyin_DIR=/path/to/lib/cmake/fooyin`.

## Load or install

On Linux, the plugin is a **`.so` shared library**. fooyin's plugin installer
accepts this file directly; no renaming or repackaging is needed.

1. Open fooyin's plugin settings and choose **Install Plugin**.
2. Select **`build/fyplugin_tta.so`** from this project's directory.
3. Restart fooyin and check that **True Audio** is enabled.

You can then add `.tta` files to your playlist and play them normally.

For a temporary run, close fooyin first and launch it with an extra plugin path:

```sh
FOOYIN_PLUGIN_PATH="$PWD/build" fooyin
```

For a system-wide installation from the command line, CMake uses the plugin
directory provided by fooyin's SDK (typically `/usr/lib/fooyin/plugins`):

```sh
sudo cmake --install build
```

To install without root, choose your own directory and add it to fooyin's search
path whenever launching the player:

```sh
cmake -S . -B build -DTTA_PLUGIN_INSTALL_DIR="$HOME/.local/lib/fooyin/plugins" \
    -DCMAKE_INSTALL_PREFIX="$HOME/.local"
cmake --build build
cmake --install build
FOOYIN_PLUGIN_PATH="$HOME/.local/lib/fooyin/plugins" fooyin
```

Restart fooyin after installing. The input backend is named **True Audio**
and declares `.tta` as a preferred extension. Keep it enabled in fooyin's input
settings. The plugin registers a decoder only; fooyin's configured metadata
readers remain responsible for tags, artwork, and library metadata.

## Tests and standalone decoding

```sh
cargo test --locked --offline
cargo clippy --offline --all-targets -- -D warnings
```

The committed fixtures are deterministic generated audio encoded by FFmpeg.
Their PCM references are checked against both the original input and FFmpeg's
decoder when generated. Tests compare every sample, exercise frame boundaries
and seeks, and reject truncated inputs, bad CRCs, invalid headers, and malformed
Rice codes. CTest also loads the actual plugin through Qt and fooyin's decoder
registry, checking file and borrowed-device input, short reads, timestamps, EOF,
errors, cancellation, and repeated initialisation.

For an optional wider comparison covering up to 32 channels and 768 kHz:

```sh
cargo build --release --offline --example decode
python3 tests/differential.py target/release/examples/decode
```

FFmpeg and Python 3 are needed only for this optional comparison and for
`python3 tests/generate_fixtures.py`. Ordinary tests use the committed fixtures.

The example decoder writes interleaved, full-scale signed 32-bit little-endian
PCM to stdout and prints stream information to stderr:

```sh
cargo run --release --offline --example decode -- album.tta > album.s32le
# Optional second argument: seek position in milliseconds
cargo run --release --offline --example decode -- album.tta 1500 > excerpt.s32le
```

## Scope and implementation

- Playback/decoding only; no TTA encoder is registered.
- Encrypted TTA, floating-point PCM, and bit depths other than 8/16/24 are rejected.
- Input must be seekable. Files and seekable `QIODevice` sources are supported;
  live/network streams and sequential archive devices are not advertised.
- Corrupt frames return an explicit error before any audio from that frame is
  delivered. CRC checks are always enabled, including normal playback.
- Output to fooyin is lossless, full-scale native-endian S32. The Rust API itself
  returns signed, right-aligned samples. Multichannel speaker positions remain
  unspecified because TTA does not store a channel mask.
- Resource limits: one million seek entries, 64 MiB per compressed frame, and
  32 million decoded channel samples per frame. Pathological files exceeding
  these limits are rejected.
- A decoder handle belongs to one thread at a time; separate handles are
  independent. The C ABI ownership and callback rules are in
  [`include/fooyin_tta.h`](include/fooyin_tta.h).

The implementation follows the original True Audio decoder by Alexander Djourik
and Pavel Zhilin, preserved under a BSD license in
[Rockbox's libtta](https://github.com/Rockbox/rockbox/tree/master/lib/rbcodec/codecs/libtta).
[`ttadec.c`](https://github.com/Rockbox/rockbox/blob/master/lib/rbcodec/codecs/libtta/ttadec.c)
describes Rice decoding and prediction;
[`filter.h`](https://github.com/Rockbox/rockbox/blob/master/lib/rbcodec/codecs/libtta/filter.h)
contains the adaptive filter. Another reference encoder/decoder is
[libtta](https://github.com/gchudov/libtta).

## License

Copyright (c) 2026 polyv0x. This plugin is licensed under the
[GNU General Public License, version 3 or later](LICENSE).

The original TTA reference code is Copyright (c) 2004 True Audio Software and
uses the BSD 3-clause license, which is
[compatible with the GNU GPL](https://www.gnu.org/licenses/license-list.html#ModifiedBSD).
The combined plugin uses GPL-3.0-or-later, matching fooyin's license, while the
original TTA attribution and BSD terms remain in
[`LICENSES/tta-reference.txt`](LICENSES/tta-reference.txt). Both notices must
accompany binary distributions; the CMake install step installs them.
