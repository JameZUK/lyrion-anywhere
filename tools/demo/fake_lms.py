#!/usr/bin/env python3
"""A stand-in Lyrion Music Server with an invented library, for demos and screenshots.

Every artist, album, track and player here is made up, and the cover art is generated, so
screenshots taken against it show nothing from a real installation. It answers the JSON-RPC
commands the web app uses (read-only: playback commands are accepted and ignored), serves
artwork, and serves each track as generated audio (a WAV of shaped noise) so the gateway's
track analysis has something to analyse. It is not a test double for the gateway's security tests (those live in
gateway/tests).

It also speaks just enough Slimproto for the browser's own player to play through the
gateway (with --slimproto-port): a player that says HELO is listed, given the "Night Ferry"
queue (24-bit/96 kHz) and sent a `strm` for its current track, served as FLAC made with
ffmpeg, so the gateway's live re-encoding to Opus can be shown. Seeks and track changes start
the stream again.

    python3 tools/demo/fake_lms.py --port 19000 --slimproto-port 13483

Needs Pillow (python3 -m pip install pillow) for the generated artwork.
"""
import argparse
import colorsys
import hashlib
import html
import io
import json
import math
import shutil
import socket
import subprocess
import threading
import random
import re
import struct
import time
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path
from urllib.parse import unquote

from PIL import Image, ImageDraw, ImageFilter, ImageFont

from demo_content import BIOS, LYRICS, REVIEWS

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
# Invented listening statistics (LMS's server-wide play counts and last played), for Home's
# Recently played and Most played shelves and the stations: albums by position in ALBUMS,
# most recently played first.
LAST_PLAYED = [6, 0, 3, 7, 1, 10, 4]


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
LOUNGE, STUDY, GARDEN, KITCHEN, BEDROOM = (f"aa:bb:cc:00:00:0{n}" for n in range(1, 6))
PLAYERS = [
    {"playerid": LOUNGE, "name": "Lounge", "model": "squeezelite", "modelname": "Squeezelite", "connected": 1, "power": 1, "isplaying": 1},
    {"playerid": KITCHEN, "name": "Kitchen", "model": "squeezelite", "modelname": "Squeezelite", "connected": 1, "power": 1, "isplaying": 1},
    {"playerid": STUDY, "name": "Study", "model": "squeezelite", "modelname": "Squeezelite", "connected": 1, "power": 1, "isplaying": 0},
    {"playerid": BEDROOM, "name": "Bedroom", "model": "squeezelite", "modelname": "Squeezelite", "connected": 1, "power": 1, "isplaying": 0},
    {"playerid": GARDEN, "name": "Garden Room", "model": "squeezelite", "modelname": "Squeezelite", "connected": 1, "power": 0, "isplaying": 0},
]
# The Kitchen plays in step with the Lounge (an LMS sync group: one shared queue).
SYNC = {KITCHEN: LOUNGE}


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


def has_role(artist, role):
    """Does the artist appear in this role (LMS's role_id)? Everyone is an album artist and
    artist; a few are composers or a conductor too; nobody is a band or a track artist."""
    if role in (None, "ALBUMARTIST", "ARTIST"):
        return True
    if role == "COMPOSER":
        return artist in COMPOSERS
    if role == "CONDUCTOR":
        return artist in CONDUCTORS
    return False


def play_count(t):
    """An invented play count: the recently played albums' tracks have been played most."""
    a = t["album_id"] - 1
    base = (len(LAST_PLAYED) - LAST_PLAYED.index(a)) * 3 if a in LAST_PLAYED else 0
    return base + int(hashlib.sha1(t["title"].encode()).hexdigest(), 16) % 4 if base else 0


TRACKS = track_rows()
for _t in TRACKS:
    _t["playcount"] = play_count(_t)
FOLDERS = folder_tree()
# Each player's queue and transport. The Lounge is playing "Night Ferry", a high-resolution
# release (for the quality badge); the Kitchen shares its queue (sync); the Study is paused
# part-way through "Glasshouse"; the Bedroom is stopped; the Garden Room is switched off.
HIRES = {"samplerate": "96000", "samplesize": "24", "bitrate": "3000kbps VBR"}


def album_tracks(album_id, **extra):
    return [{**t, **extra} for t in TRACKS if t["album_id"] == album_id]


STATES = {
    LOUNGE: {"queue": album_tracks(1, **HIRES), "index": 2, "time": 83.4, "mode": "play", "volume": 42},
    STUDY: {"queue": album_tracks(2), "index": 4, "time": 131.0, "mode": "pause", "volume": 25},
    BEDROOM: {"queue": album_tracks(6), "index": 0, "time": 0.0, "mode": "stop", "volume": 18},
    GARDEN: {"queue": album_tracks(9), "index": 1, "time": 0.0, "mode": "stop", "volume": 50, "power": 0},
}
for _s in STATES.values():
    for _k, _v in {"shuffle": 0, "repeat": 0, "sleep_at": 0, "power": 1}.items():
        _s.setdefault(_k, _v)
VOLUMES = {KITCHEN: 35}


def album_row(i):
    album, artist, year, _ = ALBUMS[i]
    return {"id": i + 1, "album": album, "artist": artist, "artist_id": ARTISTS.index(artist) + 1,
            "year": year, "artwork_track_id": str(1001 + i)}


def state_of(player):
    """The state a player's queue commands act on (a synced player shares its master's)."""
    return STATES.get(SYNC.get(player, player))


def sync_fields(player):
    master = SYNC.get(player, player)
    slaves = [p for p, m in SYNC.items() if m == master]
    if not slaves:
        return {}
    return {"sync_master": master, "sync_slaves": ",".join(slaves)}


