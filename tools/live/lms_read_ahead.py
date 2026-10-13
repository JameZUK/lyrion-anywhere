#!/usr/bin/env python3
"""Opt-in live check: how LMS treats a player that reads a whole track ahead.

Spike S1 of the rolling cache (docs/plans/wave3-design.md), kept as a regression check. It
poses as the dedicated test player (a minimal slimproto client, volume 0), loads five tracks
from the library, and checks:

  early  STMd sent as soon as a stream is fully received brings the next `strm s` at once;
         LMS then waits for that track's STMs before sending the one after.
  edit   deleting the received-but-not-started next track: `strm f` then a new `strm s`,
         while the current track carries on (so `f` must keep the audible track).
  move   the same for moving it later in the queue.
  stmu   STMu with the next track received stops playback (why the app must not send it).
  stmo   STMo leaves LMS playing; a later STMs moves it on and releases the next stream.

Usage: tools/live/lms_read_ahead.py dev/local.env [scenario ...]
The env file gives LMS_HOST, LMS_JSON_PORT and TEST_PLAYER_MAC. Only the test player is
touched; its queue is cleared at the end.
"""
import json
import socket
import struct
import sys
import threading
import time
import urllib.request

env = dict(
    line.strip().split('=', 1)
    for line in open(sys.argv[1])
    if '=' in line and not line.lstrip().startswith('#')
)
HOST, JPORT, MAC = env['LMS_HOST'], int(env['LMS_JSON_PORT']), env['TEST_PLAYER_MAC']
SCENARIOS = sys.argv[2:] or ['early', 'edit', 'move', 'stmu', 'stmo']


def rpc(player, *cmd):
    body = json.dumps({'id': 1, 'method': 'slim.request', 'params': [player, [str(c) for c in cmd]]}).encode()
    req = urllib.request.Request(f'http://{HOST}:{JPORT}/jsonrpc.js', body, {'Content-Type': 'application/json'})
    with urllib.request.urlopen(req, timeout=10) as r:
        return json.loads(r.read())['result']


def status():
    r = rpc(MAC, 'status', '-', '1')
    return r.get('mode'), int(r.get('playlist_cur_index', -1)), int(r.get('playlist_tracks', 0))


class Player:
    def __init__(self):
        self.s = socket.create_connection((HOST, 3483), timeout=30)
        self.lock = threading.Lock()
        self.t0 = time.time()
        self.received = 0
        self.events = []  # (seconds, what)
        self.streams = 0
        self.on_received = lambda n: None

    def note(self, what):
        self.events.append((time.time() - self.t0, what))

    def send(self, op, payload):
        with self.lock:
            try:
                self.s.sendall(op + struct.pack('>I', len(payload)) + payload)
            except OSError:
                pass  # closed at the end of a scenario

    def helo(self):
        mac = bytes(int(x, 16) for x in MAC.split(':'))
        caps = b'Model=squeezelite,ModelName=SqueezeLite,MaxSampleRate=48000,flc,mp3,ogg,pcm'
        self.send(b'HELO', bytes([12, 0]) + mac + bytes(16) + struct.pack('>HQ', 0x4000, 0) + b'en' + caps)

    def stat(self, ev, ts=0):
        jiffies = int((time.time() - self.t0) * 1000) & 0xFFFFFFFF
        self.send(b'STAT', ev + struct.pack('>BBBIIQHIIIIHIIH', 0, 0, 0, 1 << 21, 0, self.received, 0,
                                            jiffies, 1 << 20, 0, 0, 0, 0, ts, 0))
        if ev != b'STMt':
            self.note('-> ' + ev.decode())

    def exact(self, n):
        b = b''
        while len(b) < n:
            c = self.s.recv(n - len(b))
            if not c:
                raise EOFError('LMS closed the connection')
            b += c
        return b

    def fetch(self, req, ip, port):
        t = socket.create_connection((ip or HOST, port or JPORT), timeout=30)
        t.sendall(req)
        self.stat(b'STMc')  # connected: LMS ignores STMd for a stream it never saw start
        head = b''
        while b'\r\n\r\n' not in head:
            c = t.recv(1)
            if not c:
                break
            head += c
        self.stat(b'STMh')  # headers received
        while c := t.recv(65536):
            self.received += len(c)
        t.close()

    def run(self):
        try:
            while True:
                n = struct.unpack('>H', self.exact(2))[0]
                body = self.exact(n)
                if body[:4] != b'strm':
                    continue
                p = body[4:]
                cmd = chr(p[0])
                if cmd == 't':
                    self.stat(b'STMt', struct.unpack('>I', p[14:18])[0])
                    continue
                self.note('<- strm ' + cmd)
                if cmd == 's':
                    self.streams += 1
                    port, ip = struct.unpack('>HI', p[18:24])
                    ipstr = socket.inet_ntoa(struct.pack('>I', ip)) if ip else None
                    threading.Thread(target=self.stream, args=(self.streams, p[24:], ipstr, port), daemon=True).start()
        except (EOFError, OSError):
            pass

    def stream(self, n, req, ip, port):
        self.fetch(req, ip, port)
        self.note(f'received {n}')
        self.on_received(n)

    def wait(self, pred, secs):
        end = time.time() + secs
        while time.time() < end:
            if pred():
                return True
            time.sleep(0.05)
        return False

    def got(self, what):
        return any(w == what for _, w in self.events)

    def when(self, what):
        return next((t for t, w in self.events if w == what), None)


