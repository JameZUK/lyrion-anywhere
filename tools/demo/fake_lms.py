#!/usr/bin/env python3
"""A stand-in Lyrion Music Server with an invented library, for demos and screenshots.

Every artist, album, track and player here is made up, and the cover art is generated, so
screenshots taken against it show nothing from a real installation. It answers the JSON-RPC
commands the web app uses (read-only: playback commands are accepted and ignored), serves
artwork, and serves each track as generated audio (a WAV of shaped noise) so the gateway's
track analysis has something to analyse. It is not a test double for the gateway's security tests (those live in
gateway/tests).

    python3 tools/demo/fake_lms.py --port 19000

Needs Pillow (python3 -m pip install pillow) for the generated artwork.
"""
import argparse
import colorsys
import hashlib
import io
import json
import math
import random
import re
import struct
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

from PIL import Image, ImageDraw, ImageFilter

ALBUMS = [
    ("Night Ferry", "The Paper Lanterns", 2019, ["Harbour Lights", "Last Crossing", "Night Ferry", "Paper Moons", "Undertow", "Gulls at Dawn", "Coastline", "Home Port"]),
    ("Glasshouse", "Marisol Vega", 2021, ["Glasshouse", "Fern & Frost", "Morning Light", "Small Weather", "Greenhouse Effect", "Seedlings", "Petrichor"]),
    ("Static Bloom", "Northbound Signal", 2017, ["Transmission", "Static Bloom", "Radio Silence", "Low Orbit", "Signal Fire", "Northbound"]),
    ("Saltwater Hymns", "Ada & the Undertow", 2015, ["Saltwater Hymn", "Driftwood", "The Long Tide", "Lantern Song", "Fathoms", "Shoreline"]),
    ("Small Hours", "Kestrel Lane", 2022, ["3 a.m.", "Streetlights", "Small Hours", "Night Bus", "Neon Rain", "First Train"]),
    ("Winter Études", "Orchard Quartet", 2012, ["Étude No. 1", "Étude No. 2", "Snowfield", "Étude No. 3", "Hoarfrost", "Étude No. 4"]),
    ("Parallax", "Lumen Drift", 2020, ["Parallax", "Afterimage", "Wavelength", "Prism", "Refraction", "Event Horizon", "Lumen"]),
    ("Analogue Summer", "The Copper Hours", 2018, ["Tape Hiss", "Analogue Summer", "Polaroid", "Long Drive", "Sun Flare", "Cassette Heart"]),
    ("Tidal Lines", "Haruka Mori", 2023, ["Tidal Lines", "Rock Pools", "Moonpull", "Sea Glass", "Ebb", "Flood"]),
    ("Field Recordings", "Brass Atlas", 2016, ["Field Recording I", "Meadowlark", "Hedgerow", "Field Recording II", "Barn Owl", "Evensong"]),
    ("Blue Hour Radio", "Selene Park", 2024, ["Blue Hour", "Late Show", "Request Line", "Dedication", "Sign-off"]),
    ("Hymns for the Road", "Tin Roof Choir", 2014, ["Open Road", "Tin Roof", "Mile Marker", "Rest Stop", "Homeward"]),
]

ARTISTS = sorted({a[1] for a in ALBUMS} | {"Various Artists"})
# Invented genres, and each album's genre (by position in ALBUMS).
GENRES = ["Ambient", "Classical", "Electronic", "Folk", "Indie", "Jazz"]
ALBUM_GENRE = [3, 4, 2, 3, 4, 1, 2, 4, 0, 0, 5, 3]
# Order in which the albums were "added to the library": most recent first.
ADDED = [10, 8, 4, 6, 1, 11, 0, 2, 7, 3, 9, 5]
COMPILATIONS = {9, 11}  # "Field Recordings", "Hymns for the Road"
# Other roles: composers and a conductor (artist ids are positions in ARTISTS, plus one).
COMPOSERS = ["Orchard Quartet", "Haruka Mori"]
CONDUCTORS = ["Orchard Quartet"]
# A virtual library, so the picker shows in the demo.
LIBRARIES = [{"id": "1a2b3c4d", "name": "Lossless only (64)"}]


