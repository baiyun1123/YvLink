#!/bin/sh
# 下载 ViaLite 最新稳定版并严格比对官方 checksums.txt。可选第一个参数固定版本，例如 v0.3.0。
set -eu

requested_version=${1:-latest}
target_dir='/opt/mc-proxy/vialite'
machine=$(uname -m)
case "$machine" in
  x86_64) asset_name='vialite-linux-amd64' ;;
  aarch64|arm64) asset_name='vialite-linux-arm64' ;;
  *) echo "不支持的 CPU 架构: $machine" >&2; exit 1 ;;
esac
release_api='https://api.github.com/repos/minekube/vialite/releases'
work_dir=$(mktemp -d)
cleanup() { rm -rf "$work_dir"; }
trap cleanup EXIT HUP INT TERM

if [ "$requested_version" = latest ]; then
  curl --fail --silent --show-error --location "$release_api/latest" >"$work_dir/release.json"
else
  curl --fail --silent --show-error --location "$release_api/tags/$requested_version" >"$work_dir/release.json"
fi
python3 - "$work_dir/release.json" "$asset_name" <<'PY' >"$work_dir/urls.txt"
import json, sys
release = json.load(open(sys.argv[1], encoding='utf-8'))
assets = {item['name']: item['browser_download_url'] for item in release['assets']}
if sys.argv[2] not in assets or 'checksums.txt' not in assets:
    raise SystemExit('release is missing binary or checksums.txt')
print(release['tag_name'])
print(assets[sys.argv[2]])
print(assets['checksums.txt'])
PY
version=$(sed -n '1p' "$work_dir/urls.txt")
binary_url=$(sed -n '2p' "$work_dir/urls.txt")
checksums_url=$(sed -n '3p' "$work_dir/urls.txt")
curl --fail --silent --show-error --location "$binary_url" >"$work_dir/$asset_name"
curl --fail --silent --show-error --location "$checksums_url" >"$work_dir/checksums.txt"
(cd "$work_dir" && grep "  $asset_name$" checksums.txt | sha256sum -c -)
# ViaLite is executed by the non-root proxy service, so the directory needs
# to be traversable by that service account. The binary itself stays owned by
# root and is never written by the running proxy.
install -d -m 0755 "$target_dir"
install -m 0755 "$work_dir/$asset_name" "$target_dir/vialite"
printf 'ViaLite %s 已安装到 %s\n' "$version" "$target_dir/vialite"
