"""Offline installer regression: mirror routing and fail-closed checksums."""
import hashlib
import json
import os
from pathlib import Path
import platform
import shutil
import subprocess
import tarfile
import tempfile

ROOT = Path(__file__).resolve().parent.parent
with tempfile.TemporaryDirectory(prefix='wocli-distribution-test-') as temporary:
    root = Path(temporary)
    target_platform = ('macos-' + ('arm64' if platform.machine() == 'arm64' else 'x86_64')) if platform.system() == 'Darwin' else 'linux-x86_64'
    name = f'wocli-music-0.3.4-{target_platform}'
    bundle = root / name
    bundle.mkdir()
    binary = bundle / 'wocli'
    binary.write_text('#!/bin/sh\necho "wocli-music 0.3.4"\n')
    binary.chmod(0o755)
    archive = root / f'{name}.tar.gz'
    with tarfile.open(archive, 'w:gz') as tar:
        tar.add(bundle, arcname=name)
    sums = root / 'SHA256SUMS'
    valid = hashlib.sha256(archive.read_bytes()).hexdigest() + f'  {archive.name}\n'
    sums.write_text(valid)
    fakebin = root / 'fakebin'
    fakebin.mkdir()
    # Only network transport is replaced. The real installer extracts, verifies and runs.
    curl = fakebin / 'curl'
    curl.write_text('''#!/usr/bin/env python3
import json, os, pathlib, shutil, sys
args = sys.argv[1:]
url = next(a for a in args if a.startswith('https://'))
root = pathlib.Path(os.environ['WOCLI_INSTALL_FIXTURE'])
with (root / 'requests').open('a') as log: log.write(url + '\\n')
if '/releases/latest' in url:
 print(json.dumps({'tag_name':'v0.3.4'}))
else:
 dest = args[args.index('-o') + 1]
 shutil.copyfile(root / url.rsplit('/',1)[-1], dest)
''')
    curl.chmod(0o755)
    environment = dict(os.environ, HOME=str(root / 'home'), PATH=str(fakebin) + os.pathsep + os.environ['PATH'], WOCLI_INSTALL_FIXTURE=str(root))
    environment.pop('VERSION', None)
    environment.pop('WOCLI_DOWNLOAD_BASE', None)
    for endpoint in [None, 'github']:
        destination = root / ('mirror-install' if endpoint is None else 'github-install')
        environment['INSTALL_DIR'] = str(destination)
        if endpoint: environment['WOCLI_DOWNLOAD_BASE'] = endpoint
        (root / 'requests').write_text('')
        result = subprocess.run(['bash', str(ROOT / 'install.sh')], env=environment, capture_output=True, text=True)
        assert result.returncode == 0, result.stdout + result.stderr
        assert (destination / 'wocli').exists()
        urls = (root / 'requests').read_text().splitlines()
        assert len(urls) == 3, urls
        if endpoint is None: assert all(u.startswith('https://wocli.yhan.fun/') for u in urls), urls
        else:
            assert urls[0].startswith('https://api.github.com/'), urls
            assert all(u.startswith('https://github.com/yhan-sun/wocli-music/') for u in urls[1:]), urls
    for broken in ['', valid + valid, '0' * 64 + f'  {archive.name}\n']:
        sums.write_text(broken)
        destination = root / 'failed-install'
        environment['INSTALL_DIR'] = str(destination)
        result = subprocess.run(['bash', str(ROOT / 'install.sh')], env=environment, capture_output=True, text=True)
        assert result.returncode != 0
        assert not destination.exists(), result.stdout + result.stderr
    print('Mirror/direct routing, real installation and missing/duplicate/mismatched checksum tests passed.')
