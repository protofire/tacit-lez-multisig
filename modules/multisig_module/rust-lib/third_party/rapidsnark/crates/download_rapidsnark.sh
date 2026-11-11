#!/bin/sh

# Exit on error
set -e

# OUT_DIR is specified by the rust build environment
if [ -z "$OUT_DIR" ]; then
    echo "OUT_DIR not specified"
    exit 1
fi
# TARGET is specified by the rust build environment
if [ -z "$TARGET" ]; then
    echo "TARGET not specified"
    exit 1
fi

# Passed by build.rs from RAPIDSNARK_VERSION at the repo root. See CONTRIBUTING.md for bump instructions.
if [ -z "$RAPIDSNARK_VERSION" ]; then
    echo "RAPIDSNARK_VERSION not specified"
    exit 1
fi
VERSION="v${RAPIDSNARK_VERSION}"
# Upstream iden3 prebuilt archives (used for macOS / iOS / Android).
IDEN3_BASE="https://github.com/iden3/rapidsnark/releases/download/$VERSION"
# The iden3 Linux archives are non-PIC and built against a newer glibc, so they
# cannot be linked into a shared library (e.g. a downstream cdylib) on the build
# hosts. The Logos fork hosts -fPIC rebuilds of the Linux archives instead.
FORK_BASE="https://github.com/logos-blockchain/logos-blockchain-rust-rapidsnark/releases/download/rapidsnark-pic-$VERSION"

BUILD_DIR="$OUT_DIR/rapidsnark"
mkdir -p "$BUILD_DIR"

arch=$(echo "$TARGET" | cut -d'-' -f1)

# Map the rust target triple to the release asset slug and its hosting base URL.
case "$TARGET" in
    x86_64-*-linux-*)                       asset="rapidsnark-linux-x86_64-pic-$VERSION"; base_url="$FORK_BASE" ;;
    aarch64-*-linux-gnu*)                   asset="rapidsnark-linux-aarch64-pic-$VERSION"; base_url="$FORK_BASE" ;;
    aarch64-linux-android)                  asset="rapidsnark-android-arm64-$VERSION";    base_url="$IDEN3_BASE" ;;
    x86_64-linux-android)                   asset="rapidsnark-android-x86_64-$VERSION";   base_url="$IDEN3_BASE" ;;
    aarch64-apple-darwin)                   asset="rapidsnark-macOS-arm64-$VERSION";      base_url="$IDEN3_BASE" ;;
    x86_64-apple-darwin)                    asset="rapidsnark-macOS-x86_64-$VERSION";      base_url="$IDEN3_BASE" ;;
    aarch64-apple-ios-sim|x86_64-apple-ios) asset="rapidsnark-iOS-Simulator-$VERSION";    base_url="$IDEN3_BASE" ;;
    aarch64-apple-ios)                      asset="rapidsnark-iOS-$VERSION";              base_url="$IDEN3_BASE" ;;
    *)
        echo "Unsupported TARGET: $TARGET (no rapidsnark $VERSION asset mapping)"
        exit 1
        ;;
esac

zip_file="$BUILD_DIR/$asset.zip"

echo "Downloading $asset.zip ..."
if ! curl -fL -o "$zip_file" "$base_url/$asset.zip"; then
    echo "Failed to download $base_url/$asset.zip"
    exit 1
fi

echo "Unzipping $zip_file ..."
if ! unzip -o "$zip_file" -d "$BUILD_DIR"; then
    echo "Failed to unzip $zip_file"
    exit 1
fi

# iden3 archives extract to "$asset/{lib,bin,include}".
# build.rs expects the libraries directly under "$BUILD_DIR/$arch", so flatten the "lib" directory into that location.
# "bin" and "include" are unused (the FFI signatures are declared in src/lib.rs, no headers are needed).
dest="$BUILD_DIR/$arch"
mkdir -p "$dest"
cp "$BUILD_DIR/$asset/lib/"* "$dest/"

echo "✅ Successfully installed rapidsnark $VERSION ($asset) into $dest"
