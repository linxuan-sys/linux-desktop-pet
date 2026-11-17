#!/usr/bin/env bash
set -euo pipefail
root="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
bin_home="${XDG_BIN_HOME:-$HOME/.local/bin}"
data_home="${XDG_DATA_HOME:-$HOME/.local/share}"
data="$data_home/pet"
binary="$bin_home/pet"
launcher="$bin_home/pet-launch"
desktop="$data_home/applications/pet.desktop"
if [[ "$(uname -m)" != '@ARCH@' ]]; then
    echo "此安装包需要 @ARCH@ 架构，当前为 $(uname -m)" >&2
    exit 1
fi
"$root/bin/pet" --help >/dev/null
if [[ "$root" == "$data" ]]; then
    echo '请从解压目录运行安装脚本。' >&2
    exit 1
fi
mkdir -p -- "$bin_home" "$data" "$(dirname -- "$desktop")"
install -m755 -- "$root/bin/pet" "$binary"
install -m755 -- "$root/pet-launch" "$launcher"
install -m644 -- "$root/RELEASE.txt" "$root/LICENSE" "$data/"
rm -f -- \
    "$bin_home/desktop-pet" \
    "$bin_home/desktop-pet-launch" \
    "$data_home/applications/desktop-pet.desktop"
exec_path=${launcher//\\/\\\\}
exec_path=${exec_path//\"/\\\"}
exec_path=${exec_path//\`/\\\`}
exec_path=${exec_path//\$/\\\$}
{
    printf '[Desktop Entry]\nType=Application\nName=Pet\n'
    printf 'Comment=Choose and configure a native Wayland desktop pet\nIcon=face-smile\n'
    printf 'Exec="%s"\n' "$exec_path"
    printf 'Terminal=false\nCategories=Game;Amusement;\nStartupNotify=false\n'
} > "$desktop"
{
    printf '#!/usr/bin/env bash\nset -euo pipefail\n'
    printf 'binary=%q\nlauncher=%q\ndata=%q\ndesktop=%q\n' \
        "$binary" "$launcher" "$data" "$desktop"
    cat <<'UNINSTALL'
rm -f -- "$binary" "$launcher" "$desktop"
rm -rf -- "$data"
echo '统一桌宠已卸载，八角色下载资源、用户配置和位置记录仍保留。'
UNINSTALL
} > "$data/uninstall.sh"
chmod 755 "$data/uninstall.sh"
if command -v update-desktop-database >/dev/null 2>&1; then
    update-desktop-database "$data_home/applications" >/dev/null 2>&1 || true
fi
printf '已安装统一桌宠。在终端运行 pet tui 打开 TUI。\n卸载：%s/uninstall.sh\n' "$data"
case ":$PATH:" in
    *":$bin_home:"*) ;;
    *) printf '提示：%s 不在当前 PATH 中；应用菜单仍可直接启动。\n' "$bin_home" ;;
esac