failures = []


def check(cond, msg):
    print(('  ok   ' if cond else '  FAIL ') + msg, flush=True)
    if not cond:
        failures.append(msg)


def scenario(name, tracks):
    print(f'== {name}', flush=True)
    pl = Player()
    ready = {}

    def on_received(n):
        if n == 1:
            pl.stat(b'STMs')
        pl.stat(b'STMd')
        ready[n] = True

    pl.on_received = on_received
    threading.Thread(target=pl.run, daemon=True).start()
    pl.helo()
    time.sleep(2)
    rpc(MAC, 'mixer', 'volume', '0')
    rpc(MAC, 'playlistcontrol', 'cmd:load', 'track_id:' + ','.join(map(str, tracks)))
    try:
        check(pl.wait(lambda: ready.get(2), 30), 'track 2 streamed after an early STMd')
        if name == 'early':
            starts = [t for t, w in pl.events if w == '<- strm s']
            check(len(starts) >= 2 and starts[1] - pl.when('-> STMd') < 2, 'the next strm s came within 2 s of STMd')
            time.sleep(8)
            check(pl.streams == 2, 'LMS held track 3 until track 2 started')
            check(status()[:2] == ('play', 0), 'LMS still shows track 1 playing')
            pl.stat(b'STMs')
            check(pl.wait(lambda: pl.streams == 3, 10), 'STMs for track 2 released track 3')
            check(pl.wait(lambda: status()[1] == 1, 5), 'LMS moved on to track 2')
        elif name in ('edit', 'move'):
            edited = len(pl.events)
            if name == 'edit':
                rpc(MAC, 'playlist', 'delete', '1')
            else:
                rpc(MAC, 'playlist', 'move', '1', '3')
            check(pl.wait(lambda: pl.got('<- strm f'), 5), 'LMS sent strm f')
            check(pl.wait(lambda: pl.streams == 3, 5), 'then a new strm s for the new next track')
            check(all(w != '<- strm q' for _, w in pl.events[edited:]), 'and no strm q')
            check(status()[:2] == ('play', 0), 'track 1 carries on playing')
        elif name in ('stmu', 'stmo'):
            pl.stat(b'STMu' if name == 'stmu' else b'STMo')
            time.sleep(3)
            mode, cur, _ = status()
            if name == 'stmu':
                check(mode == 'stop', f'STMu stops playback (mode={mode}, index {cur})')
            else:
                check((mode, cur) == ('play', 0), f'STMo leaves LMS playing track 1 (mode={mode}, index {cur})')
                check(pl.streams == 2, 'and sends no new stream')
                pl.stat(b'STMs')
                check(pl.wait(lambda: pl.streams == 3, 10), 'STMs after the gap released track 3')
                check(pl.wait(lambda: status()[1] == 1, 5), 'LMS moved on to track 2')
    finally:
        rpc(MAC, 'stop')
        rpc(MAC, 'playlist', 'clear')
        pl.s.close()
        time.sleep(1)


def main():
    titles = rpc('', 'titles', '0', '5')['titles_loop']
    tracks = [t['id'] for t in titles]
    if len(tracks) < 5:
        sys.exit('the library needs at least five tracks')
    for name in SCENARIOS:
        scenario(name, tracks)
    if failures:
        sys.exit(f'{len(failures)} check(s) failed')
    print('all checks passed')


main()
