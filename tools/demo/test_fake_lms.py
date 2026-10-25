"""Tests for the demo fake LMS's Slimproto side (no network: a socket pair stands in).

    python3 -m unittest tools/demo/test_fake_lms.py

A client's bytes are untrusted: truncated, oversize or junk frames must never raise, only
end or be ignored. The happy path is a HELO that lists the player and starts its stream, a
SETD that names it, and the close that takes it off the list again.
"""
import copy
import random
import socket
import struct
import sys
import threading
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import fake_lms  # noqa: E402

MAC = bytes([0x02, 0x4C, 0x41, 0x00, 0x00, 0x09])
PLAYER = "02:4c:41:00:00:09"


def client_frame(op, body):
    """A player-to-server frame: the opcode, a 4-byte length, the body."""
    return op + struct.pack(">I", len(body)) + body


def helo(mac=MAC):
    # Device id, revision, MAC, then capabilities (enough of a HELO for the fake).
    return client_frame(b"HELO", bytes([12, 0]) + mac + b"\x00" * 10 + b"Model=squeezelite")


def read_frames(data):
    """Split what the server sent into (opcode, payload) pairs."""
    out = []
    while len(data) >= 2:
        n = struct.unpack(">H", data[:2])[0]
        out.append((data[2:6], data[6:2 + n]))
        data = data[2 + n:]
    return out


class SlimTestCase(unittest.TestCase):
    def setUp(self):
        self._players = copy.deepcopy(fake_lms.PLAYERS)
        self._states = copy.deepcopy(fake_lms.STATES)
        self._slim = dict(fake_lms.SLIM)

    def tearDown(self):
        fake_lms.PLAYERS[:] = self._players
        fake_lms.STATES.clear()
        fake_lms.STATES.update(self._states)
        fake_lms.SLIM.clear()
        fake_lms.SLIM.update(self._slim)

    def feed(self, data, chunk=None):
        """Run one connection over `data` (sent in chunks of `chunk` bytes, then closed) and
        return what the server sent back. An exception in the server is raised here."""
        server, client = socket.socketpair()
        client.settimeout(5)
        errors = []

        def run():
            try:
                fake_lms.slim_client(server)
            except BaseException as e:  # noqa: BLE001 - reported to the test
                errors.append(e)

        t = threading.Thread(target=run)
        t.start()
        step = chunk or max(1, len(data))
        try:
            for i in range(0, len(data), step):
                client.sendall(data[i:i + step])
        except OSError:
            pass  # the server may hang up first (an oversize frame)
        client.shutdown(socket.SHUT_WR)
        got = b""
        try:
            while part := client.recv(65536):
                got += part
        except OSError:
            pass
        t.join(5)
        client.close()
        self.assertFalse(t.is_alive(), "the connection never ended")
        if errors:
            raise errors[0]
        return got

    def listed(self, player=PLAYER):
        return [x for x in fake_lms.PLAYERS if x["playerid"] == player]


