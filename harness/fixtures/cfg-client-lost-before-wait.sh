#!/bin/sh
# Adapted from tmux regress/cfg-client-lost-before-wait.sh @ 8f25579c
# Copyright (c) 2007 Nicholas Marriott <nicholas.marriott@gmail.com>
# Permission to use, copy, modify, and distribute this software for any purpose
# with or without fee is hereby granted, provided that the above copyright
# notice and this permission notice appear in all copies.
# THE SOFTWARE IS PROVIDED "AS IS" AND THE AUTHOR DISCLAIMS ALL WARRANTIES
# WITH REGARD TO THIS SOFTWARE INCLUDING ALL IMPLIED WARRANTIES OF
# MERCHANTABILITY AND FITNESS. IN NO EVENT SHALL THE AUTHOR BE LIABLE FOR
# ANY SPECIAL, DIRECT, INDIRECT, OR CONSEQUENTIAL DAMAGES OR ANY DAMAGES
# WHATSOEVER RESULTING FROM LOSS OF USE, DATA OR PROFITS, WHETHER IN AN ACTION
# OF CONTRACT, NEGLIGENCE OR OTHER TORTIOUS ACTION, ARISING OUT OF OR IN
# CONNECTION WITH THE USE OR PERFORMANCE OF THIS SOFTWARE.
PATH=/bin:/usr/bin
TERM=screen
export TERM
python3 - "$TEST_TMUX" <<'PY'
import os
from pathlib import Path
import re
import socket
import struct
import subprocess
import sys
import tempfile
import time

rmux = sys.argv[1]

def message(kind, body=b""):
    return struct.pack("<4sHHIHH", b"RMUX", 1, kind, len(body), 0, 0) + body

def string(value):
    return struct.pack("<I", len(value)) + value

def wait_for(predicate):
    end = time.monotonic() + 5
    while time.monotonic() < end:
        result = predicate()
        if result:
            return result
        time.sleep(0.05)
    raise AssertionError("timed out waiting for client cleanup")

with tempfile.TemporaryDirectory(prefix="rmux-cfg-early-", dir="/tmp") as tmp:
    root = Path(tmp)
    sockpath = root / "socket"
    go = root / "go"
    conf = root / "conf"
    conf.write_text("run-shell 'i=0; while [ ! -f %s ] && [ $i -lt 100 ]; "
        "do sleep 0.1; i=$((i + 1)); done'\n" % go)
    server = subprocess.Popen([rmux, "-D", "-v", "-S", str(sockpath),
        "-f", str(conf)], cwd=tmp, stdout=subprocess.DEVNULL)
    logpath = root / ("rmux-server-%d.log" % server.pid)

    def log():
        return logpath.read_text() if logpath.exists() else ""

    try:
        wait_for(sockpath.exists)
        with socket.socket(socket.AF_UNIX, socket.SOCK_STREAM) as client:
            client.connect(str(sockpath))
            # All identify messages precede the malformed empty COMMAND in
            # one readiness turn, before the server queue fixed point.
            client.sendall(message(108, string(tmp.encode())) +
                message(107, struct.pack("<i", os.getpid())) +
                message(106) + message(200))
            lost = wait_for(lambda: re.search(r"lost client (\S+)", log()))[1]
            wait_for(lambda: "free client %s (0 references)" % lost in log())
            contents = log()
            callback = "cmdq_next <client-%d>: [cfg_client_done/" % os.getpid()
            assert contents.index("lost client " + lost) < \
                contents.index(callback), contents
            assert "cmdq_next <global>: [cfg_done/" not in contents, contents
    finally:
        go.touch()
        server.terminate()
        try:
            server.wait(timeout=3)
        except subprocess.TimeoutExpired:
            server.kill()
            server.wait()
PY