def status(player, args=()):
    st = state_of(player)
    info = next((p for p in PLAYERS if p["playerid"] == player), None)
    if st is None or info is None:
        return {}
    base = {"player_name": info["name"], "player_connected": 1, "power": st["power"],
            "mixer volume": VOLUMES.get(player, st["volume"]), "playlist shuffle": st["shuffle"],
            "playlist repeat": st["repeat"], "can_seek": 1, **sync_fields(player)}
    queue = st["queue"]
    if not queue:
        return {**base, "mode": "stop", "playlist_tracks": 0}
    st["index"] = max(0, min(st["index"], len(queue) - 1))
    cur = queue[st["index"]]
    # `status - <n>` starts at the current track; `status <start> <n>` pages the queue.
    args = [str(a) for a in args]
    start = st["index"] if args[:1] == ["-"] else int(args[0]) if args[:1] and args[0].isdigit() else 0
    count = int(args[1]) if len(args) > 1 and args[1].isdigit() else len(queue)
    rows = [{**t, "playlist index": i} for i, t in enumerate(queue)][start:start + count]
    # A browser player's position moves on while it plays (the others' stay put, for the shots).
    elapsed = st["time"] + (time.time() - st["since"] if "since" in st and st["mode"] == "play" else 0)
    out = {**base, "mode": st["mode"], "time": round(min(elapsed, cur["duration"]), 1), "duration": cur["duration"],
           "playlist_cur_index": str(st["index"]), "playlist_tracks": len(queue), "playlist_loop": rows}
    left = st["sleep_at"] - time.time()
    if left > 0:
        out["will_sleep_in"] = round(left, 1)
    return out


def selected_tracks(p):
    """The tracks a playlistcontrol command names."""
    if "track_id" in p:
        wanted = [int(x) for x in p["track_id"].split(",") if x.isdigit()]
        return [t for w in wanted for t in TRACKS if t["id"] == w]
    for key in ("album_id", "artist_id"):
        if key in p:
            wanted = [int(x) for x in p[key].split(",") if x.isdigit()]
            return [t for t in TRACKS if t[key] in wanted]
    if "genre_id" in p:
        return [t for t in TRACKS if str(ALBUM_GENRE[t["album_id"] - 1] + 1) == p["genre_id"]]
    if "year" in p:
        return [t for t in TRACKS if str(t["year"]) == p["year"]]
    if "playlist_id" in p:
        return TRACKS[:9]
    return []


def queue_command(player, c, p):
    """Queue and transport commands, applied to the player's (or its group's) state.
    Returns whether the command was one of these."""
    st = state_of(player)
    if st is None:
        return False
    op, rest = c[0], c[1:]
    q = st["queue"]
    if op == "playlistcontrol":
        tracks = selected_tracks(p)
        how = p.get("cmd")
        if how == "load":
            st["queue"] = list(tracks)
            st["index"] = int(p["play_index"]) if p.get("play_index", "").isdigit() else 0
            st["time"], st["mode"] = 0.0, "play"
        elif how == "insert":
            at = st["index"] + 1 if q else 0
            st["queue"] = q[:at] + tracks + q[at:]
        elif how == "add":
            st["queue"] = q + tracks
        return True
    if op == "playlist" and rest:
        sub = rest[0]
        if sub == "index" and len(rest) > 1:
            v = rest[1]
            n = st["index"] + int(v) if v[:1] in "+-" and v[1:].isdigit() else int(v) if v.isdigit() else st["index"]
            st["index"], st["time"] = max(0, min(n, len(q) - 1)), 0.0
        elif sub == "delete" and len(rest) > 1 and rest[1].isdigit():
            i = int(rest[1])
            if 0 <= i < len(q):
                del q[i]
                if i < st["index"]:
                    st["index"] -= 1
        elif sub == "move" and len(rest) > 2 and rest[1].isdigit() and rest[2].isdigit():
            i, j = int(rest[1]), int(rest[2])
            if 0 <= i < len(q) and 0 <= j < len(q):
                q.insert(j, q.pop(i))
        elif sub == "clear":
            st["queue"], st["index"], st["mode"] = [], 0, "stop"
        elif sub in ("shuffle", "repeat") and len(rest) > 1 and rest[1].isdigit():
            st[sub] = int(rest[1])
        return True
    if op == "play":
        st["mode"] = "play"
    elif op == "stop":
        st["mode"] = "stop"
    elif op == "pause":
        if rest and rest[0] in ("0", "1"):
            st["mode"] = "pause" if rest[0] == "1" else "play"
        else:
            st["mode"] = "pause" if st["mode"] == "play" else "play"
    elif op == "time" and rest:
        try:
            st["time"] = max(0.0, float(rest[0]))
        except ValueError:
            pass
    elif op == "power" and rest and rest[0] in ("0", "1"):
        st["power"] = int(rest[0])
    elif op == "mixer" and rest[:1] == ["volume"] and len(rest) > 1 and rest[1] != "?":
        try:
            v = max(0, min(100, int(float(rest[1]))))
        except ValueError:
            return True
        if player in VOLUMES:
            VOLUMES[player] = v
        else:
            st["volume"] = v
    elif op == "sleep" and rest and rest[0].isdigit():
        st["sleep_at"] = time.time() + int(rest[0]) if int(rest[0]) > 0 else 0
    else:
        return False
    return True


