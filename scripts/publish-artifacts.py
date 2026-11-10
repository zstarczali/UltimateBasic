"""Publish the UltimateBasic packages of a "Build and Publish" run over explicit FTPS (TLS on port 21).

  publish-artifacts.py --validate-run     the package.yml run (BUILD_RUN_ID) completed successfully
  publish-artifacts.py ARTIFACTS          upload ultimate-basic-<platform>.zip of every platform (ARTIFACTS, extracted);
                                          the names carry no version, the upload replaces the previous ones
"""
import ftplib
import json
import os
from pathlib import Path, PurePosixPath
import ssl
import sys
import urllib.request

WORKFLOW = '.github/workflows/package.yml'
PLATFORMS = ('windows-x64', 'windows-arm64', 'macos-x64', 'macos-arm64', 'linux-x64')
PACKAGES = {f'ultimate-basic-{p}.zip' for p in PLATFORMS}


def validate_run():
    run_id = os.environ['BUILD_RUN_ID']
    if not run_id.isascii() or not run_id.isdecimal():
        raise ValueError('Build run ID must be a positive integer.')
    repository = os.environ['GH_REPOSITORY']
    request = urllib.request.Request(
        f'https://api.github.com/repos/{repository}/actions/runs/{run_id}',
        headers={'Authorization': 'Bearer ' + os.environ['GH_TOKEN'],
                 'Accept': 'application/vnd.github+json',
                 'X-GitHub-Api-Version': '2022-11-28'},
    )
    with urllib.request.urlopen(request, timeout=30) as response:
        run = json.load(response)
    if run.get('path', '').split('@')[0] != WORKFLOW:
        raise ValueError('The selected run is not a "Build and Publish UltimateBasic64" run.')
    if run.get('status') != 'completed' or run.get('conclusion') != 'success':
        raise ValueError('The selected build must have completed successfully.')
    print(f'Validated build {run_id}, commit {run["head_sha"]}.')


def collect_files(root):
    """name → path of every platform's ZIP (the .deb / .rpm artifacts are not published)."""
    files = {}
    for path in sorted(Path(root).rglob('*')):
        if path.is_symlink():
            raise ValueError('Artifact symlinks are not supported.')
        if not path.is_file() or path.name not in PACKAGES:
            continue
        if path.name in files:
            raise ValueError(f'Duplicate output filename: {path.name}')
        if path.stat().st_size == 0:
            raise ValueError(f'Empty artifact file: {path.name}')
        files[path.name] = path
    missing = PACKAGES - files.keys()
    if missing:
        raise ValueError('Missing platform packages: ' + ', '.join(sorted(missing)))
    return files


def safe_error(error):
    message = str(error)
    for key in ('FTP_PASSWORD', 'GH_TOKEN'):
        secret = os.environ.get(key)
        if secret:
            message = message.replace(secret, '[REDACTED]')
    return ' '.join(message.splitlines())[:600]


def open_directory(ftp, directory):
    try:
        ftp.cwd(str(directory))
        return
    except ftplib.error_perm as error:
        if not str(error).startswith('550'):
            raise
    ftp.cwd('/')
    for part in directory.parts[1:]:
        try:
            ftp.cwd(part)
        except ftplib.error_perm as error:
            if not str(error).startswith('550'):
                raise
            ftp.mkd(part)
            ftp.cwd(part)


def upload(root):
    files = collect_files(root)
    password = os.environ.get('FTP_PASSWORD', '')
    if not password:
        raise ValueError('Set the FTP_PASSWORD repository Actions secret before publishing.')
    directory = PurePosixPath(os.environ['FTP_DIRECTORY'])
    if not directory.is_absolute() or '..' in directory.parts:
        raise ValueError('FTP_DIRECTORY must be an absolute directory without parent traversal.')
    stage = 'opening FTPS connection'
    try:
        # Validate the server certificate and encrypt both login and data connections.
        with ftplib.FTP_TLS(context=ssl.create_default_context(), timeout=120) as ftp:
            stage = 'connecting to server'
            ftp.connect(os.environ['FTP_HOST'], 21)
            stage = 'login (check Forpsi country/IP restrictions and the FTP_PASSWORD secret)'
            ftp.login(os.environ['FTP_USERNAME'], password)
            stage = 'enabling encrypted data transfers'
            ftp.prot_p()
            ftp.set_pasv(True)
            stage = f'opening target directory {directory}'
            open_directory(ftp, directory)
            for name, path in files.items():
                stage = f'uploading {name}'
                print(f'Uploading {name} ({path.stat().st_size} bytes)', flush=True)
                with path.open('rb') as data:
                    ftp.storbinary('STOR ' + name, data, blocksize=256 * 1024)
                stage = f'verifying uploaded size of {name}'
                ftp.voidcmd('TYPE I')
                if ftp.size(name) != path.stat().st_size:
                    raise ValueError(f'Remote size verification failed: {name}')
    except (ftplib.Error, OSError, EOFError) as error:
        detail = safe_error(error)
        if stage.startswith('login') and str(error).startswith('530'):
            detail += ' Forpsi can also reject valid credentials through country/IP restrictions. Check its FTP log for DENY.'
        raise ValueError(f'FTP {stage}: {detail}') from None
    print(f'Published {len(files)} files. Existing unrelated files were retained.')


if __name__ == '__main__':
    try:
        if sys.argv[1:] == ['--validate-run']:
            validate_run()
        elif len(sys.argv) == 2:
            upload(sys.argv[1])
        else:
            raise ValueError('Usage: publish-artifacts.py ARTIFACT_DIRECTORY | --validate-run')
    except Exception as error:
        # FTP errors can contain server responses: never print login details.
        if isinstance(error, ValueError):
            print(f'Publish failed: {error}', file=sys.stderr)
        else:
            print(f'Publish failed ({type(error).__name__}). Check the connection and source build.', file=sys.stderr)
        sys.exit(1)
