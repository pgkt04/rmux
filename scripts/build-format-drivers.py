#!/usr/bin/env python3
"""Build G10 C references from a read-only pinned tmux checkout into temp."""
import argparse
import os
import pathlib
import shlex
import subprocess
import tarfile
import tempfile

PIN = '8f25579c'
REPO = pathlib.Path(__file__).resolve().parent.parent


def run(command, cwd=None, **kwargs):
    return subprocess.run(command, cwd=cwd, check=True, timeout=600, **kwargs)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--tmux-source', type=pathlib.Path, default=pathlib.Path.home() / 'fun/tmux')
    parser.add_argument('--output', type=pathlib.Path, default=pathlib.Path('/tmp/swarm-rmux-build/g10-reference'))
    args = parser.parse_args()
    output = args.output.resolve()
    allowed = [REPO, pathlib.Path('/tmp/swarm-rmux-build').resolve()]
    if not any(output == root or root in output.parents for root in allowed):
        parser.error('--output must be inside rmux or /tmp/swarm-rmux-build')
    output.mkdir(parents=True, exist_ok=True)
    build = pathlib.Path(tempfile.mkdtemp(prefix='tmux-8f25579c-', dir=output))
    archive = build / 'source.tar'
    with archive.open('wb') as stream:
        run(['git', '-C', str(args.tmux_source), 'archive', PIN], stdout=stream)
    with tarfile.open(archive) as source:
        source.extractall(build, filter='data')
    archive.unlink()
    environment = os.environ.copy()
    if os.uname().sysname == 'Darwin':
        prefix = run(['brew', '--prefix'], capture_output=True, text=True).stdout.strip()
        pkg_paths = [f'{prefix}/opt/{library}/lib/pkgconfig' for library in ['libevent', 'ncurses', 'utf8proc']]
        environment['PKG_CONFIG_PATH'] = ':'.join(pkg_paths + [environment.get('PKG_CONFIG_PATH', '')])
    run(['sh', 'autogen.sh'], cwd=build, env=environment)
    run(['./configure', '--disable-jemalloc'], cwd=build, env=environment)
    run(['make', '-j', str(min(4, os.cpu_count() or 1))], cwd=build, env=environment)
    # Ask the configured Makefile for its exact compiler, feature and link flags.
    makefile = build / 'driver-vars.mk'
    makefile.write_text('include Makefile\n$(info DRIVER_CC=$(CC) $(DEFS) $(DEFAULT_INCLUDES) $(INCLUDES) $(AM_CPPFLAGS) $(CPPFLAGS) $(AM_CFLAGS) $(CFLAGS))\n$(info DRIVER_LIBS=$(LDFLAGS) $(tmux_LDFLAGS) $(LIBS))\n.PHONY: driver-vars\ndriver-vars:\n\t@:\n')
    values = run(['make', '--no-print-directory', '-s', '-f', str(makefile), 'driver-vars'], cwd=build, capture_output=True, text=True).stdout.splitlines()
    compiler = shlex.split(next(value.removeprefix('DRIVER_CC=') for value in values if value.startswith('DRIVER_CC=')))
    libraries = shlex.split(next(value.removeprefix('DRIVER_LIBS=') for value in values if value.startswith('DRIVER_LIBS=')))
    # Only tmux.c's main conflicts with the reference entry points.
    run(compiler + ['-Dmain=tmux_reference_main', '-c', 'tmux.c', '-o', 'tmux_nomain.o'], cwd=build)
    objects = [str(path) for path in build.glob('*.o') if path.name not in {'tmux.o', 'tmux_nomain.o'}]
    objects.extend(str(path) for path in (build / 'compat').glob('*.o'))
    common = compiler + ['-Wno-missing-prototypes', '-Wno-missing-declarations', '-I' + str(build)]
    helper = output / 'helper-driver'
    sort = output / 'sort-driver'
    run(common + [str(REPO / 'scripts/format-helper-driver.c')] + objects + [str(build / 'tmux_nomain.o')] + libraries + ['-o', str(helper)], cwd=build)
    sort_objects = [obj for obj in objects if pathlib.Path(obj).name not in {'sort.o', 'paste.o'}]
    run(common + [str(REPO / 'scripts/format-sort-driver.c')] + sort_objects + [str(build / 'tmux_nomain.o')] + libraries + ['-o', str(sort)], cwd=build)
    print(f'RMUX_G10_HELPER_DRIVER={helper}')
    print(f'RMUX_G10_SORT_DRIVER={sort}')


if __name__ == '__main__':
    main()
