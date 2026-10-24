#!/bin/sh
# Сборка нативного VoIP-стека libtgvoip (Telegram) под Android.
# Клонирует закреплённый libtgvoip, патчит Android.mk под
# TGVOIP_USE_CUSTOM_CRYPTO + TGVOIP_USE_CALLBACK_AUDIO_IO,
# собирает libtgvoip.so и кладёт в jniLibs.
# Использование: ./build-tgvoip.sh   (нужны git, NDK, собранный opus от build-rust.sh)
set -e
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
NDK="${ANDROID_NDK_HOME:-$ANDROID_HOME/ndk/27.0.12077973}"
if [ ! -x "$NDK/ndk-build" ]; then
  echo "NDK не найден: $NDK (ANDROID_NDK_HOME / ANDROID_HOME)" >&2
  exit 1
fi
SRC="$ROOT/android/third_party/libtgvoip"
PIN="2d2592860478e60d972b96e67ee034b8a71bb57a" # ветка tdesktop

if [ ! -d "$SRC/.git" ]; then
  rm -rf "$SRC"
  mkdir -p "$(dirname "$SRC")"
  git clone --depth 1 --branch tdesktop https://github.com/telegramdesktop/libtgvoip "$SRC"
fi
git -C "$SRC" fetch --depth 1 origin "$PIN" >/dev/null 2>&1 || true
git -C "$SRC" checkout -q "$PIN" 2>/dev/null || true

git -C "$SRC" checkout -q -- Android.mk EchoCanceller.cpp
# PROP_VALUE_MAX без OpenSL-заголовков
python3 - "$SRC/audio/AudioOutput.cpp" <<'PY2'
import sys
p = sys.argv[1]
s = open(p).read()
if not s.startswith("#include <sys/system_properties.h>"):
    s = "#include <sys/system_properties.h>\n" + s
    open(p, "w").write(s)
PY2
python3 - "$SRC/EchoCanceller.cpp" <<'PY3'
import sys
p = sys.argv[1]
s = open(p).read()
start = s.find("\tconfig.high_pass_filter.enabled = enableAEC;")
end = s.find("\taudioFrame=new webrtc::AudioFrame();")
if start > 0 and end > start:
    s = s[:start] + "\tconfig.high_pass_filter.enabled = enableAEC;\n\tconfig.gain_controller2.enabled = enableAGC;\n\tapm->ApplyConfig(config);\n\n" + s[end:]
s = s.replace("apm->GetStatistics().voice_detected.value_or(false)", "true")
open(p, "w").write(s)
PY3
cp "$ROOT/android/tgvoip/tgvoip_shim.cpp" "$SRC/tgvoip_shim.cpp"
cp "$ROOT/android/tgvoip/audio_frame_proxies_impl.cc" "$SRC/audio_frame_proxies_impl.cc"
mkdir -p "$SRC/webrtc_dsp/api"
[ -f "$SRC/webrtc_dsp/api/scoped_refptr.h" ] || \
  printf '#pragma once\n#include "rtc_base/scoped_ref_ptr.h"\n' > "$SRC/webrtc_dsp/api/scoped_refptr.h"
cp "$ROOT/android/tgvoip/audio_frame_proxies.h" \
  "$SRC/webrtc_dsp/modules/audio_processing/include/audio_frame_proxies.h"

python3 - "$SRC" <<'PY'
import sys
src = sys.argv[1]
p = src + "/Android.mk"
s = open(p).read()

s = s.replace("LOCAL_CPPFLAGS := -Wall -std=c++11", "LOCAL_CPPFLAGS := -Wall -std=c++14")
s = s.replace(
    "-DANDROID -finline-functions",
    "-DANDROID -DTGVOIP_USE_CUSTOM_CRYPTO "
    "-DTGVOIP_USE_CALLBACK_AUDIO_IO -finline-functions",
)
s = s.replace("LOCAL_MODULE := voip", "LOCAL_MODULE := tgvoip")
s = s.replace("LOCAL_EXPORT_LDLIBS := -llog -lOpenSLES", "LOCAL_LDLIBS := -llog -lOpenSLES")
s = s.replace("./logging.cpp \\", "./logging.cpp \\\n./TgVoip.cpp \\\n./tgvoip_shim.cpp \\\n./audio_frame_proxies_impl.cc \\")
s = s.replace("./audio/AudioIO.cpp \\", "./audio/AudioIO.cpp \\\n./audio/AudioIOCallback.cpp \\")