def with_index(out, names, c):
    """LMS's A–Z index (`indexList`: each first letter and how many start with it), when
    asked for with `tags:…Z…`."""
    if not any(x.startswith("tags:") and "Z" in x for x in c):
        return out
    index = []
    for n in names:
        k = n[:1].upper()
        k = {"É": "E"}.get(k, k)
        if index and index[-1][0] == k:
            index[-1][1] += 1
        else:
            index.append([k, 1])
    return {**out, "indexList": index}


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
            wanted = p["album_id"].split(",")
            rows = [r for r in rows if str(r["id"]) in wanted]
        if "artist_id" in p:
            rows = [r for r in rows if str(r["artist_id"]) == p["artist_id"]]
            rows = [r for r in rows if has_role(r["artist"], p.get("role_id"))]
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
        elif order == "recentlyplayed":
            rows = sorted((r for r in rows if r["id"] - 1 in LAST_PLAYED), key=lambda r: LAST_PLAYED.index(r["id"] - 1))
        elif order == "playcount":
            plays = lambda r: sum(t["playcount"] for t in TRACKS if t["album_id"] == r["id"])
            rows = sorted((r for r in rows if plays(r)), key=lambda r: -plays(r))
        elif order == "random":
            rows = random.sample(rows, len(rows))
        elif order == "artistalbum":
            rows = sorted(rows, key=lambda r: (r["artist"], r["album"]))
        elif order == "yearalbum":
            rows = sorted(rows, key=lambda r: (r["year"], r["album"]))
        else:
            rows = sorted(rows, key=lambda r: r["album"])
        return with_index(page(rows, rest, "albums_loop"), [r["album"] for r in rows], c)
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
        if "artist_id" in p:
            rows = [r for r in rows if str(r["id"]) == p["artist_id"]]
        if p.get("sort") == "playcount":
            plays = lambda r: sum(t["playcount"] for t in TRACKS if t["artist"] == r["artist"])
            rows = sorted((r for r in rows if plays(r)), key=lambda r: -plays(r))
        return with_index(page(rows, rest, "artists_loop"), [r["artist"] for r in rows], c)
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
            # The gateway's lookup before a download or an analysis: a library file. Lists of
            # ids (history, stations) are answered from the library too.
            wanted = p["track_id"].split(",")
            rows = [{**t, "type": "wav", "url": f"file:///srv/music/demo/{t['id']}.wav",
                     "filesize": 44 + int(t["duration"] * RATE) * 2, "modificationTime": "Monday, 1 January 2024",
                     **({"lyrics": LYRICS[t["title"]]} if t["album_id"] == 1 and t["title"] in LYRICS else {})}
                    for t in rows if str(t["id"]) in wanted]
        if "album_id" in p:
            wanted = p["album_id"].split(",")
            rows = [t for t in rows if str(t["album_id"]) in wanted]
        if "artist_id" in p:
            wanted = p["artist_id"].split(",")
            rows = [t for t in rows if str(t["artist_id"]) in wanted and has_role(t["artist"], p.get("role_id"))]
        if "genre_id" in p:
            wanted = p["genre_id"].split(",")
            rows = [t for t in rows if str(ALBUM_GENRE[t["album_id"] - 1] + 1) in wanted]
        if "year" in p:
            rows = [t for t in rows if str(t["year"]) == p["year"]]
        if "search" in p:
            rows = [t for t in rows if p["search"].lower() in t["title"].lower()]
        return page(rows, rest, "titles_loop")
    if op == "search":
        term = p.get("term", "").lower()
        albums = [{"album_id": r["id"], "album": r["album"], "artist": r["artist"], "artwork_track_id": r["artwork_track_id"]}
                  for r in (album_row(i) for i in range(len(ALBUMS))) if term in r["album"].lower()]
        artists = [{"contributor_id": i + 1, "contributor": a} for i, a in enumerate(ARTISTS) if term in a.lower()]
        tracks = [{"track_id": t["id"], "track": t["title"], "artist": t["artist"], "album": t["album"], "coverid": t["coverid"]}
                  for t in TRACKS if term in t["title"].lower()]
        return {"count": len(albums) + len(artists) + len(tracks), "albums_count": len(albums), "albums_loop": albums,
                "contributors_count": len(artists), "contributors_loop": artists,
                "tracks_count": len(tracks), "tracks_loop": tracks[:20]}
    if op == "status":
        return status(player, rest)
    if op == "syncgroups":
        groups = {}
        for slave, master in SYNC.items():
            groups.setdefault(master, [master]).append(slave)
        names = {p["playerid"]: p["name"] for p in PLAYERS}
        return {"syncgroups_loop": [{"sync_members": ",".join(m), "sync_member_names": ",".join(names[x] for x in m)}
                                    for m in groups.values()]}
    if queue_command(player, c, p):
        slim_follow(player, op, rest)
        return {}
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
        rows = [{"id": 501, "playlist": "Sunday Morning"}, {"id": 502, "playlist": "Road Trip"}, {"id": 503, "playlist": "Focus"},
                {"id": 504, "playlist": "Late Hours"}]
        if "search" in p:
            rows = [r for r in rows if p["search"].lower() in r["playlist"].lower()]
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
    if op == "discovermix" and rest[:1] == ["items"] and p.get("item_id", "").startswith("0."):
        # One of Discover Mix's moods: mixes that play at once (and can be added to).
        mood = p["item_id"]
        names = ["Mix 1", "Mix 2", "Mix 3"]
        return {"count": len(names), "base": {"actions": {
                    "add": {"cmd": ["discovermix", "add"], "itemsParams": "params", "player": 0}}},
                "item_loop": [{"text": f"{n}", "nextWindow": "nowPlaying", "params": {"mix": f"{mood}.{i}"},
                               "actions": {"go": {"cmd": ["discovermix", "play"], "params": {"mix": f"{mood}.{i}"}}}}
                              for i, n in enumerate(names)]}
    if op == "anywhere" and rest[:1] == ["play_record"]:
        # The Lyrion Anywhere plugin counting an offline play (the demo takes every one).
        return {"ok": 1}
    if op == "anywhere" and rest:
        # The plugin's settings page (tools/demo/admin.html), with invented users and devices.
        return admin_answer(rest[0], p)
    if op == "musicartistinfo" and rest:
        # The Music & Artist Information plugin: biographies, photos, reviews (invented).
        what = rest[0]
        if what in ("biography", "artistphoto") and p.get("artist_id", "").isdigit():
            i = int(p["artist_id"]) - 1
            name = ARTISTS[i] if 0 <= i < len(ARTISTS) else None
            if what == "biography":
                return {"biography": BIOS[name]} if name in BIOS else {}
            if name in BIOS:
                return {"url": f"https://images.example.com/artists/{i + 1}.jpg",
                        "credits": "generated for the demo"}
            return {}
        if what == "albumreview" and p.get("album_id", "").isdigit():
            i = int(p["album_id"]) - 1
            title = ALBUMS[i][0] if 0 <= i < len(ALBUMS) else None
            return {"albumreview": REVIEWS[title]} if title in REVIEWS else {}
        if what == "lyrics":
            return {}
        return {}
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
    return {}


