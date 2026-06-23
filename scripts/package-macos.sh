#!/usr/bin/env bash
#
# Package rustReader as a macOS .app bundle.
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

echo "Signing app bundle with ad-hoc signature..."
codesign --force --deep --sign - "${app_bundle}" >/dev/null

echo "Done: ${app_bundle}"