s = s.replace("$(LOCAL_PATH)/../opus/include ", "$(OPUS_PREFIX)/include $(OPUS_PREFIX)/include/opus ")
s = s.replace(" $(LOCAL_PATH)/../boringssl/include/", "")
s = s.replace(
    "LOCAL_PATH := $(call my-dir)",
    "LOCAL_PATH := $(call my-dir)\n\n"
    "include $(CLEAR_VARS)\n"
    "LOCAL_MODULE := opus\n"
    "LOCAL_SRC_FILES := $(OPUS_PREFIX)/lib/libopus.a\n"
    "LOCAL_EXPORT_C_INCLUDES := $(OPUS_PREFIX)/include/opus\n"
    "include $(PREBUILT_STATIC_LIBRARY)\n\n"
    "include $(CLEAR_VARS)",
)
s = s.replace(
    "include $(BUILD_STATIC_LIBRARY)",
    "LOCAL_STATIC_LIBRARIES := opus\ninclude $(BUILD_SHARED_LIBRARY)",
)
open(p, "w").write(s)
PY

cat > "$SRC/Application.mk" <<'EOF'
APP_PLATFORM := android-26
APP_STL := c++_static
APP_CPPFLAGS := -fexceptions -frtti
EOF

# libopus под Android (нужен libtgvoip); тот же рецепт, что в build-rust.sh.
TC="$NDK/toolchains/llvm/prebuilt/darwin-x86_64/bin"
[ "$(uname)" != "Darwin" ] && TC="$NDK/toolchains/llvm/prebuilt/linux-x86_64/bin"

build_opus() { # arch host-triple clang
  ARCH="$1"; HOST_TRIPLE="$2"; CLANG="$3"
  PREFIX="/tmp/opus-android-$ARCH/prefix"
  if [ -f "$PREFIX/lib/libopus.a" ]; then return; fi
  echo "== opus $ARCH =="
  rm -rf "/tmp/opus-android-$ARCH" && mkdir -p "/tmp/opus-android-$ARCH"
  cp -r "$HOME/.cargo/registry/src/"*/audiopus_sys-*/opus "/tmp/opus-android-$ARCH/src"
  (
    cd "/tmp/opus-android-$ARCH/src"
    [ -f configure ] || ./autogen.sh >/tmp/opus-autogen-$ARCH.log 2>&1
    CC="$TC/$CLANG" AR="$TC/llvm-ar" RANLIB="$TC/llvm-ranlib" \
      ./configure --host="$HOST_TRIPLE" --enable-static --disable-shared \
      --disable-doc --disable-extra-programs --with-pic --prefix="$PREFIX" \
      >/tmp/opus-conf-$ARCH.log 2>&1
    make -j8 >/tmp/opus-make-$ARCH.log 2>&1 && make install >/tmp/opus-inst-$ARCH.log 2>&1
  )
}

[ -f /tmp/opus-android-arm64/prefix/lib/libopus.a ] || build_opus arm64 aarch64-linux-android aarch64-linux-android26-clang
[ -f /tmp/opus-android-x86_64/prefix/lib/libopus.a ] || build_opus x86_64 x86_64-linux-android x86_64-linux-android26-clang

build_abi() {
  ARCH="$1"; ABI="$2"
  PREFIX="/tmp/opus-android-$ARCH/prefix"
  if [ ! -f "$PREFIX/lib/libopus.a" ]; then
    echo "Нет $PREFIX/lib/libopus.a — сначала ./android/build-rust.sh" >&2
    exit 1
  fi
  echo "== libtgvoip $ABI =="
  "$NDK/ndk-build" -C "$SRC" \
    NDK_PROJECT_PATH="$SRC" \
    APP_BUILD_SCRIPT="$SRC/Android.mk" \
    NDK_APPLICATION_MK="$SRC/Application.mk" \
    APP_ABI="$ABI" \
    OPUS_PREFIX="$PREFIX" \
    -j8 >/tmp/tgvoip-$ABI.log 2>&1 || {
      echo "Ошибка сборки, лог: /tmp/tgvoip-$ABI.log" >&2; tail -20 /tmp/tgvoip-$ABI.log >&2; exit 1;
    }
  mkdir -p "$ROOT/android/app/src/main/jniLibs/$ABI"
  cp "$SRC/libs/$ABI/libtgvoip.so" "$ROOT/android/app/src/main/jniLibs/$ABI/libtgvoip.so"
  ls -la "$ROOT/android/app/src/main/jniLibs/$ABI/libtgvoip.so"
}

build_abi arm64 arm64-v8a
build_abi x86_64 x86_64
echo "OK: libtgvoip.so (arm64-v8a, x86_64)"
