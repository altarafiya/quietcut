#!/bin/sh
set -eu

output_dir=$(realpath "${1:-.}")
cd "$(dirname "$0")/.."
cargo build --locked --release --bin quietcut-cli
binary="${CARGO_TARGET_DIR:-target}/release/quietcut-cli"
version=$(sed -n 's/^version = "\(.*\)"$/\1/p' Cargo.toml)
revision=$(sed -n 's/^REVISION = "\(.*\)"$/\1/p' linux/quietcut.py)
test -n "$version" && test -n "$revision"
architecture=$(dpkg --print-architecture)
libc=$(readelf --version-info "$binary" | sed -n 's/.*Name: GLIBC_\([0-9.]*\).*/\1/p' | sort -V | tail -n 1)
test -n "$libc"
staging=$(mktemp -d)
trap 'rm -rf -- "$staging"' EXIT
doc="$staging/usr/share/doc/quietcut"
install -Dm755 "$binary" "$staging/usr/bin/quietcut-cli"
install -Dm755 linux/launch.sh "$staging/usr/bin/quietcut"
for module in linux/quietcut.py linux/interface.py linux/timeline.py; do
    install -Dm644 "$module" "$staging/usr/lib/quietcut/$(basename "$module")"
done
install -Dm644 linux/quietcut.desktop "$staging/usr/share/applications/quietcut.desktop"
install -Dm644 assets/quietcut.png "$staging/usr/share/icons/hicolor/256x256/apps/quietcut.png"
install -Dm644 LICENSE "$doc/copyright"
install -Dm644 linux/README.md "$doc/README.md"
install -Dm644 THIRD-PARTY-NOTICES.md "$doc/THIRD-PARTY-NOTICES.md"
cp -R licenses "$doc/licenses"
if [ -n "${QUIETCUT_LINUX_PATCH:-}" ]; then
    gzip -n -c "$QUIETCUT_LINUX_PATCH" > "$doc/linux-port.patch.gz"
fi
size=$(du -sk "$staging" | cut -f 1)
mkdir "$staging/DEBIAN"
cat > "$staging/DEBIAN/control" <<CONTROL
Package: quietcut
Version: $version-$revision
Section: video
Priority: optional
Architecture: $architecture
Maintainer: QuietCut contributors <quietcut@localhost>
Installed-Size: $size
Depends: libc6 (>= $libc), libgcc-s1, ffmpeg, python3, python3-pyqt6, python3-pyqt6.qtmultimedia (>= 6.8), qt6-svg-plugins
Homepage: https://github.com/altarafiya/quietcut
Description: Video editor with silence removal and audio leveling
 Edit and join video clips, preview silence removal and audio leveling, and
 render the result using the QuietCut Rust engine and FFmpeg.
CONTROL
(cd "$staging" && find usr -type f -print0 | sort -z | xargs -0 md5sum > DEBIAN/md5sums)
mkdir -p "$output_dir"
dpkg-deb --root-owner-group -Zxz --build "$staging" "$output_dir/quietcut_${version}-${revision}_$architecture.deb"