def folder_tree():
    """The music folder: one folder per genre, an album folder in each, its tracks inside."""
    rows = {0: []}
    next_id = [5000]

    def new_id():
        next_id[0] += 1
        return next_id[0]

    for g, genre in enumerate(GENRES):
        gid = new_id()
        rows[0].append({"id": gid, "filename": genre, "type": "folder",
                        "url": f"file:///srv/music/{genre}"})
        rows[gid] = []
        for aidx, (album, artist, _, _) in enumerate(ALBUMS):
            if ALBUM_GENRE[aidx] != g:
                continue
            fid = new_id()
            name = f"{artist} - {album}"
            rows[gid].append({"id": fid, "filename": name, "type": "folder",
                              "url": f"file:///srv/music/{genre}/{name}"})
            rows[fid] = [{"id": t["id"], "filename": f"{t['tracknum']:02d} {t['title']}.flac", "type": "track",
                          "title": t["title"], "coverid": t["coverid"],
                          "duration": f"{t['duration'] // 60}:{t['duration'] % 60:02d}",
                          "url": f"file:///srv/music/{genre}/{name}/{t['tracknum']:02d}.flac"}
                         for t in TRACKS if t["album_id"] == aidx + 1]
            rows[fid].append({"id": new_id(), "filename": "cover.jpg", "type": "unknown"})
    return rows
PLAYERS = [
    {"playerid": "aa:bb:cc:00:00:01", "name": "Lounge", "model": "squeezelite", "connected": 1, "power": 1, "isplaying": 1},
    {"playerid": "aa:bb:cc:00:00:02", "name": "Study", "model": "squeezelite", "connected": 1, "power": 1, "isplaying": 0},
    {"playerid": "aa:bb:cc:00:00:03", "name": "Garden Room", "model": "squeezelite", "connected": 1, "power": 0, "isplaying": 0},
]


def track_rows():
    rows, tid = [], 100
    for aidx, (album, artist, year, titles) in enumerate(ALBUMS):
        album_id = aidx + 1
        artist_id = ARTISTS.index(artist) + 1
        for n, title in enumerate(titles, 1):
            tid += 1
            seconds = 150 + (int(hashlib.sha1(title.encode()).hexdigest(), 16) % 180)
            rows.append({
                "id": tid, "title": title, "artist": artist, "artist_id": artist_id, "album": album,
                "album_id": album_id, "tracknum": n, "duration": seconds, "year": year,
                "coverid": str(1000 + album_id), "artwork_track_id": str(1000 + album_id),
                "type": "flc", "samplerate": "44100", "samplesize": "16", "bitrate": "1020kbps VBR",
            })
    return rows


TRACKS = track_rows()
FOLDERS = folder_tree()
NOW = [t for t in TRACKS if t["album_id"] == 1]  # the Lounge is playing "Night Ferry"
STATE = {"index": 2, "time": 83.4, "volume": 42, "mode": "play", "shuffle": 0, "repeat": 0}


def album_row(i):
    album, artist, year, _ = ALBUMS[i]
    return {"id": i + 1, "album": album, "artist": artist, "artist_id": ARTISTS.index(artist) + 1,
            "year": year, "artwork_track_id": str(1001 + i)}


def status(player):
    playing = player == PLAYERS[0]["playerid"]
    base = {"player_name": next((p["name"] for p in PLAYERS if p["playerid"] == player), "Player"),
            "player_connected": 1, "power": 1, "mixer volume": STATE["volume"] if playing else 30,
            "playlist shuffle": STATE["shuffle"], "playlist repeat": STATE["repeat"], "can_seek": 1}
    if not playing:
        return {**base, "mode": "stop", "playlist_tracks": 0}
    cur = NOW[STATE["index"]]
    return {**base, "mode": STATE["mode"], "time": STATE["time"], "duration": cur["duration"],
            "playlist_cur_index": str(STATE["index"]), "playlist_tracks": len(NOW),
            "playlist_loop": [{**t, "playlist index": i} for i, t in enumerate(NOW)]}


def page(rows, args, key):
    start, count = int(args[0]), int(args[1])
    return {"count": len(rows), key: rows[start:start + count]}


def params(cmd):
    return dict(str(p).split(":", 1) for p in cmd if isinstance(p, str) and ":" in p)


ICON = lambda name: f"plugins/{name}/html/images/icon.png"


