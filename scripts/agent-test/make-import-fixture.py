#!/usr/bin/env python3
"""Build a folder of real image files for the desktop import test.

    make-import-fixture.py <dir> [--count 30]

Layout (everything is regenerated from scratch):
    <dir>/lib/photo_NN.jpg          distinct geotagged JPEGs
    <dir>/lib/sub/photo_00_copy.jpg  byte-for-byte copy of photo_00.jpg
    <dir>/lib/lookalike_a.jpg / _b.jpg  same picture, different capture time
    <dir>/lib/pair.jpg + pair.dng    a JPEG with a RAW next to it (a TIFF)
Prints the number of files that must end up in the library.
"""
import argparse, os, shutil, sys
from PIL import Image, ImageDraw
import piexif


def dms(v):
    d = int(v); m = int((v - d) * 60); s = int(((v - d) * 60 - m) * 6000)
    return ((d, 1), (m, 1), (s, 100))


def exif(lat, lon, when):
    return piexif.dump({
        "GPS": {
            piexif.GPSIFD.GPSLatitudeRef: "N", piexif.GPSIFD.GPSLatitude: dms(lat),
            piexif.GPSIFD.GPSLongitudeRef: "E", piexif.GPSIFD.GPSLongitude: dms(lon),
        },
        "Exif": {piexif.ExifIFD.DateTimeOriginal: when},
    })


def picture(index):
    img = Image.new("RGB", (640, 480), ((index * 37) % 255, (index * 71) % 255, (index * 113) % 255))
    ImageDraw.Draw(img).text((40, 40), f"photo {index:02d}", fill="white")
    return img


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("dir")
    ap.add_argument("--count", type=int, default=30)
    args = ap.parse_args()

    lib = os.path.join(args.dir, "lib")
    shutil.rmtree(args.dir, ignore_errors=True)
    os.makedirs(os.path.join(lib, "sub"))

    for i in range(args.count):
        picture(i).save(os.path.join(lib, f"photo_{i:02d}.jpg"),
                        exif=exif(31.0 + i * 0.01, 121.0 + i * 0.01, f"2025:01:{1 + i % 28:02d} 10:00:00"))
    shutil.copyfile(os.path.join(lib, "photo_00.jpg"), os.path.join(lib, "sub", "photo_00_copy.jpg"))

    same = picture(900)
    same.save(os.path.join(lib, "lookalike_a.jpg"), exif=exif(35.0, 139.0, "2025:02:01 10:00:00"))
    same.save(os.path.join(lib, "lookalike_b.jpg"), exif=exif(35.0, 139.0, "2025:02:01 10:00:01"))

    pair = picture(901)
    pair.save(os.path.join(lib, "pair.jpg"), exif=exif(39.9, 116.4, "2025:03:01 10:00:00"))
    pair.save(os.path.join(lib, "pair.dng"), format="TIFF")

    # A camera file: carries its own small thumbnail in the EXIF, which an
    # import records first and replaces with a real one later.
    import io
    shot = picture(960).resize((1600, 1200))
    small = io.BytesIO()
    shot.resize((160, 120)).save(small, format="JPEG")
    with_thumb = piexif.dump({
        "0th": {piexif.ImageIFD.Make: "TestCam"},
        "Exif": {piexif.ExifIFD.DateTimeOriginal: "2025:05:01 10:00:00"},
        "1st": {piexif.ImageIFD.Compression: 6},
        "thumbnail": small.getvalue(),
    })
    shot.save(os.path.join(lib, "camera.jpg"), exif=with_thumb)

    # Not a person's photos: must stay out of the library.
    os.makedirs(os.path.join(lib, "assets"))
    os.makedirs(os.path.join(lib, "Tool.app", "Contents"))
    os.makedirs(os.path.join(lib, "node_modules", "pkg"))
    picture(950).save(os.path.join(lib, "assets", "icon.png"))
    picture(951).save(os.path.join(lib, "web.jpg"))  # no EXIF
    picture(952).save(os.path.join(lib, "Tool.app", "Contents", "bg.jpg"),
                      exif=exif(1.0, 1.0, "2025:04:01 10:00:00"))
    picture(953).save(os.path.join(lib, "node_modules", "pkg", "logo.jpg"),
                      exif=exif(2.0, 2.0, "2025:04:02 10:00:00"))

    # Distinct files: the JPEGs, two lookalikes, pair.jpg and pair.dng. The copy is a duplicate.
    print(args.count + 2 + 2 + 1)
    return 0


if __name__ == "__main__":
    sys.exit(main())