def admin_answer(op, p):
    """`anywhere <op>` for the plugin's settings page: invented users, devices and activity."""
    now = int(time.time())
    if op == "status":
        return {"enabled": 1, "running": 1, "pid": 4242, "uptime": 3 * 86400 + 5 * 3600 + 17 * 60,
                "listen": "127.0.0.1:9443", "binary_ok": 1, "binary": "Bin/x86_64-linux/anywhere-gateway",
                "data_dir": "/srv/lms/prefs/anywhere"}
    if op == "users":
        return {"data": {"users": [
            {"username": "demo", "role": "admin", "totp_enabled": True, "disabled": False,
             "downloads": True, "download_formats": ["original", "mp3", "opus"]},
            {"username": "alex", "role": "listener", "totp_enabled": False, "disabled": False,
             "downloads": True, "download_formats": ["opus"]},
            {"username": "sam", "role": "listener", "totp_enabled": True, "disabled": False, "downloads": False},
            {"username": "guest", "role": "listener", "totp_enabled": False, "disabled": True, "downloads": False},
        ]}}
    if op == "grants":
        return {"data": {"players": {"alex": [STUDY], "sam": [BEDROOM, GARDEN]}.get(p.get("username"), [])}}
    if op == "devices":
        ua = "Mozilla/5.0 (demo browser)"
        return {"data": {"devices": [
            {"id": 11, "name": "Demo's phone", "username": "demo", "client_kind": "android",
             "client_platform": "Android 15", "last_addr": "203.0.113.24", "last_seen": now - 120,
             "mac": "02:4c:41:3a:91:0c"},
            {"id": 12, "name": "Kitchen tablet", "username": "demo", "client_kind": "pwa",
             "client_platform": "Chrome on Android", "user_agent": ua, "last_addr": "198.51.100.7",
             "last_seen": now - 3600 * 5, "mac": "02:4c:41:5e:27:b4"},
            {"id": 14, "name": "Alex's laptop", "username": "alex", "client_kind": "browser",
             "client_platform": "Firefox on Linux", "user_agent": ua, "last_addr": "198.51.100.42",
             "last_seen": now - 86400, "mac": "02:4c:41:77:10:e2"},
            {"id": 15, "name": "Sam's phone", "username": "sam", "client_kind": "pwa",
             "client_platform": "Safari on iOS", "user_agent": ua, "last_addr": "203.0.113.90",
             "last_seen": now - 86400 * 3, "mac": "02:4c:41:c4:08:6d"},
        ]}}
    if op == "audit":
        rows = [(300, "login.ok", "demo", "Demo's phone"), (3600 * 2, "download", "alex", "Night Ferry (Opus)"),
                (3600 * 5, "pairing.ok", "demo", "Kitchen tablet"), (3600 * 5 + 60, "pairing.created", "demo", ""),
                (86400, "login.ok", "alex", "Alex's laptop"), (86400 + 600, "login.failed", "alex", ""),
                (86400 * 2, "plays", "sam", "12 plays"), (86400 * 3, "player.granted", "alex", "Study"),
                (86400 * 4, "user.disabled", "guest", ""), (86400 * 6, "totp.enabled", "sam", "")]
        return {"data": {"audit": [{"at": now - a, "event": e, "username": u, "detail": d} for a, e, u, d in rows]}}
    if op == "settings":
        return {"data": {"session_days": 90, "session_days_default": 90}}
    if op == "plugins":
        return {"data": {"plugins": [
            {"command": "podcasts", "name": "Podcasts", "kind": "apps"},
            {"command": "communityradio", "name": "Community Radio", "kind": "radios"},
            {"command": "discovermix", "name": "Discover Mix", "kind": "mymusic"},
            {"commands": ["randomplay"], "name": "Random Mix", "kind": "mymusic", "admin_only": False},
        ]}}
    return {"data": {}}


