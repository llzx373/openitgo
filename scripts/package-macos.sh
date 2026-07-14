#!/usr/bin/env bash
#
# Package rustReader as a macOS .app bundle.
#
# Requires libmpv from Homebrew (`brew install mpv`); the library and its
# dependencies are bundled into Contents/Frameworks, so the resulting .app
# does not need a Homebrew mpv installation.
#
# Usage:
#   scripts/package-macos.sh [output_dir]
#
# If output_dir is omitted, the bundle is written to target/release/bundle.

set -euo pipefail

script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
project_root="$(cd "${script_dir}/.." && pwd)"

app_name="rustReader"
binary_name="rust-reader-app"
bundle_id="com.liu.rustReader"
version="0.1.0"

output_dir="${1:-${project_root}/target/release/bundle}"
app_bundle="${output_dir}/${app_name}.app"
contents_dir="${app_bundle}/Contents"
macos_dir="${contents_dir}/MacOS"
resources_dir="${contents_dir}/Resources"
frameworks_dir="${contents_dir}/Frameworks"

echo "Building release binary..."
cd "${project_root}"
cargo build --release -p "${binary_name}"

echo "Creating app bundle at ${app_bundle}..."
rm -rf "${app_bundle}"
mkdir -p "${macos_dir}" "${resources_dir}"

cp "${project_root}/target/release/${binary_name}" "${macos_dir}/${app_name}"
chmod +x "${macos_dir}/${app_name}"

if [[ -f "${project_root}/assets/icon/AppIcon.icns" ]]; then
    cp "${project_root}/assets/icon/AppIcon.icns" "${resources_dir}/AppIcon.icns"
else
    echo "Warning: assets/icon/AppIcon.icns not found; app will use the default icon."
fi

info_plist_template="${project_root}/assets/icon/Info.plist.template"
info_plist="${contents_dir}/Info.plist"

sed \
    -e "s/{{APP_NAME}}/${app_name}/g" \
    -e "s/{{BUNDLE_ID}}/${bundle_id}/g" \
    -e "s/{{VERSION}}/${version}/g" \
    "${info_plist_template}" > "${info_plist}"

plutil -lint "${info_plist}" >/dev/null

# Bundle libmpv and its Homebrew dependencies into Contents/Frameworks and
# rewrite their install names to @rpath, so the app runs on machines without
# a Homebrew mpv installation.
bundle_mpv() {
    mkdir -p "${frameworks_dir}"

    echo "Bundling libmpv and its Homebrew dependencies..."
    local mpv_prefix libmpv
    mpv_prefix="$(brew --prefix mpv)"
    libmpv="$(ls "${mpv_prefix}"/lib/libmpv.*.dylib 2>/dev/null | head -1 || true)"
    if [[ -z "${libmpv}" ]]; then
        echo "Error: libmpv not found. Run: brew install mpv" >&2
        exit 1
    fi

    # Recursively collect Homebrew dylib dependencies (excluding system libs).
    collect_deps() {
        local lib="$1"
        otool -L "${lib}" | awk 'NR>1 {print $1}' | while read -r dep; do
            case "${dep}" in
                /opt/homebrew/*|/usr/local/*)
                    if [[ ! -f "${frameworks_dir}/$(basename "${dep}")" ]]; then
                        cp "${dep}" "${frameworks_dir}/"
                        collect_deps "${dep}"
                    fi
                    ;;
            esac
        done
    }

    cp "${libmpv}" "${frameworks_dir}/"
    collect_deps "${libmpv}"

    # Rewrite install names to @rpath and add rpath to the executable.
    local dylib name
    for dylib in "${frameworks_dir}"/*.dylib; do
        name="$(basename "${dylib}")"
        install_name_tool -id "@rpath/${name}" "${dylib}" 2>/dev/null || true
        otool -L "${dylib}" | awk 'NR>1 {print $1}' | while read -r dep; do
            case "${dep}" in
                /opt/homebrew/*|/usr/local/*)
                    install_name_tool -change "${dep}" "@rpath/$(basename "${dep}")" "${dylib}" || true
                    ;;
            esac
        done
    done
    install_name_tool -add_rpath "@executable_path/../Frameworks" "${macos_dir}/${app_name}"
    # The main binary links libmpv directly; rewrite that reference too.
    otool -L "${macos_dir}/${app_name}" | awk 'NR>1 {print $1}' | while read -r dep; do
        case "${dep}" in
            /opt/homebrew/*|/usr/local/*)
                install_name_tool -change "${dep}" "@rpath/$(basename "${dep}")" "${macos_dir}/${app_name}" || true
                ;;
        esac
    done
}

bundle_mpv

echo "Signing bundled dylibs and app bundle..."
for dylib in "${frameworks_dir}"/*.dylib; do
    codesign --force --sign - "${dylib}" >/dev/null
done
codesign --force --deep --sign - "${app_bundle}" >/dev/null

echo "Done: ${app_bundle}"