def answer(player, cmd):
    c = [str(x) for x in cmd]
    op, rest, p = c[0], c[1:], params(cmd)
    if op in ("players", "serverstatus"):
        return {"player count": len(PLAYERS), "players_loop": PLAYERS, "version": "9.0.0"}
    if op == "albums":
        rows = [album_row(i) for i in range(len(ALBUMS))]
        if "album_id" in p:
            rows = [r for r in rows if str(r["id"]) == p["album_id"]]
        if "artist_id" in p:
            rows = [r for r in rows if str(r["artist_id"]) == p["artist_id"]]
        if "search" in p:
            rows = [r for r in rows if p["search"].lower() in r["album"].lower()]
        if "genre_id" in p:
            rows = [r for r in rows if str(ALBUM_GENRE[r["id"] - 1] + 1) == p["genre_id"]]
        if "year" in p:
            rows = [r for r in rows if str(r["year"]) == p["year"]]
        if p.get("compilation") == "1":
            rows = [{**r, "artist": "Various Artists"} for r in rows if r["id"] - 1 in COMPILATIONS]
        if "library_id" in p:
            rows = [r for r in rows if r["id"] % 3]  # "lossless only": most of them
        order = p.get("sort", "album")
        if order == "new":
            rows = sorted(rows, key=lambda r: ADDED.index(r["id"] - 1))
        elif order == "random":
            rows = random.sample(rows, len(rows))
        elif order == "artistalbum":
            rows = sorted(rows, key=lambda r: (r["artist"], r["album"]))
        elif order == "yearalbum":
            rows = sorted(rows, key=lambda r: (r["year"], r["album"]))
        else:
            rows = sorted(rows, key=lambda r: r["album"])
        return page(rows, rest, "albums_loop")
    if op == "artists":
        rows = [{"id": i + 1, "artist": a} for i, a in enumerate(ARTISTS)]
        role = p.get("role_id")
        if role == "ALBUMARTIST":
            rows = [r for r in rows if r["artist"] != "Various Artists"]
        elif role == "COMPOSER":
            rows = [r for r in rows if r["artist"] in COMPOSERS]
        elif role == "CONDUCTOR":
            rows = [r for r in rows if r["artist"] in CONDUCTORS]
        elif role in ("BAND", "TRACKARTIST"):
            rows = []
        if "genre_id" in p:
            names = {a[1] for i, a in enumerate(ALBUMS) if str(ALBUM_GENRE[i] + 1) == p["genre_id"]}
            rows = [r for r in rows if r["artist"] in names]
        if "search" in p:
            rows = [r for r in rows if p["search"].lower() in r["artist"].lower()]
        return page(rows, rest, "artists_loop")
    if op == "genres":
        rows = [{"id": i + 1, "genre": g, "favorites_url": f"db:genre.name={g}"} for i, g in enumerate(GENRES)]
        if "genre_id" in p:
            rows = [r for r in rows if str(r["id"]) == p["genre_id"]]
        if "search" in p:
            rows = [r for r in rows if p["search"].lower() in r["genre"].lower()]
        return page(rows, rest, "genres_loop")
    if op == "years":
        rows = [{"year": y} for y in sorted({a[2] for a in ALBUMS}, reverse=True)]
        return page(rows, rest, "years_loop")
    if op == "musicfolder":
        rows = FOLDERS.get(int(p.get("folder_id", 0)), [])
        return page(rows, rest, "folder_loop")
    if op == "libraries":
        return {"folder_loop": LIBRARIES}
    if op in ("titles", "tracks"):
        rows = TRACKS
        if "track_id" in p:
            # The gateway's lookup before a download or an analysis: a library file.
            rows = [{**t, "type": "wav", "url": f"file:///srv/music/demo/{t['id']}.wav",
                     "filesize": len(audio(t["id"])), "modificationTime": "Monday, 1 January 2024"}
                    for t in rows if str(t["id"]) == p["track_id"]]
        if "album_id" in p:
            rows = [t for t in rows if str(t["album_id"]) == p["album_id"]]
        if "search" in p:
            rows = [t for t in rows if p["search"].lower() in t["title"].lower()]
        return page(rows, rest, "titles_loop")
    if op == "search":
        term = p.get("term", "").lower()
        hits = lambda xs: [x for x in xs if term in x.lower()]
        return {"count": 0, "albums_count": len(hits([a[0] for a in ALBUMS])),
                "albums_loop": [{"album_id": i + 1, "album": a[0]} for i, a in enumerate(ALBUMS) if term in a[0].lower()],
                "contributors_loop": [{"contributor_id": i + 1, "contributor": a} for i, a in enumerate(ARTISTS) if term in a.lower()],
                "tracks_loop": [{"track_id": t["id"], "track": t["title"]} for t in TRACKS if term in t["title"].lower()][:20]}
    if op == "status":
        return status(player)
    if op == "favorites":
        items = [
            {"id": "f1.0", "name": "Night Ferry", "type": "playlist", "isaudio": 1, "hasitems": 1, "image": "music/1001/cover"},
            {"id": "f1.1", "name": "Morning Light", "type": "audio", "isaudio": 1, "hasitems": 0, "image": "music/1002/cover"},
            {"id": "f1.2", "name": "Community Radio", "type": "audio", "isaudio": 1, "hasitems": 0, "image": ICON("CommunityRadio")},
            {"id": "f1.3", "name": "Parallax", "type": "playlist", "isaudio": 1, "hasitems": 1, "image": "music/1007/cover"},
        ]
        return {"count": len(items), "title": "Favourites", "loop_loop": items}
    if op == "playlists":
        if rest and rest[0] == "tracks":
            return page(TRACKS[:9], rest[1:], "playlisttracks_loop")
        rows = [{"id": 501, "playlist": "Sunday Morning"}, {"id": 502, "playlist": "Road Trip"}, {"id": 503, "playlist": "Focus"}]
        return page(rows, rest, "playlists_loop")
    if op == "apps":
        return {"count": 3, "appss_loop": [
            {"cmd": "podcasts", "name": "Podcasts", "icon": ICON("Podcasts")},
            {"cmd": "musicshop", "name": "Music Shop", "icon": ICON("MusicShop")},
            {"cmd": "cloudtunes", "name": "Cloud Tunes", "icon": ICON("CloudTunes")}]}
    if op == "radios":
        return {"count": 2, "radioss_loop": [
            {"cmd": "communityradio", "name": "Community Radio", "icon": ICON("CommunityRadio")},
            {"cmd": "worldradio", "name": "World Radio", "icon": ICON("WorldRadio")}]}
    if op == "menu":
        return {"count": 4, "item_loop": [
            {"id": "pluginDiscoverMix", "node": "myMusic", "text": "Discover Mix", "icon": ICON("DiscoverMix"),
             "actions": {"go": {"cmd": ["discovermix", "menu"], "player": 0}}},
            {"id": "randomplay", "node": "myMusic", "text": "Random Mix", "isANode": 1},
            {"id": "randomtracks", "node": "randomplay", "text": "Song Mix", "nextWindow": "nowPlaying",
             "actions": {"go": {"cmd": ["randomplay", "tracks"]}}},
            {"id": "randomalbums", "node": "randomplay", "text": "Album Mix", "nextWindow": "nowPlaying",
             "actions": {"go": {"cmd": ["randomplay", "albums"]}}}]}
    if op in ("podcasts", "musicshop", "cloudtunes", "communityradio", "worldradio", "discovermix"):
        names = {"discovermix": ["Similar to the current track", "Late-night mood", "Energetic", "Calm"],
                 "communityradio": ["Local stations", "By genre", "Most popular"],
                 "worldradio": ["Europe", "Asia", "Americas", "Search"]}.get(op, ["Featured", "New", "Search"])
        return {"count": len(names), "item_loop": [
            {"text": n, "actions": {"go": {"cmd": [op, "items"], "params": {"item_id": f"0.{i}", "menu": op}}}}
            for i, n in enumerate(names)]}
    if op in ("trackinfo", "albuminfo", "artistinfo", "genreinfo", "yearinfo", "folderinfo"):
        items = [
            {"text": "Play next", "actions": {"go": {"cmd": ["playlistcontrol"], "params": {"cmd": "insert", "track_id": "103"}}}, "nextWindow": "parent"},
            {"text": "Add to queue", "actions": {"go": {"cmd": ["playlistcontrol"], "params": {"cmd": "add", "track_id": "103"}}}, "nextWindow": "parent"},
            {"text": "Discover Mix: similar tracks", "nextWindow": "nowPlaying", "actions": {"go": {"cmd": ["discovermix", "similar"], "params": {"track_id": "103"}}}},
            {"text": "Album: Night Ferry", "actions": {"go": {"cmd": ["albuminfo", "items"], "params": {"album_id": "1", "menu": "1"}}}},
            {"text": "Artist: The Paper Lanterns", "actions": {"go": {"cmd": ["artistinfo", "items"], "params": {"artist_id": "1", "menu": "1"}}}},
            {"text": "Genre: Indie Folk", "type": "text"},
            {"text": "Year: 2019", "type": "text"},
        ]
        return {"count": len(items), "item_loop": items}
    if op == "mixer" and rest[:1] == ["volume"] and len(rest) > 1 and rest[1] != "?":
        STATE["volume"] = max(0, min(100, int(float(rest[1]))))
    if op == "pause":
        STATE["mode"] = "pause" if STATE["mode"] == "play" else "play"
    return {}