class FrameParsing(SlimTestCase):
    def test_truncated_header_is_ignored(self):
        for n in range(1, 8):
            with self.subTest(n=n):
                self.assertEqual(self.feed(helo()[:n]), b"")
                self.assertEqual(self.listed(), [])

    def test_truncated_body_is_ignored(self):
        frame = helo()
        for n in (8, 9, 12, len(frame) - 1):
            with self.subTest(n=n):
                self.assertEqual(self.feed(frame[:n]), b"")
                self.assertEqual(self.listed(), [])

    def test_short_helo_is_ignored(self):
        self.assertEqual(self.feed(client_frame(b"HELO", b"\x0c\x00\x02")), b"")
        self.assertEqual(self.listed(), [])

    def test_oversize_frame_ends_the_connection(self):
        data = b"STAT" + struct.pack(">I", (1 << 20) + 1) + b"x" * 64 + helo()
        self.assertEqual(self.feed(data), b"")
        self.assertEqual(self.listed(), [])
        # The largest length taken is 1 MiB; the HELO after such a frame is read.
        big = client_frame(b"STAT", b"\x00" * (1 << 20))
        got = self.feed(big + helo(), chunk=65536)
        self.assertEqual(read_frames(got)[0][0], b"setd")

    def test_maximum_length_field_does_not_raise(self):
        self.assertEqual(self.feed(b"HELO\xff\xff\xff\xff" + b"\x00" * 32), b"")

    def test_unknown_opcodes_and_empty_bodies_are_ignored(self):
        data = client_frame(b"\x00\x00\x00\x00", b"") + client_frame(b"SETD", b"") + client_frame(b"STAT", b"junk")
        self.assertEqual(self.feed(data), b"")

    def test_setd_before_helo_is_ignored(self):
        names = [x["name"] for x in fake_lms.PLAYERS]
        self.feed(client_frame(b"SETD", b"\x00Intruder"))
        self.assertEqual([x["name"] for x in fake_lms.PLAYERS], names)

    def test_junk_never_raises(self):
        rnd = random.Random(20261025)
        for i in range(300):
            data = bytes(rnd.getrandbits(8) for _ in range(rnd.randint(0, 200)))
            if i % 3 == 0:  # junk after a real HELO, so the SETD and later paths are reached too
                data = helo() + data
            if i % 5 == 0:  # a plausible frame with a junk body
                op = rnd.choice([b"HELO", b"SETD", b"STAT", b"BYE!"])
                data += client_frame(op, data[:rnd.randint(0, len(data))])
            with self.subTest(i=i):
                self.feed(data, chunk=rnd.randint(1, 32))
            fake_lms.PLAYERS[:] = copy.deepcopy(self._players)

    def test_frames_split_across_reads_are_joined(self):
        got = self.feed(helo() + client_frame(b"SETD", b"\x00Desk"), chunk=1)
        self.assertEqual([op for op, _ in read_frames(got)], [b"setd", b"strm", b"strm"])


class HappyPath(SlimTestCase):
    def test_helo_lists_the_player_and_starts_its_stream(self):
        server, client = socket.socketpair()
        client.settimeout(5)
        t = threading.Thread(target=fake_lms.slim_client, args=(server,))
        t.start()
        client.sendall(helo())
        got = b""
        while len(read_frames(got)) < 3:
            got += client.recv(65536)
        frames = read_frames(got)
        self.assertEqual(frames[0], (b"setd", b"\x00"))
        (q_op, q), (s_op, s) = frames[1], frames[2]
        self.assertEqual((q_op, q[:1]), (b"strm", b"q"))
        self.assertEqual((s_op, s[:3]), (b"strm", b"s1f"))
        self.assertEqual(struct.unpack(">H", s[18:20])[0], fake_lms.HTTP_PORT)
        self.assertEqual(s[24:], f"GET /stream.mp3?player={PLAYER} HTTP/1.0\r\n\r\n".encode())
        self.assertEqual(len(self.listed()), 1)
        self.assertIn(PLAYER, fake_lms.SLIM)
        st = fake_lms.state_of(PLAYER)
        self.assertEqual(st["mode"], "play")
        self.assertEqual(st["queue"][0]["samplerate"], "96000")
        # SETD names it (cut to 64 characters).
        client.sendall(client_frame(b"SETD", b"\x00" + b"D" * 80 + b"\x00rest"))
        pause = threading.Event()
        for _ in range(100):
            if self.listed()[0]["name"] != "This browser":
                break
            pause.wait(0.02)
        self.assertEqual(self.listed()[0]["name"], "D" * 64)
        # A pause through the queue commands reaches the player as a strm p.
        st["mode"] = "pause"
        fake_lms.slim_follow(PLAYER, "pause", [])
        got = b""
        while not read_frames(got):
            got += client.recv(65536)
        self.assertEqual(read_frames(got)[0][1][:1], b"p")
        # Closing takes it off the list.
        client.close()
        t.join(5)
        self.assertFalse(t.is_alive())
        self.assertEqual(self.listed(), [])
        self.assertNotIn(PLAYER, fake_lms.SLIM)

    def test_strm_frames(self):
        for c in "qpu":
            with self.subTest(c=c):
                s = fake_lms.strm(c)
                self.assertEqual(len(s), 24)
                self.assertEqual(s[:2], c.encode() + b"0")
        s = fake_lms.strm("s", PLAYER)
        self.assertEqual(s[:7], b"s1f????")
        self.assertTrue(s[24:].startswith(b"GET /stream.mp3?player="))
        self.assertEqual(fake_lms.slim_frame(b"strm", b"abc"), b"\x00\x07strmabc")


if __name__ == "__main__":
    unittest.main()
