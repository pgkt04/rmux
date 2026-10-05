#!/bin/sh
set -eu
# Link the helper driver to the oracle build's pinned objects without tmux main.
root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
build=${1:?usage: build-format-helper-driver.sh pinned-oracle-build-dir output}
out=${2:?usage: build-format-helper-driver.sh pinned-oracle-build-dir output}
python3 - "$root/scripts/format-helper-driver.c" "$build" "$out" <<'PY'
import pathlib,re,shlex,subprocess,sys
source,build,out=sys.argv[1:]
b=pathlib.Path(build)
make=(b/'Makefile').read_text().replace('\\\n',' ')
variables={}
for line in make.splitlines():
    match=re.match(r'([A-Za-z_][A-Za-z_0-9]*) = (.*)',line)
    if match: variables[match[1]]=match[2]
def value(name):
    raw=variables.get(name,'')
    for _ in range(20):
        expanded=re.sub(r'\$\(([^)]+)\)',lambda m:variables.get(m[1],''),raw)
        if expanded==raw: break
        raw=expanded
    return shlex.split(raw)
objects=[str(p) for p in sorted(b.glob('*.o')) if p.name!='tmux.o']
objects += [str(p) for p in sorted((b/'compat').glob('*.o'))]
if not objects: raise SystemExit('pinned oracle build objects missing')
subprocess.run(value('CC')+value('DEFS')+value('DEFAULT_INCLUDES')+value('INCLUDES')+value('AM_CPPFLAGS')+value('CPPFLAGS')+value('AM_CFLAGS')+value('CFLAGS')+['-I.',source,'-o',out]+objects+value('LDFLAGS')+value('LIBS'),cwd=b,check=True)
PY