RATE = 8000
_AUDIO = {}


def audio(tid):
    """A track as a mono 16-bit WAV: noise shaped into quiet and loud sections, with a fade
    in and out, the same every time (so its waveform looks like music)."""
    if tid in _AUDIO:
        return _AUDIO[tid]
    row = next(t for t in TRACKS if t["id"] == tid)
    rnd = random.Random(tid)
    n = int(row["duration"] * RATE)
    levels, t = [], 0
    while t < n:
        length = int(RATE * rnd.uniform(6, 24))
        levels.append((t, t + length, rnd.uniform(0.15, 0.9)))
        t += length
    out = bytearray()
    sect = 0
    for i in range(n):
        while levels[sect][1] <= i:
            sect += 1
        sec = i / RATE
        env = levels[sect][2] * (0.65 + 0.35 * abs(math.sin(math.pi * 2 * sec)))
        env *= min(1.0, sec / 3.0, (n - i) / (8.0 * RATE))
        out += struct.pack("<h", int(max(-1.0, min(1.0, rnd.gauss(0, 0.3) * env)) * 32767))
    head = b"RIFF" + struct.pack("<I", 36 + len(out)) + b"WAVEfmt " + struct.pack(
        "<IHHIIHH", 16, 1, 1, RATE, RATE * 2, 2, 16) + b"data" + struct.pack("<I", len(out))
    _AUDIO[tid] = bytes(head + out)
    return _AUDIO[tid]


