#!/usr/bin/env python3
"""Generate a synthetic photo library for agent tests.

Writes a footos database plus real image files into an app data directory, so
the app can be exercised without a device photo library:

    <data_dir>/footos.db
    <data_dir>/thumbnails/seed_thumb_NN.jpg     (shared by many rows)
    <data_dir>/Photos/seed_NNNNNN.jpg           (hard links to a few originals)

Rows are clustered around real cities and spread over several years, which is
what the map clustering and the timeline need to be meaningful. Output is
deterministic for a given --count and --seed.

Usage: seed.py <data_dir> [--count 20000] [--seed 7]
"""

import argparse
import math
import os
import random
import sqlite3
import sys
from datetime import datetime, timedelta

from PIL import Image, ImageDraw

# (name, lat, lon, spread in degrees, relative weight)
PLACES = [
    ("Shanghai", 31.2304, 121.4737, 0.25, 22),
    ("Beijing", 39.9042, 116.4074, 0.20, 14),
    ("Chengdu", 30.5728, 104.0668, 0.18, 8),
    ("Lhasa", 29.6520, 91.1721, 0.30, 4),
    ("Tokyo", 35.6762, 139.6503, 0.22, 12),
    ("Kyoto", 35.0116, 135.7681, 0.08, 6),
    ("Bangkok", 13.7563, 100.5018, 0.15, 5),
    ("Paris", 48.8566, 2.3522, 0.10, 8),
    ("Reykjavik", 64.1466, -21.9426, 0.60, 4),
    ("New York", 40.7128, -74.0060, 0.12, 7),
    ("San Francisco", 37.7749, -122.4194, 0.15, 6),
    ("Queenstown", -45.0312, 168.6626, 0.25, 3),
    ("Fiji (antimeridian)", -17.7134, 178.0650, 1.20, 1),
]

VARIANTS = 24
SCHEMA = """
CREATE TABLE photos (
    id INTEGER PRIMARY KEY,
    path TEXT NOT NULL UNIQUE,
    hash TEXT NOT NULL,
    make TEXT,
    model TEXT,
    date_taken TEXT,
    width INTEGER,
    height INTEGER,
    lat REAL,
    lon REAL,
    iso INTEGER,
    f_number REAL,
    exposure_time TEXT,
    orientation INTEGER,
    thumb_path TEXT,
    file_size INTEGER DEFAULT 0,
    media_type TEXT DEFAULT 'photo',
    duration_ms INTEGER,
    video_codec TEXT,
    audio_codec TEXT,
    video_path TEXT
);
CREATE INDEX idx_photos_hash ON photos (hash);
CREATE INDEX idx_photos_date_taken ON photos (date_taken);
CREATE INDEX idx_photos_media_type ON photos (media_type);
PRAGMA user_version = 3;
"""


def make_image(index: int, size: int, path: str) -> None:
    """A flat-coloured tile with its number, so screenshots are easy to read."""
    hue = index / VARIANTS
    r, g, b = [int(120 + 110 * math.sin(2 * math.pi * (hue + k / 3))) for k in range(3)]
    img = Image.new("RGB", (size, size), (r, g, b))
    draw = ImageDraw.Draw(img)
    step = size // 8
    for i in range(0, size, step):
        draw.line([(i, 0), (size, size - i)], fill=(r // 2, g // 2, b // 2), width=max(1, size // 128))
    draw.rectangle([size * 0.3, size * 0.38, size * 0.7, size * 0.62], fill=(0, 0, 0))
    draw.text((size * 0.36, size * 0.44), f"{index:02d}", fill=(255, 255, 255))
    img.save(path, "JPEG", quality=82)


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("data_dir")
    parser.add_argument("--count", type=int, default=20000)
    parser.add_argument("--seed", type=int, default=7)
    args = parser.parse_args()

    rng = random.Random(args.seed)
    data_dir = os.path.abspath(args.data_dir)
    thumbs_dir = os.path.join(data_dir, "thumbnails")
    photos_dir = os.path.join(data_dir, "Photos")
    os.makedirs(thumbs_dir, exist_ok=True)
    os.makedirs(photos_dir, exist_ok=True)

    # Start clean: previous database, report and seeded files.
    for name in ("footos.db", "footos.db-wal", "footos.db-shm", "footos.db-journal", "agent-report.json"):
        try:
            os.remove(os.path.join(data_dir, name))
        except FileNotFoundError:
            pass
    for folder in (thumbs_dir, photos_dir):
        for name in os.listdir(folder):
            if name.startswith("seed_") or name == "map_previews.json":
                os.remove(os.path.join(folder, name))

    thumbs, originals = [], []
    for v in range(VARIANTS):
        thumb = os.path.join(thumbs_dir, f"seed_thumb_{v:02d}.jpg")
        original = os.path.join(photos_dir, f"seed_original_{v:02d}.jpg")
        make_image(v, 256, thumb)
        make_image(v, 1280, original)
        thumbs.append(thumb)
        originals.append((original, os.path.getsize(original)))

    weights = [p[4] for p in PLACES]
    start = datetime(2022, 1, 1, 8, 0, 0)
    span_days = 3 * 365
    rows = []
    for i in range(args.count):
        _, lat0, lon0, spread, _ = rng.choices(PLACES, weights=weights)[0]
        lat = max(-85.0, min(85.0, rng.gauss(lat0, spread / 3)))
        lon = ((rng.gauss(lon0, spread / 3) + 180) % 360) - 180
        # Trips: photos bunch up into bursts rather than spreading evenly.
        day = int(rng.triangular(0, span_days, span_days * 0.7))
        taken = start + timedelta(days=day, seconds=rng.randint(0, 14 * 3600))
        v = rng.randrange(VARIANTS)
        original, size = originals[v]
        path = os.path.join(photos_dir, f"seed_{i:06d}.jpg")
        os.link(original, path)
        is_video = i % 97 == 0
        rows.append((
            path, f"seed:{i}", "Seed", "Synthetic",
            None if i % 211 == 0 else taken.strftime("%Y-%m-%d %H:%M:%S"),
            1280, 1280, lat, lon, 100, 1.8, "1/120", 1,
            thumbs[v], size, "video" if is_video else "photo",
            12000 if is_video else None,
        ))

    # A handful of rows without GPS: they must not show up on the map.
    no_gps = max(1, args.count // 100)
    for i in range(no_gps):
        v = rng.randrange(VARIANTS)
        original, size = originals[v]
        path = os.path.join(photos_dir, f"seed_nogps_{i:06d}.jpg")
        os.link(original, path)
        rows.append((
            path, f"seed-nogps:{i}", "Seed", "Synthetic", "2023-06-01 12:00:00",
            1280, 1280, None, None, 100, 1.8, "1/120", 1, thumbs[v], size, "photo", None,
        ))

    db = sqlite3.connect(os.path.join(data_dir, "footos.db"))
    db.executescript(SCHEMA)
    db.executemany(
        """INSERT INTO photos (path, hash, make, model, date_taken, width, height, lat, lon,
                               iso, f_number, exposure_time, orientation, thumb_path, file_size,
                               media_type, duration_ms)
           VALUES (?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?)""",
        rows,
    )
    db.commit()
    db.close()

    print(f"seeded {args.count} geotagged + {no_gps} without GPS into {data_dir}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
