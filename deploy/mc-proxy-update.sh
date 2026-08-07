#!/bin/sh
# 仅接受 GitHub Release 的 Ubuntu 24.04 x86_64 安装包，下载后先运行 --version，
# 再原子替换二进制。失败会回滚上一份并重启旧服务。
set -eu

repo_api='https://api.github.com/repos/baiyun1123/YvLink/releases/latest'
asset_name='YvLink-ubuntu-24.04-x86_64.tar.gz'
install_dir='/opt/mc-proxy'
binary_path="$install_dir/mc-proxy"
state_dir='/var/lib/mc-proxy'
status_path="$state_dir/update-status.json"
mkdir -p "$state_dir"
work_dir=$(mktemp -d "$state_dir/update.XXXXXX")
cleanup() { rm -rf "$work_dir"; }
trap cleanup EXIT HUP INT TERM

write_status() {
  update_state=$1
  update_message=$2
  UPDATE_STATE=$update_state UPDATE_MESSAGE=$update_message python3 - <<'PY' >"$status_path.tmp"
import json, os
print(json.dumps({
    "state": os.environ["UPDATE_STATE"],
    "message": os.environ["UPDATE_MESSAGE"],
}, ensure_ascii=False))
PY
  mv "$status_path.tmp" "$status_path"
}

# Exit 0 only when the GitHub release is newer than the installed stable
# x.y.z version. This prevents a delayed or manually selected older release
# from downgrading a server that was compiled or updated more recently.
release_is_newer() {
  python3 - "$1" "$2" <<'PY'
import re, sys

def parse(value):
    match = re.fullmatch(r"(\d+)\.(\d+)\.(\d+)", value)
    if not match:
        raise ValueError(value)
    return tuple(map(int, match.groups()))

try:
    installed, available = parse(sys.argv[1]), parse(sys.argv[2])
except ValueError:
    raise SystemExit(2)
raise SystemExit(0 if available > installed else 1)
PY
}

if ! systemctl is-active --quiet mc-proxy.service; then
  write_status 'deferred' 'mc-proxy 服务未运行，跳过自动升级。'
  exit 0
fi

curl --fail --silent --show-error --location --retry 3 --connect-timeout 15 "$repo_api" >"$work_dir/release.json"
python3 - "$work_dir/release.json" "$asset_name" <<'PY' >"$work_dir/asset.txt"
import json, sys
release = json.load(open(sys.argv[1], encoding='utf-8'))
asset = next((a for a in release['assets'] if a['name'] == sys.argv[2]), None)
if not asset:
    raise SystemExit('release does not include required Ubuntu 24.04 x86_64 package')
print(release['tag_name'])
print(asset['browser_download_url'])
PY
release_tag=$(sed -n '1p' "$work_dir/asset.txt")
asset_url=$(sed -n '2p' "$work_dir/asset.txt")
current_version=$("$binary_path" --version 2>/dev/null || true)
expected_version=${release_tag#v}
if [ "$current_version" = "$expected_version" ]; then
  write_status 'up-to-date' "当前已是 $release_tag。"
  exit 0
fi
if release_is_newer "$current_version" "$expected_version"; then
  :
else
  compare_result=$?
  if [ "$compare_result" -eq 1 ]; then
    write_status 'up-to-date' "当前版本 $current_version 不低于 GitHub 的 $release_tag，跳过降级。"
    exit 0
  else
    write_status 'failed' "无法比较当前版本 $current_version 与 GitHub 版本 $release_tag。"
    exit 1
  fi
fi

write_status 'downloading' "正在下载 $release_tag。"
curl --fail --silent --show-error --location --retry 3 --connect-timeout 15 "$asset_url" >"$work_dir/release.tar.gz"
mkdir "$work_dir/package"
tar -xzf "$work_dir/release.tar.gz" -C "$work_dir/package"
candidate=$(find "$work_dir/package" -type f -name mc-proxy -print -quit)
if [ -z "$candidate" ] || [ ! -x "$candidate" ]; then
  write_status 'failed' '发布包中没有可执行 mc-proxy。'
  exit 1
fi
if [ "$($candidate --version)" != "$expected_version" ]; then
  write_status 'failed' '发布包版本与 GitHub 标签不一致。'
  exit 1
fi

install -m 0755 "$candidate" "$binary_path.next"
cp -p "$binary_path" "$binary_path.previous"
mv -f "$binary_path.next" "$binary_path"
if systemctl restart mc-proxy.service && sleep 2 && systemctl is-active --quiet mc-proxy.service; then
  write_status 'updated' "已升级到 $release_tag。"
  exit 0
fi

mv -f "$binary_path.previous" "$binary_path"
systemctl restart mc-proxy.service || true
write_status 'rolled-back' "升级到 $release_tag 失败，已回滚上一版本。"
exit 1
