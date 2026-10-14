#!/usr/bin/env python3
"""Gateway latency and throughput against the demo LMS, for docs/PERFORMANCE.md.

Starts the invented-library LMS (tools/demo/fake_lms.py) and a gateway on loopback with a
scratch data directory, signs in, and times:
  - sign-in (dominated by the password hash, by design);
  - GET /v1/me (the gateway alone);
  - POST /v1/rpc albums 0 50 (the gateway, its policy and the round trip to LMS);
one request at a time, then with several clients at once. It also reports the gateway's
memory (resident set) before and after. Nothing here touches a real LMS.

Usage: tools/demo/bench.py [GATEWAY_BINARY]   (default target/release/anywhere-gateway)
"""
import concurrent.futures
import json
import os
import shutil
import statistics
import subprocess
import sys
import tempfile
import time
import urllib.request

ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
GW = sys.argv[1] if len(sys.argv) > 1 else os.path.join(ROOT, 'target/release/anywhere-gateway')
BASE = 'http://127.0.0.1:19543'
PASSWORD = 'bench-password-123'


def request(method, path, body=None, token=None, headers=None):
    h = {'Content-Type': 'application/json', **(headers or {})}
    if token:
        h['Authorization'] = 'Bearer ' + token
    data = json.dumps(body).encode() if body is not None else None
    req = urllib.request.Request(BASE + path, data, h, method=method)
    with urllib.request.urlopen(req, timeout=30) as r:
        return json.loads(r.read() or b'null')


def timed(fn, n):
    out = []
    for _ in range(n):
        t = time.perf_counter()
        fn()
        out.append((time.perf_counter() - t) * 1000)
    return out


def summary(ms):
    ms = sorted(ms)
    q = lambda p: ms[min(len(ms) - 1, int(p * len(ms)))]
    return f'median {statistics.median(ms):.2f} ms, p95 {q(0.95):.2f} ms, p99 {q(0.99):.2f} ms'


def rss_kb(pid):
    with open(f'/proc/{pid}/status') as f:
        return {k: int(v.split()[0]) for k, v in (l.split(':', 1) for l in f) if k in ('VmRSS', 'VmHWM')}


def main():
    data = tempfile.mkdtemp()
    procs = []
    try:
        procs.append(subprocess.Popen([sys.executable, os.path.join(ROOT, 'tools/demo/fake_lms.py'), '--port', '19000'],
                                      stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL))
        subprocess.run([GW, 'admin', '--data-dir', data, 'create-user', 'bench', '--role', 'admin'],
                       input=PASSWORD + '\n', text=True, check=True, stdout=subprocess.DEVNULL)
        t0 = time.perf_counter()
        gw = subprocess.Popen([GW, 'serve', '--data-dir', data, '--listen', '127.0.0.1:19543',
                               '--admin-listen', '127.0.0.1:19545', '--lms', '127.0.0.1', '--lms-http-port', '19000',
                               '--lms-cli-port', '19090', '--lms-slimproto-port', '13483'],
                              stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        procs.append(gw)
        while True:
            try:
                urllib.request.urlopen(BASE + '/healthz', timeout=1).read()
                break
            except OSError:
                time.sleep(0.01)
        print(f'start-up to /healthz: {(time.perf_counter() - t0) * 1000:.0f} ms')
        time.sleep(1)
        print(f'memory at rest: {rss_kb(gw.pid)}')

        auth = {'X-Anywhere-Client': '1'}
        login = {'username': 'bench', 'password': PASSWORD, 'device_name': 'Bench', 'refresh': 'body'}
        token = None

        def sign_in():
            nonlocal token
            token = request('POST', '/v1/auth/login', login, headers=auth)['access_token']
        print('sign-in:', summary(timed(sign_in, 5)))

        me = lambda: request('GET', '/v1/me', token=token)
        rpc = lambda: request('POST', '/v1/rpc', {'player': '', 'cmd': ['albums', 0, 50, 'tags:al']}, token=token)
        for f in (me, rpc):
            f()
        print('GET /v1/me, one at a time:', summary(timed(me, 500)))
        print('POST /v1/rpc albums 0 50, one at a time:', summary(timed(rpc, 300)))

        for name, f, n in (('GET /v1/me', me, 4000), ('POST /v1/rpc albums', rpc, 1500)):
            for clients in (8, 32):
                with concurrent.futures.ThreadPoolExecutor(clients) as ex:
                    t = time.perf_counter()
                    lat = list(ex.map(lambda _: timed(f, 1)[0], range(n)))
                    secs = time.perf_counter() - t
                print(f'{name}, {clients} clients: {n / secs:.0f} requests/s, {summary(lat)}')
        print(f'memory after: {rss_kb(gw.pid)}')
    finally:
        for p in procs:
            p.terminate()
        for p in procs:
            p.wait()
        shutil.rmtree(data, ignore_errors=True)


if __name__ == '__main__':
    main()
