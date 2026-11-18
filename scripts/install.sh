#!/usr/bin/env bash
set -euo pipefail

repo="${PET_REPOSITORY:-linxuan-sys/linux-desktop-pet}"
if [[ ! "$repo" =~ ^[A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+$ ]]; then
    echo "PET_REPOSITORY 必须是 owner/repository 格式。" >&2
    exit 1
fi

case "$(uname -m)" in
    x86_64 | amd64) arch=x86_64 ;;
    aarch64 | arm64) arch=aarch64 ;;
    *)
        echo "不支持的 CPU 架构：$(uname -m)；仅支持 x86_64 和 aarch64。" >&2
        exit 1
        ;;
esac

for command in curl tar sha256sum; do
    if ! command -v "$command" >/dev/null 2>&1; then
        echo "缺少安装依赖：$command" >&2
        exit 1
    fi
done

download_root="${TMPDIR:-/tmp}"
stage="$(mktemp -d "$download_root/pet-install.XXXXXX")"
trap 'rm -rf -- "$stage"' EXIT

base="https://github.com/$repo/releases/latest/download"
archive="pet-$arch.tar.gz"
curl --fail --location --retry 3 --connect-timeout 15 \
    --proto '=https' --tlsv1.2 \
    --output "$stage/$archive" "$base/$archive"
curl --fail --location --retry 3 --connect-timeout 15 \
    --proto '=https' --tlsv1.2 \
    --output "$stage/$archive.sha256" "$base/$archive.sha256"

(
    cd "$stage"
    sha256sum --check "$archive.sha256"
    tar -xzf "$archive"
)

mapfile -t package_roots < <(
    find "$stage" -mindepth 1 -maxdepth 1 -type d -name 'pet-*' -print
)
if [[ "${#package_roots[@]}" -ne 1 || ! -x "${package_roots[0]}/install.sh" ]]; then
    echo "下载的程序包结构无效。" >&2
    exit 1
fi

"${package_roots[0]}/install.sh"
echo "安装完成。运行 pet tui 开始选择桌宠。"
