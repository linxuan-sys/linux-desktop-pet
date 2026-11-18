#!/usr/bin/env python3
"""Package existing WebP bytes and processing markers as independent resource releases."""
import argparse
import gzip
import hashlib
import json
from pathlib import Path
import re
import tarfile

ROOT = Path(__file__).resolve().parents[1]
PETS = ('claude', 'dsh', 'codex', 'grok', 'qwen', 'gemini', 'glm', 'kimi')


def digest(path):
    with path.open('rb') as stream:
        return hashlib.file_digest(stream, 'sha256').hexdigest()


def package(pet, tag, output):
    name = pet + '-girl'
    media = ROOT / name / 'media'
    files = sorted((media / 'anim').glob('*.webp'))
    if not files:
        raise RuntimeError(f'No animations for {name}')
    if (media / 'ASSETS.json').exists():
        if json.loads((media / 'ASSETS.json').read_text()).get('status') not in ('raw', 'cleaned'):
            raise RuntimeError(f'Unsupported processing status for {name}')
        files.append(media / 'ASSETS.json')
    manifest_files = {str(path.relative_to(media)): {'bytes': path.stat().st_size, 'sha256': digest(path)} for path in files}
    archive = output / f'{name}-assets.tar.gz'
    temporary = output / ('.' + archive.name + '.tmp')
    with temporary.open('wb') as raw:
        with gzip.GzipFile(fileobj=raw, mode='wb', filename='', mtime=0, compresslevel=1) as compressed:
            with tarfile.open(fileobj=compressed, mode='w|', format=tarfile.PAX_FORMAT) as tar:
                for path in files:
                    info = tar.gettarinfo(str(path), arcname=str(path.relative_to(media)))
                    info.uid = info.gid = info.mtime = 0
                    info.uname = info.gname = ''
                    info.mode = 0o644
                    with path.open('rb') as stream:
                        tar.addfile(info, stream)
    temporary.replace(archive)
    manifest = {'version': 1, 'pet': name, 'tag': tag, 'archive_bytes': archive.stat().st_size, 'archive_sha256': digest(archive), 'files': manifest_files}
    (output / f'{name}.manifest.json').write_text(json.dumps(manifest, ensure_ascii=False, indent=2) + '\n')
    print(f'ASSETS {name}: {len(files)} files; {archive.stat().st_size / 2**20:.2f} MiB; source bytes and ASSETS.json unchanged', flush=True)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('pet', choices=(*PETS, 'all'), nargs='?', default='all')
    parser.add_argument('--tag', default='assets-v1')
    parser.add_argument('--output', type=Path)
    args = parser.parse_args()
    if not re.fullmatch(r'assets-[A-Za-z0-9_.-]+', args.tag):
        parser.error('resource tags must start with assets- and contain only letters, digits, dot, dash and underscore')
    output = (args.output or ROOT / 'dist' / 'assets' / args.tag).resolve()
    output.mkdir(parents=True, exist_ok=True)
    for pet in PETS if args.pet == 'all' else (args.pet,):
        package(pet, args.tag, output)
    checks = [f'{digest(p)}  {p.name}\n' for p in sorted(output.glob('*')) if p.is_file() and p.name != 'SHA256SUMS']
    (output / 'SHA256SUMS').write_text(''.join(checks))


if __name__ == '__main__':
    main()
