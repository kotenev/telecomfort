#!/bin/sh
# Сборка Rust-ядра (cdylib) для Android: opus static + .so + strip + jniLibs.
# Использование: ./build-rust.sh [debug|release]   (по умолчанию release)
# Требует: rustup stable + targets aarch64/x86_64-linux-android, Android NDK.
set -e
MODE="${1:-release}"
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
NDK="${ANDROID_NDK_HOME:-$ANDROID_HOME/ndk/27.0.12077973}"
if [ ! -d "$NDK" ]; then
  echo "NDK не найден: $NDK (ANDROID_NDK_HOME / ANDROID_HOME)" >&2
  exit 1
fi
TC="$NDK/toolchains/llvm/prebuilt/darwin-x86_64/bin"
if [ "$(uname)" != "Darwin" ]; then
  TC="$NDK/toolchains/llvm/prebuilt/linux-x86_64/bin"
fi
if [ -x "$HOME/.rustup/toolchains/stable-aarch64-apple-darwin/bin/rustc" ]; then
  export RUSTC="$HOME/.rustup/toolchains/stable-aarch64-apple-darwin/bin/rustc"
fi

# 1. Статический libopus под каждую arch (audiopus-sys не умеет кроссить autotools).
build_opus() { # arch host-triple clang
  ARCH="$1"; HOST_TRIPLE="$2"; CLANG="$3"
  PREFIX="/tmp/opus-android-$ARCH/prefix"
  if [ -f "$PREFIX/lib/libopus.a" ]; then echo "opus $ARCH уже собран"; return; fi
  echo "== opus $ARCH =="
  rm -rf "/tmp/opus-android-$ARCH" && mkdir -p "/tmp/opus-android-$ARCH"
  cp -r "$HOME/.cargo/registry/src/"*/audiopus_sys-*/opus "/tmp/opus-android-$ARCH/src"
  cd "/tmp/opus-android-$ARCH/src"
  [ -f configure ] || ./autogen.sh >/tmp/opus-autogen-$ARCH.log 2>&1
  CC="$TC/$CLANG" AR="$TC/llvm-ar" RANLIB="$TC/llvm-ranlib" \
    ./configure --host="$HOST_TRIPLE" --enable-static --disable-shared \
    --disable-doc --disable-extra-programs --with-pic --prefix="$PREFIX" \
    >/tmp/opus-conf-$ARCH.log 2>&1
  make -j8 >/tmp/opus-make-$ARCH.log 2>&1 && make install >/tmp/opus-inst-$ARCH.log 2>&1
  cd "$ROOT"
}

build_opus arm64 aarch64-linux-android aarch64-linux-android26-clang
build_opus x86_64 x86_64-linux-android x86_64-linux-android26-clang

# 2. Rust .so (профиль + strip).
CARGO_MODE=""
[ "$MODE" = "release" ] && CARGO_MODE="--release"
for ARCH in aarch64 x86_64; do
  TARGET="${ARCH}-linux-android"
  UPPER=$(echo "$ARCH" | tr 'a-z' 'A-Z')
  export "CARGO_TARGET_${UPPER}_LINUX_ANDROID_LINKER=$TC/${ARCH}-linux-android26-clang"
  export "CARGO_TARGET_${UPPER}_LINUX_ANDROID_AR=$TC/llvm-ar"
  echo "== libtelecomfort $TARGET ($MODE) =="
  SHORT="$ARCH"; [ "$ARCH" = "aarch64" ] && SHORT="arm64"
  CCENV="CC_${ARCH}_linux_android"
  ARENV="AR_${ARCH}_linux_android"
  env OPUS_STATIC=1 OPUS_LIB_DIR="/tmp/opus-android-$SHORT/prefix/lib" \
    "$CCENV=$TC/${ARCH}-linux-android26-clang" \
    "$ARENV=$TC/llvm-ar" \
    cargo build --lib --target "$TARGET" $CARGO_MODE
  SO="$ROOT/target/$TARGET/${MODE}/libtelecomfort.so"
  "$TC/llvm-strip" "$SO" 2>/dev/null || true
  ABI="arm64-v8a"; [ "$ARCH" = "x86_64" ] && ABI="x86_64"
  mkdir -p "$ROOT/android/app/src/main/jniLibs/$ABI"
  cp "$SO" "$ROOT/android/app/src/main/jniLibs/$ABI/"
  ls -la "$ROOT/android/app/src/main/jniLibs/$ABI/libtelecomfort.so"
done
echo OK
