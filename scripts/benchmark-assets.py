#!/usr/bin/env python3
"""Measure a headless resource check or playback in an isolated child process."""
import argparse
import json
import os
import resource
import subprocess
import time
from pathlib import Path

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument('binary', type=Path)
parser.add_argument('media', type=Path)
parser.add_argument('--pet', default='kimi', help='character selected by the unified pet binary')
parser.add_argument('--playback', type=int, help='real-time headless playback duration in seconds')
parser.add_argument('--warmup', type=float, default=2, help='exclude initial seconds from steady CPU sample')
args = parser.parse_args()
command = [str(args.binary.resolve())]
if args.binary.name == 'pet':
    command += ['run', args.pet]
command += ['--media-dir', str(args.media.resolve())]
command += ['--benchmark-playback', str(args.playback)] if args.playback is not None else ['--check-assets']
start = time.monotonic()
process = subprocess.Popen(command, text=True, stdout=subprocess.PIPE, stderr=subprocess.PIPE)
first = last = None
hz = os.sysconf('SC_CLK_TCK')
while process.poll() is None:
    elapsed = time.monotonic() - start
    if elapsed >= args.warmup:
        try:
            parts = Path(f'/proc/{process.pid}/stat').read_text().rsplit(')', 1)[1].split()
            status = Path(f'/proc/{process.pid}/status').read_text()
            rss = int(next(line.split()[1] for line in status.splitlines() if line.startswith('VmRSS:'))) / 1024
            sample = (elapsed, (int(parts[11]) + int(parts[12])) / hz, rss)
            if first is None:
                first = sample
            last = sample
        except (OSError, IndexError):
            pass
    time.sleep(0.05)
stdout, stderr = process.communicate()
wall = time.monotonic() - start
usage = resource.getrusage(resource.RUSAGE_CHILDREN)
steady = (last[1] - first[1]) / (last[0] - first[0]) * 100 if first and last and last[0] > first[0] else None
print(json.dumps({
    'binary': str(args.binary), 'media': str(args.media), 'returncode': process.returncode,
    'peak_rss_mib': round(usage.ru_maxrss / 1024, 3),
    'steady_rss_mib': round(last[2], 3) if last is not None else None,
    'cpu_percent_one_core_including_load': round((usage.ru_utime + usage.ru_stime) / wall * 100, 3),
    'steady_cpu_percent_one_core': round(steady, 3) if steady is not None else None,
    'wall_seconds': round(wall, 3), 'user_seconds': round(usage.ru_utime, 3),
    'system_seconds': round(usage.ru_stime, 3), 'stdout': stdout.strip(), 'stderr': stderr.strip(),
}, ensure_ascii=False, indent=2))
raise SystemExit(process.returncode)