def cover(key, size):
    """Abstract artwork, the same every time for the same key."""
    h = hashlib.sha256(key.encode()).digest()
    hue = h[0] / 255
    c1 = tuple(int(x * 255) for x in colorsys.hls_to_rgb(hue, 0.42, 0.65))
    c2 = tuple(int(x * 255) for x in colorsys.hls_to_rgb((hue + 0.12 + h[1] / 1200) % 1, 0.18, 0.55))
    img = Image.new("RGB", (size, size), c2)
    d = ImageDraw.Draw(img)
    for y in range(size):
        t = y / size
        d.line([(0, y), (size, y)], fill=tuple(int(c1[i] * (1 - t) + c2[i] * t) for i in range(3)))
    for i in range(3 + h[2] % 4):
        r = size * (0.12 + h[3 + i] / 255 * 0.35)
        x, y = size * h[8 + i] / 255, size * h[12 + i] / 255
        tint = tuple(int(x * 255) for x in colorsys.hls_to_rgb((hue + 0.5 + i * 0.07) % 1, 0.6 + i * 0.05, 0.5))
        d.ellipse([x - r, y - r, x + r, y + r], outline=tint, width=max(2, size // 60))
    img = img.filter(ImageFilter.GaussianBlur(size / 400))
    out = io.BytesIO()
    img.save(out, "PNG")
    return out.getvalue()


class Handler(BaseHTTPRequestHandler):
    def log_message(self, *a):
        pass

    def _send(self, body, ctype):
        self.send_response(200)
        self.send_header("Content-Type", ctype)
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    def do_POST(self):
        req = json.loads(self.rfile.read(int(self.headers.get("Content-Length", 0))) or b"{}")
        player, cmd = req.get("params", ["", []])
        self._send(json.dumps({"id": req.get("id"), "method": "slim.request", "params": [player, cmd],
                               "result": answer(player, cmd)}).encode(), "application/json")

    def do_GET(self):
        m = re.match(r"^/music/(\w+)/cover(?:_(\d+)x\d+)?", self.path)
        if m:
            return self._send(cover(m.group(1), min(int(m.group(2) or 300), 800)), "image/png")
        m = re.match(r"^/music/(\d+)/download$", self.path)
        if m and any(t["id"] == int(m.group(1)) for t in TRACKS):
            return self._send(audio(int(m.group(1))), "audio/wav")
        m = re.match(r"^/plugins/(\w+)/html/", self.path)
        if m:
            return self._send(cover("icon-" + m.group(1), 160), "image/png")
        self.send_error(404)


if __name__ == "__main__":
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument("--port", type=int, default=19000)
    a = ap.parse_args()
    ThreadingHTTPServer(("127.0.0.1", a.port), Handler).serve_forever()
