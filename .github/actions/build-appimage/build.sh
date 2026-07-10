#!/bin/sh
set -eu

if [ -z "${PROJECT_NAME:-}" ] || [ -z "${ARCH_TYPE:-}" ]; then
    echo "PROJECT_NAME and ARCH_TYPE are required" >&2
    exit 2
fi

case "$ARCH_TYPE" in
    x86_64)
        APPIMAGETOOL_SHA256=a6d71e2b6cd66f8e8d16c37ad164658985e0cf5fcaa950c90a482890cb9d13e0
        RUNTIME_SHA256=1cc49bcf1e2ccd593c379adb17c9f85a36d619088296504de95b1d06215aebbf
        ;;
    aarch64)
        APPIMAGETOOL_SHA256=1b00524ba8c6b678dc15ef88a5c25ec24def36cdfc7e3abb32ddcd068e8007fe
        RUNTIME_SHA256=7d5d772b7c32f0c84caf0a452a3072a5709027d7eac5856feb89a7a7a8881372
        ;;
    *)
        echo "Unsupported AppImage architecture: $ARCH_TYPE" >&2
        exit 2
        ;;
esac

apk add --no-cache \
    adwaita-icon-theme \
    ca-certificates \
    file \
    git \
    gtk4.0 \
    hicolor-icon-theme \
    libadwaita \
    py3-gobject3 \
    py3-pip \
    py3-setuptools \
    py3-setuptools_scm \
    py3-wheel \
    python3 \
    shared-mime-info \
    wget

git config --global --add safe.directory /source

BUILD_ROOT=$(mktemp -d)
INSTALL_ROOT="$BUILD_ROOT/install"
APPDIR="$BUILD_ROOT/AppDir"
mkdir -p "$INSTALL_ROOT" "$APPDIR/usr/bin" "$APPDIR/usr/share/metainfo" "$APPDIR/etc/ssl" "$APPDIR/etc/fonts"

python3 -m pip install \
    --break-system-packages \
    --root-user-action ignore \
    --root "$INSTALL_ROOT" \
    --prefix /usr \
    --no-deps \
    --no-build-isolation \
    /source

apk del git py3-pip py3-setuptools py3-setuptools_scm py3-wheel

cp -a /lib "$APPDIR/"
cp -a /usr/lib "$APPDIR/usr/"
cp -a /usr/share "$APPDIR/usr/"
cp -a /etc/ssl/. "$APPDIR/etc/ssl/"
cp -a /etc/fonts/. "$APPDIR/etc/fonts/"
cp -a "$INSTALL_ROOT/usr/." "$APPDIR/usr/"
cp /usr/bin/python3.12 "$APPDIR/usr/bin/"
ln -s python3.12 "$APPDIR/usr/bin/python3"

cp /yggdrasil/yggdrasil* "$APPDIR/usr/bin/"
cp /yggstack/yggstack "$APPDIR/usr/bin/"
cp /source/xdg/io.github.ergolyam.Drosophila.desktop "$APPDIR/"
cp /source/xdg/io.github.ergolyam.Drosophila.svg "$APPDIR/"
cp /source/xdg/io.github.ergolyam.Drosophila.metainfo.xml "$APPDIR/usr/share/metainfo/"
cp /action/AppRun "$APPDIR/AppRun"
chmod +x "$APPDIR/AppRun"

sed -i 's#"/usr/lib/gdk-pixbuf-2.0/2.10.0/loaders/#"#' \
    "$APPDIR/usr/lib/gdk-pixbuf-2.0/2.10.0/loaders.cache"

# The GTK cairo renderer is used to avoid loading host-specific GPU drivers.
# Mesa's LLVM/Gallium drivers are therefore dead weight and unsafe to dlopen
# across libc boundaries.
rm -rf "$APPDIR/usr/lib/gallium-pipe"
rm -f "$APPDIR/usr/lib"/libLLVM*.so* "$APPDIR/usr/lib"/libgallium-*.so

APPIMAGETOOL="$BUILD_ROOT/appimagetool-$ARCH_TYPE.AppImage"
RUNTIME="$BUILD_ROOT/runtime-$ARCH_TYPE"
wget -qO "$APPIMAGETOOL" \
    "https://github.com/AppImage/appimagetool/releases/download/continuous/appimagetool-$ARCH_TYPE.AppImage"
wget -qO "$RUNTIME" \
    "https://github.com/AppImage/type2-runtime/releases/download/continuous/runtime-$ARCH_TYPE"
printf '%s  %s\n' "$APPIMAGETOOL_SHA256" "$APPIMAGETOOL" | sha256sum -c -
printf '%s  %s\n' "$RUNTIME_SHA256" "$RUNTIME" | sha256sum -c -
chmod +x "$APPIMAGETOOL" "$RUNTIME"

ARCH="$ARCH_TYPE" APPIMAGE_EXTRACT_AND_RUN=1 "$APPIMAGETOOL" \
    --no-appstream \
    --runtime-file "$RUNTIME" \
    "$APPDIR" \
    "/output/$PROJECT_NAME-$ARCH_TYPE.AppImage"
chmod a+rx "/output/$PROJECT_NAME-$ARCH_TYPE.AppImage"
