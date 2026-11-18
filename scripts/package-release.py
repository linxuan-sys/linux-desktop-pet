#!/usr/bin/env python3
"""Build small program-only releases; versioned animation resources download separately."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import tarfile

ROOT = Path(__file__).resolve().parents[1]
SOURCE = json.loads((ROOT / 'resources/source.json').read_text())


def package(args, target, binary_dir):
    crate_name = 'desktop-pet'
    product_name = 'pet'
    binary = binary_dir / 'pet'
    version = re.search(r'^version\s*=\s*"([^"]+)"', (ROOT / crate_name / 'Cargo.toml').read_text(), re.M)[1]
    arch = {'x86_64': 'x86_64', 'aarch64': 'aarch64'}.get(target.split('-')[0])
    if not arch:
        raise RuntimeError(f'Unsupported release architecture: {target}')
    label = f'{product_name}-{version}-{arch}'
    stage = args.output / label
    if stage.exists():
        shutil.rmtree(stage)
    (stage / 'bin').mkdir(parents=True)
    shutil.copy2(binary, stage / 'bin' / product_name)
    source = {'version': 1, 'repository': args.repository, 'tag': args.asset_tag}
    (stage / 'resources.json').write_text(json.dumps(source, indent=2) + '\n')
    for script in ('run.sh', 'install.sh', 'pet-launch'):
        text = (ROOT / 'scripts' / 'packaging' / script).read_text().replace('@ARCH@', arch)
        (stage / script).write_text(text)
        (stage / script).chmod(0o755)
    shutil.copy2(ROOT / 'LICENSE', stage / 'LICENSE')
    versions = subprocess.check_output(['readelf', '--version-info', binary], text=True)
    glibc = sorted(set(re.findall(r'GLIBC_(\d+\.\d+(?:\.\d+)?)', versions)), key=lambda s: tuple(map(int, s.split('.'))))
    minimum = glibc[-1] if glibc else 'none (static build)'
    text = f'''pet {version} — program-only release; animations download separately
Target: {target}; minimum glibc: {minimum}

Requirements: Linux / {arch}; Wayland compositor supporting zwlr_layer_shell_v1.
Tested compositor families: niri and Hyprland. X11 is unsupported.
Other compositor support is not implied by the archive format.

Portable TUI: ./run.sh tui
Per-user install (no sudo): ./install.sh
Application menu: Pet (opens the configured terminal)
Direct start: ./run.sh run kimi
Download one pet: ./run.sh download kimi
Uninstall: $XDG_DATA_HOME/{product_name}/uninstall.sh
          (default ~/.local/share/{product_name}/uninstall.sh)
No Rust compiler or preparation step is required.
Animation files are NOT included in this program package.
The TUI downloads only the selected character from the independent resource release.
Resource release: https://github.com/{args.repository}/releases/tag/{args.asset_tag}
Use d in the TUI to download; downloaded pets remain available offline.
No autostart is enabled. User configuration is preserved.

Lossless: original resolution, 120 frames/clip, 12 fps, unchanged visible RGB and alpha.
Original WebP bytes and ASSETS.json are preserved unchanged.
Needed clips are decoded/cropped once on a background thread and cached losslessly in RAM.
Default action-cache target: 32 MiB; one oversized active clip may stand alone.
The full source sheet is released after cropping; cold decode has a transient memory peak.
Existing green-screen cleanup is retained. Cleaned assets are not repaired again.
Legacy Codex alpha/seam repairs run once per decoded clip, never once per playback loop.
The premultiply configuration switch remains available.

Source code: MIT, see LICENSE.
Artwork is NOT covered by this source-code MIT license.
Artwork/animation upstream: https://github.com/hunmer/aboutme-pets
Follow the upstream artwork terms; do not assume commercial redistribution rights.
'''
    (stage / 'RELEASE.txt').write_text(text)
    sums = []
    for path in sorted(stage.rglob('*')):
        if path.is_file():
            digest = hashlib.file_digest(path.open('rb'), 'sha256').hexdigest()
            sums.append(f'{digest}  {path.relative_to(stage)}\n')
    (stage / 'SHA256SUMS').write_text(''.join(sums))
    archive = args.output / f'{label}.tar.gz'
    temporary = archive.with_name('.' + archive.name + '.tmp')
    with tarfile.open(temporary, 'w:gz', compresslevel=1) as tar:
        tar.add(stage, arcname=label)
    with temporary.open('rb') as stream:
        digest = hashlib.file_digest(stream, 'sha256').hexdigest()
        os.fsync(stream.fileno())
    temporary.replace(archive)
    archive.with_suffix(archive.suffix + '.sha256').write_text(f'{digest}  {archive.name}\n')
    print(f'RELEASE {archive} ({archive.stat().st_size / 2**20:.2f} MiB; glibc {minimum})', flush=True)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--jobs', type=int, default=1, help='parallel builds/exports; default 1 to limit peak power')
    parser.add_argument('--target', help='Rust target triple; build on the oldest supported glibc system')
    parser.add_argument('--no-build', action='store_true', help='use existing release binaries')
    parser.add_argument('--repository', default=os.environ.get('GITHUB_REPOSITORY', SOURCE['repository']))
    parser.add_argument('--asset-tag', default=SOURCE['tag'])
    parser.add_argument('--output', type=Path, default=ROOT / 'dist' / 'programs')
    args = parser.parse_args()
    if args.jobs < 1:
        parser.error('--jobs must be positive')
    target = args.target or re.search(r'^host: (.+)$', subprocess.check_output(['rustc', '-vV'], text=True), re.M)[1]
    args.output = args.output.resolve()
    args.output.mkdir(parents=True, exist_ok=True)
    if not re.fullmatch(r'[A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+', args.repository) or not re.fullmatch(r'assets-[A-Za-z0-9_.-]+', args.asset_tag):
        parser.error('invalid resource repository or assets tag')
    if not args.no_build:
        command = ['cargo', 'build', '--release', '--locked', '--jobs', str(args.jobs)]
        if args.target:
            command += ['--target', target]
        command += ['-p', 'desktop-pet']
        subprocess.run(command, check=True, cwd=ROOT)
    binary_dir = Path(os.environ.get('CARGO_TARGET_DIR', ROOT / 'target'))
    if not binary_dir.is_absolute():
        binary_dir = ROOT / binary_dir
    if args.target:
        binary_dir /= target
    binary_dir /= 'release'
    binary = binary_dir / 'pet'
    # An interrupted build can leave an existing but unusable executable.
    try:
        if target == subprocess.check_output(['rustc', '-vV'], text=True).split('host: ', 1)[1].splitlines()[0]:
            subprocess.run([str(binary), '--help'], check=True, capture_output=True)
        else:
            subprocess.run(['readelf', '-h', str(binary)], check=True, capture_output=True)
    except (OSError, subprocess.CalledProcessError) as error:
        raise RuntimeError(f'Invalid release binary {binary}; rebuild before packaging') from error
    package(args, target, binary_dir)


if __name__ == '__main__':
    main()
