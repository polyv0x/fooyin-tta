#!/usr/bin/env bash
set -euo pipefail

if [[ $# -ne 3 ]]; then
    echo "Usage: $0 BUILD_DIR ARCH FOOYIN_VERSION" >&2
    exit 1
fi

source_dir=$(cd -- "$(dirname -- "$0")/.." && pwd)
build_dir=$(realpath "$1")
arch="$2"
fooyin_version="$3"

case "$arch" in
    x86_64|aarch64) ;;
    *) echo "Unsupported architecture: $arch" >&2; exit 1 ;;
esac
if [[ "$(uname -m)" != "$arch" ]]; then
    echo "Release packages must be built and tested on their native architecture" >&2
    exit 1
fi

cd "$source_dir"
version=$(python3 -c 'import tomllib; print(tomllib.load(open("Cargo.toml", "rb"))["package"]["version"])')
package="fooyin-tta-$version-linux-$arch-fooyin-$fooyin_version"
package_dir="$build_dir/package/$package"
mkdir -p "$package_dir/LICENSES" "$source_dir/dist"

DESTDIR="$build_dir/stage" cmake --install "$build_dir" --strip
install -m755 "$build_dir/stage/usr/lib/fooyin/plugins/fyplugin_tta.so" "$package_dir/fyplugin_tta.so"
install -m644 LICENSE README.md "$package_dir/"
install -m644 LICENSES/tta-reference.txt "$package_dir/LICENSES/"

# Exercise the stripped, installed binary that users will receive.
QT_QPA_PLATFORM=offscreen "$build_dir/tta_plugin_test" \
    "$package_dir/fyplugin_tta.so" "$source_dir/tests/fixtures"

{
    echo "True Audio $version"
    echo "Architecture: $arch"
    echo "Fooyin SDK: $fooyin_version"
    printf 'Source commit: '
    git rev-parse --verify HEAD 2>/dev/null || echo "uncommitted"
    printf 'Qt: '
    qmake6 -query QT_VERSION
    printf 'glibc: '
    getconf GNU_LIBC_VERSION
    rustc --version
    c++ --version | head -n 1
    readelf -h "$package_dir/fyplugin_tta.so"
    readelf -d "$package_dir/fyplugin_tta.so"
} > "$package_dir/BUILDINFO.txt"

tar --sort=name --owner=0 --group=0 --numeric-owner \
    -C "$build_dir/package" -czf "$source_dir/dist/$package.tar.gz" "$package"
echo "Created dist/$package.tar.gz"