def history_plays(now):
    """Invented listening for the demo user over the last year: a JSON list of plays for
    `anywhere-gateway admin demo-seed-history` (evenings and weekends, favourite albums
    played most, a streak or two, a few new artists lately)."""
    rnd = random.Random(20240601)
    # Albums by position, with how much each is liked (the first few are favourites).
    likes = {0: 10, 6: 8, 3: 7, 1: 6, 7: 5, 10: 4, 4: 4, 8: 3, 2: 3, 5: 2, 9: 1, 11: 1}
    # Artists first heard recently (their albums only played in the last weeks).
    late = {10: 40, 8: 75}
    day0 = now - now % 86400
    plays = []
    for d in range(380, -1, -1):
        day = day0 - d * 86400
        weekday = (day // 86400 + 3) % 7  # 0 = Monday
        if rnd.random() < (0.12 if d > 200 else 0.06):
            continue  # a day off
        sessions = rnd.choice([1, 1, 2, 2, 3]) + (1 if weekday >= 5 else 0)
        for _ in range(sessions):
            hour = rnd.choice([7, 8, 12, 13, 17, 18, 19, 20, 20, 21, 21, 22, 23] + ([10, 11, 15, 16] if weekday >= 5 else []))
            t = day + hour * 3600 + rnd.randrange(3600)
            pool = [a for a in likes if not (a in late and d > late[a])]
            album = rnd.choices(pool, weights=[likes[a] for a in pool])[0]
            tracks = [x for x in TRACKS if x["album_id"] == album + 1]
            start = rnd.randrange(len(tracks)) if rnd.random() < 0.3 else 0
            for x in tracks[start:start + rnd.randint(3, len(tracks))]:
                if t > now - 60:
                    break
                heard = x["duration"] if rnd.random() < 0.9 else x["duration"] * rnd.uniform(0.5, 0.95)
                plays.append({"track_id": x["id"], "album_id": x["album_id"], "artist_id": x["artist_id"],
                              "year": x["year"], "played_at": int(t), "seconds": round(heard, 1)})
                t += int(heard) + rnd.randint(1, 4)
    return plays


PLUGIN_HTML = Path(__file__).resolve().parents[2] / "plugin/Plugins/LyrionAnywhere/HTML/EN/plugins/LyrionAnywhere"
# What the settings template is given (Template Toolkit names), with invented values.
TT_VALUES = {
    "anywhere_assets": "demo", "anywhere_csrf": "0123456789abcdef0123456789abcdef",
    "anywhere_public_url": "https://music.example.com", "prefs.pref_enabled": "1",
    "prefs.pref_listen": "127.0.0.1:9443", "prefs.pref_public_url": "https://music.example.com",
    "prefs.pref_trusted_proxies": "", "prefs.pref_client_ip_header": "", "prefs.pref_gateway_path": "",
    "prefs.pref_transcode": "1", "prefs.pref_ffmpeg_path": "", "prefs.pref_download_cache_mb": "2048",
    "prefs.pref_analysis": "1",
}
ADMIN_CHROME = """<!doctype html><html lang="en"><head><meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1"><title>Settings - Lyrion Anywhere</title>
<style>
body { margin: 0; font: 14px/1.45 system-ui, sans-serif; background: #f3f3f3; color: #222; }
.demo-top { background: #2d2d2d; color: #eee; padding: 10px 20px; font-weight: 600; }
.demo-tabs { background: #e4e4e4; padding: 8px 20px; border-bottom: 1px solid #ccc; }
.demo-tabs span { margin-right: 18px; color: #555; } .demo-tabs b { margin-right: 18px; }
.demo-page { padding: 16px 24px 40px; max-width: 1180px; }
.demo-setting { display: grid; grid-template-columns: 190px 1fr; gap: 4px 16px; padding: 8px 0; border-bottom: 1px solid #ddd; }
.demo-setting h4 { margin: 0; font-size: 14px; } .demo-setting .desc { margin: 0 0 4px; color: #666; }
.demo-setting > div { grid-column: 2; }
.stdedit { width: 320px; }
</style></head><body>
<div class="demo-top">Lyrion Music Server &middot; Settings (demo)</div>
<div class="demo-tabs"><span>Basic Settings</span><span>Player</span><span>Advanced</span><b>Lyrion Anywhere</b></div>
<div class="demo-page">
"""


def admin_page():
    """The plugin's own settings template, filled in as LMS would (a tiny subset of
    Template Toolkit: values, IF/UNLESS, the setting wrapper), in a plain stand-in for LMS's
    settings chrome."""
    src = (PLUGIN_HTML / "settings/basic.html").read_text()
    out, stack = [], []

    def on():
        return all(flag for _, flag in stack)

    def truthy(expr):
        expr = expr.strip()
        if expr.endswith(")") and ".match(" in expr:
            name, pat = expr.split(".match(", 1)
            return bool(re.search(pat.strip("()'").replace("(?i)", ""), TT_VALUES.get(name, ""), re.I))
        if " AND NOT " in expr:
            a, b = expr.split(" AND NOT ", 1)
            return truthy(a) and not truthy(b)
        return bool(TT_VALUES.get(expr))

    for i, part in enumerate(re.split(r"\[%(.*?)%\]", src, flags=re.S)):
        if i % 2 == 0:
            if on():
                out.append(part)
            continue
        d = part.strip()
        if d.startswith("PROCESS"):
            out.append(ADMIN_CHROME if "header" in d else "</div></body></html>")
        elif d.startswith("WRAPPER setting"):
            attrs = dict(re.findall(r'(\w+)="(.*?)"', d))
            stack.append(("wrapper", True))
            if on():
                out.append(f'<div class="demo-setting"><h4>{attrs.get("title", "")}</h4><div>'
                           f'<p class="desc">{attrs.get("desc", "")}</p>')
        elif d.startswith("IF "):
            stack.append(("if", truthy(d[3:])))
        elif d.startswith("UNLESS "):
            stack.append(("if", not truthy(d[7:])))
        elif d == "END":
            kind, _ = stack.pop()
            if kind == "wrapper" and on():
                out.append("</div></div>")
        elif on():
            name = d.split("|")[0].strip()
            out.append(html.escape(TT_VALUES.get(name, "")))
    return "".join(out).encode()


def artist_photo(key, size=600):
    """A made-up "photo": out-of-focus figures on a stage, lit from behind, with bokeh."""
    h = hashlib.sha256(key.encode()).digest()
    hue = h[0] / 255
    w, ht = size, int(size * 0.75)
    img = Image.new("RGB", (w, ht))
    d = ImageDraw.Draw(img)
    top, bottom = _rgb(hue, 0.32, 0.6), _rgb(hue + 0.06, 0.07, 0.45)
    for y in range(ht):
        t = y / ht
        d.line([(0, y), (w, y)], fill=tuple(int(top[i] * (1 - t) + bottom[i] * t) for i in range(3)))
    # Stage lights, out of focus.
    lights = Image.new("RGB", (w, ht), (0, 0, 0))
    g = ImageDraw.Draw(lights)
    for i in range(18):
        v = h[(i * 3) % 32]
        x, y = w * ((v * 37 + i * 53) % 256) / 255, ht * (0.05 + 0.45 * ((v * 11 + i * 29) % 256) / 255)
        r = w * (0.015 + 0.035 * ((v + i * 7) % 256) / 255)
        g.ellipse([x - r, y - r, x + r, y + r], fill=_rgb(hue + 0.45 + (i % 3) * 0.08, 0.62, 0.85))
    img = Image.blend(img, lights.filter(ImageFilter.GaussianBlur(w / 90)), 0.45)
    # A haze of light behind the figures.
    haze = Image.new("L", (w, ht), 0)
    ImageDraw.Draw(haze).ellipse([w * 0.15, ht * 0.05, w * 0.85, ht * 0.85], fill=150)
    img = Image.composite(Image.new("RGB", (w, ht), _rgb(hue + 0.1, 0.75, 0.6)), img,
                          haze.filter(ImageFilter.GaussianBlur(w / 8)))
    # The figures: soft silhouettes with a faint rim of light.
    people = 1 + h[10] % 3
    mask = Image.new("L", (w, ht), 0)
    m = ImageDraw.Draw(mask)
    for i in range(people):
        cx = w * (i + 1) / (people + 1) + (h[11 + i] - 128) / 255 * w * 0.08
        s = ht * (0.30 + 0.06 * h[14 + i] / 255) * (0.9 if people == 3 and i != 1 else 1.0)
        head_y = ht * 0.40 + (h[17 + i] % 20) / 20 * ht * 0.06
        m.ellipse([cx - s * 0.16, head_y, cx + s * 0.16, head_y + s * 0.42], fill=255)
        m.rectangle([cx - s * 0.08, head_y + s * 0.34, cx + s * 0.08, head_y + s * 0.56], fill=255)
        m.polygon([(cx - s * 0.14, head_y + s * 0.5), (cx + s * 0.14, head_y + s * 0.5),
                   (cx + s * 0.5, head_y + s * 0.62), (cx + s * 0.62, ht * 1.1), (cx - s * 0.62, ht * 1.1),
                   (cx - s * 0.5, head_y + s * 0.62)], fill=255)
    soft = mask.filter(ImageFilter.GaussianBlur(w / 160))
    rim = mask.filter(ImageFilter.GaussianBlur(w / 70))
    img = Image.composite(Image.new("RGB", (w, ht), _rgb(hue + 0.1, 0.8, 0.5)), img, rim.point(lambda v: int(v * 0.35)))
    img = Image.composite(Image.new("RGB", (w, ht), (14, 12, 18)), img, soft)
    out = io.BytesIO()
    img.save(out, "JPEG", quality=85)
    return out.getvalue()


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


def _rgb(h, l, s):
    return tuple(int(x * 255) for x in colorsys.hls_to_rgb(h % 1, l, s))


def _font(px):
    try:
        return ImageFont.load_default(size=max(8, int(px)))
    except (TypeError, OSError):
        return ImageFont.load_default()


def cover(key, size):
    """Generated artwork, the same every time for the same key: one of a few abstract
    styles, with the album's title and artist on it as on a real sleeve."""
    h = hashlib.sha256(key.encode()).digest()
    album = int(key) - 1001 if key.isdigit() and 0 <= int(key) - 1001 < len(ALBUMS) else None
    style = album % 5 if album is not None else h[5] % 5
    hue = h[0] / 255
    c1, c2 = _rgb(hue, 0.45, 0.65), _rgb(hue + 0.12 + h[1] / 1200, 0.16, 0.55)
    img = Image.new("RGB", (size, size), c2)
    d = ImageDraw.Draw(img)
    w = max(2, size // 60)
    for y in range(size):
        t = y / size
        d.line([(0, y), (size, y)], fill=tuple(int(c1[i] * (1 - t) + c2[i] * t) for i in range(3)))
    if style == 0:  # rings
        for i in range(3 + h[2] % 4):
            r = size * (0.12 + h[3 + i] / 255 * 0.35)
            x, y = size * h[8 + i] / 255, size * h[12 + i] / 255
            d.ellipse([x - r, y - r, x + r, y + r], outline=_rgb(hue + 0.5 + i * 0.07, 0.6 + i * 0.05, 0.5), width=w)
    elif style == 1:  # a low sun over a horizon of bands
        sun = _rgb(hue + 0.08, 0.68, 0.85)
        r = size * 0.22
        d.ellipse([size / 2 - r, size * 0.52 - r, size / 2 + r, size * 0.52 + r], fill=sun)
        for i in range(7):
            y = size * (0.58 + i * 0.06)
            d.rectangle([0, y, size, y + size * 0.03], fill=_rgb(hue + 0.55, 0.12 + i * 0.03, 0.4))
    elif style == 2:  # a grid of tiles
        n = 4 + h[2] % 3
        cell = size / n
        for gx in range(n):
            for gy in range(n):
                v = h[(gx * n + gy) % 32]
                if v % 3:
                    pad = cell * 0.12
                    d.rounded_rectangle([gx * cell + pad, gy * cell + pad, (gx + 1) * cell - pad, (gy + 1) * cell - pad],
                                        radius=int(cell * 0.2), fill=_rgb(hue + v / 900, 0.3 + (v % 7) * 0.06, 0.55))
    elif style == 3:  # waves
        for i in range(9):
            pts = [(x, size * (0.18 + i * 0.09) + math.sin(x / size * math.tau * (1 + h[3] % 3) + i * 0.6) * size * 0.04)
                   for x in range(0, size + 8, 8)]
            d.line(pts, fill=_rgb(hue + 0.5 + i * 0.02, 0.55 + i * 0.03, 0.6), width=w)
    else:  # stacked arcs
        for i in range(6):
            r = size * (0.15 + i * 0.12)
            d.arc([size * 0.15 - r, size * 1.0 - r, size * 0.15 + r, size * 1.0 + r], 270, 360,
                  fill=_rgb(hue + 0.4 + i * 0.05, 0.62, 0.7), width=w * 3)
    img = img.filter(ImageFilter.GaussianBlur(size / 500))
    if album is not None:
        title, artist = ALBUMS[album][0], ALBUMS[album][1]
        d = ImageDraw.Draw(img)
        pad = size * 0.06
        d.text((pad, pad), title, font=_font(size * 0.075), fill=(250, 248, 240))
        d.text((pad, pad + size * 0.095), artist, font=_font(size * 0.05), fill=(235, 232, 225))
    out = io.BytesIO()
    img.save(out, "JPEG", quality=88)
    return out.getvalue()


# --- Slimproto, for the browser's own player (through the gateway) -----------------------

HTTP_PORT = 19000
SLIM = {}  # player id -> its connection, for the browser players that said HELO
SLIM_LOCK = threading.Lock()
_FLAC = {}
_FLAC_LOCK = threading.Lock()


def slim_frame(op, payload):
    """A server-to-player frame: a 2-byte length (of opcode and payload), the opcode, the payload."""
    return struct.pack(">H", 4 + len(payload)) + op + payload


def slim_send(player, op, payload):
    with SLIM_LOCK:
        sock = SLIM.get(player)
    if sock:
        try:
            sock.sendall(slim_frame(op, payload))
        except OSError:
            pass


def strm(command, player=""):
    """A `strm`: `s` starts this player's stream of its current track (FLAC, whose PCM
    parameters the player reads from the stream); `q`, `p` and `u` stop, pause and unpause."""
    head = bytearray(24)
    head[0] = ord(command)
    head[1] = ord("1" if command == "s" else "0")  # autostart
    head[2] = ord("f")
    head[3:7] = b"????"
    head[7] = 255  # buffer threshold (KB)
    head[8] = ord("0")  # S/PDIF: auto
    head[10] = ord("0")  # no transition
    head[12] = 1  # output threshold
    head[18:20] = struct.pack(">H", HTTP_PORT)  # the IP stays 0: this server
    request = f"GET /stream.mp3?player={player} HTTP/1.0\r\n\r\n".encode() if command == "s" else b""
    return bytes(head) + request


def start_stream(player):
    """Start the player's stream of its current track from its position, as LMS does after a
    seek or a track change."""
    st = state_of(player)
    if not st["queue"] or st["mode"] != "play":
        return
    st["since"] = time.time()
    slim_send(player, b"strm", strm("q"))
    slim_send(player, b"strm", strm("s", player))


def slim_follow(player, op, rest):
    """A queue or transport command for a browser player: its stream starts, pauses or stops."""
    if player not in SLIM:
        return
    st = state_of(player)
    if op == "pause":
        if st["mode"] == "pause":
            st["time"] += time.time() - st.pop("since", time.time())
            return slim_send(player, b"strm", strm("p"))
        st["since"] = time.time()
        return slim_send(player, b"strm", strm("u"))
    if op == "stop" or (op == "playlist" and rest[:1] == ["clear"]):
        st.pop("since", None)
        return slim_send(player, b"strm", strm("q"))
    if op in ("time", "play", "playlistcontrol") or (op == "playlist" and rest[:1] == ["index"]):
        start_stream(player)


def flac(tid, seconds):
    """A track from `seconds` in, as 24-bit/96 kHz stereo FLAC made from its generated audio
    with ffmpeg, or None without ffmpeg."""
    ffmpeg = shutil.which("ffmpeg")
    if not ffmpeg:
        return None
    key = (tid, int(seconds))
    with _FLAC_LOCK:
        if key not in _FLAC:
            out = subprocess.run(
                [ffmpeg, "-v", "error", "-ss", str(int(seconds)), "-f", "wav", "-i", "pipe:0", "-ac", "2", "-ar", "96000",
                 "-c:a", "flac", "-sample_fmt", "s32", "-bits_per_raw_sample", "24", "-f", "flac", "pipe:1"],
                input=audio(tid), capture_output=True, check=True)
            _FLAC.clear()  # one at a time: they are large
            _FLAC[key] = out.stdout
        return _FLAC[key]


def slim_client(conn):
    """One player's connection: HELO lists the player (named by its SETD reply) and starts its
    stream, and its closing takes the player off the list; everything else it says is read and
    ignored."""
    player, buf = None, b""
    try:
        while data := conn.recv(65536):
            buf += data
            while len(buf) >= 8:
                op, n = buf[:4], struct.unpack(">I", buf[4:8])[0]
                if n > 1 << 20:
                    return
                if len(buf) < 8 + n:
                    break
                body, buf = buf[8:8 + n], buf[8 + n:]
                if op == b"HELO" and len(body) >= 8 and player is None:
                    player = ":".join(f"{b:02x}" for b in body[2:8])
                    with SLIM_LOCK:
                        SLIM[player] = conn
                    if not any(x["playerid"] == player for x in PLAYERS):
                        PLAYERS.append({"playerid": player, "name": "This browser", "model": "squeezelite",
                                        "modelname": "Lyrion Anywhere", "connected": 1, "power": 1, "isplaying": 1})
                    STATES.setdefault(player, {"queue": album_tracks(1, **HIRES), "index": 0, "time": 0.0, "mode": "play",
                                               "volume": 100, "shuffle": 0, "repeat": 0, "sleep_at": 0, "power": 1})
                    conn.sendall(slim_frame(b"setd", b"\x00"))  # what is the player called?
                    start_stream(player)
                elif op == b"SETD" and player and body[:1] == b"\x00" and len(body) > 1:
                    name = body[1:].split(b"\x00")[0].decode("utf-8", "replace")[:64]
                    for x in PLAYERS:
                        if x["playerid"] == player and name:
                            x["name"] = name
    except OSError:
        pass
    finally:
        with SLIM_LOCK:
            if player and SLIM.get(player) is conn:
                del SLIM[player]
                PLAYERS[:] = [x for x in PLAYERS if x["playerid"] != player]  # gone from the list, as LMS does
        conn.close()


def slim_serve(port):
    srv = socket.create_server(("127.0.0.1", port))
    while True:
        conn, _ = srv.accept()
        threading.Thread(target=slim_client, args=(conn,), daemon=True).start()


class Handler(BaseHTTPRequestHandler):
    def log_message(self, *a):
        pass

    def _send(self, body, ctype):
        self.send_response(200)
        self.send_header("Content-Type", ctype)
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        try:
            self.wfile.write(body)
        except (BrokenPipeError, ConnectionResetError):
            pass  # the client went away (a page moving on); nothing to do

    def do_POST(self):
        req = json.loads(self.rfile.read(int(self.headers.get("Content-Length", 0))) or b"{}")
        player, cmd = req.get("params", ["", []])
        self._send(json.dumps({"id": req.get("id"), "method": "slim.request", "params": [player, cmd],
                               "result": answer(player, cmd)}).encode(), "application/json")

    def do_GET(self):
        m = re.match(r"^/music/(\w+)/cover(?:_(\d+)x\d+)?", self.path)
        if m:
            key = m.group(1)
            if key == "current":
                # The player's current track (the gateway asks with ?player=<id>).
                q = re.search(r"player=([0-9a-fA-F:%]+)", self.path)
                st = state_of(unquote(q.group(1)).lower()) if q else None
                key = st["queue"][st["index"]]["coverid"] if st and st["queue"] else key
            return self._send(cover(key, min(int(m.group(2) or 300), 800)), "image/jpeg")
        m = re.match(r"^/stream\.mp3\?player=([0-9a-f:]{17})$", self.path)
        st = state_of(m.group(1)) if m and m.group(1) in SLIM else None
        if st and st["queue"]:
            body = flac(st["queue"][st["index"]]["id"], st["time"])
            if body:
                return self._send(body, "audio/flac")
        m = re.match(r"^/music/(\d+)/download$", self.path)
        if m and any(t["id"] == int(m.group(1)) for t in TRACKS):
            return self._send(audio(int(m.group(1))), "audio/wav")
        m = re.match(r"^/imageproxy/([^/]+)/image", self.path)
        m = m and re.fullmatch(r"https://images\.example\.com/artists/(\d+)\.jpg", unquote(m.group(1)))
        if m:
            return self._send(artist_photo(f"artist-{m.group(1)}"), "image/jpeg")
        # The plugin's settings page, with its own script, style sheet and logo.
        if self.path.split("?")[0] == "/demo/admin":
            return self._send(admin_page(), "text/html; charset=utf-8")
        m = re.match(r"^/plugins/LyrionAnywhere/(settings/(?:anywhere\.css|anywhere\.js|qr\.js)|html/images/logo\.png)(?:\?|$)", self.path)
        if m:
            ctype = {"css": "text/css", "js": "text/javascript", "png": "image/png"}[m.group(1).rsplit(".", 1)[1]]
            return self._send((PLUGIN_HTML / m.group(1)).read_bytes(), ctype)
        m = re.match(r"^/plugins/(\w+)/html/", self.path)
        if m:
            return self._send(cover("icon-" + m.group(1), 160), "image/jpeg")
        self.send_error(404)


if __name__ == "__main__":
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument("--port", type=int, default=19000)
    ap.add_argument("--slimproto-port", type=int, default=0,
                    help="also speak Slimproto here, for the browser's own player (0: not at all)")
    ap.add_argument("--history-json", action="store_true",
                    help="print invented listening history (for `anywhere-gateway admin demo-seed-history`) and exit")
    a = ap.parse_args()
    if a.history_json:
        print(json.dumps(history_plays(int(time.time()))))
    else:
        HTTP_PORT = a.port
        if a.slimproto_port:
            threading.Thread(target=slim_serve, args=(a.slimproto_port,), daemon=True).start()
        ThreadingHTTPServer(("127.0.0.1", a.port), Handler).serve_forever()
