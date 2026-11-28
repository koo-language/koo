"""Writes `compiler/koo-rt/src/tzdata.bin`, the time zones a Koo program carries.

The zones are those of a named release of the IANA time zone database, compiled to the
TZif files of RFC 8536 as the Python package `tzdata` ships them, which is how they are
had without a compiler of the database's own:

    python3 -I compiler/koo-rt/tz/make.py path/to/tzdata/zoneinfo

where the directory is the `tzdata/zoneinfo` of the package's wheel (2026.6 is IANA
2026f, sha256 0a10eec6e13ef16b627fabed9a5f2ba61a62b1c67f327305f4e7e412a70b4aec). A zone
that is another's link, the same file under another name, is kept once.

The file is little more than the TZif files one after another, with what finds them:

    0   b"KOOTZ1\\0\\0"
    8   the release, eight bytes, padded with zeros: b"2026f\\0\\0\\0"
    16  the number of names, and of files, each in two bytes, big-endian
    20  for each name, in the order of their bytes: where it is among the names (two
        bytes), its length (one), a zero, and the number of its file (two)
    ..  for each file, and one more, where it begins among the files (four bytes)
    ..  the names, one after another
    ..  the files, one after another
"""

import hashlib
import os
import struct
import sys

root = sys.argv[1]
out = os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "src", "tzdata.bin")
release = None
with open(os.path.join(root, "tzdata.zi"), encoding="ascii") as zi:
    first = zi.readline()
    assert first.startswith("# version "), first
    release = first.split()[2]
names = []
for dirpath, _, files in os.walk(root):
    for file in files:
        path = os.path.join(dirpath, file)
        with open(path, "rb") as f:
            if f.read(4) != b"TZif":
                continue
        names.append(os.path.relpath(path, root).replace(os.sep, "/"))
names.sort(key=lambda name: name.encode())
files, numbered, of = [], {}, []
for name in names:
    with open(os.path.join(root, name), "rb") as f:
        data = f.read()
    assert data[4] >= ord("2"), f"{name} is TZif version 1"
    key = hashlib.sha256(data).digest()
    if key not in numbered:
        numbered[key] = len(files)
        files.append(data)
    of.append(numbered[key])
assert len(release) <= 8 and len(names) < 65536 and len(files) < 65536
head = b"KOOTZ1\0\0" + release.encode().ljust(8, b"\0") + struct.pack(">HH", len(names), len(files))
entries, text = b"", b""
for name, number in zip(names, of):
    encoded = name.encode()
    assert len(encoded) < 256 and len(text) < 65536
    entries += struct.pack(">HBBH", len(text), len(encoded), 0, number)
    text += encoded
starts, at = b"", 0
for data in files:
    starts += struct.pack(">I", at)
    at += len(data)
starts += struct.pack(">I", at)
blob = head + entries + starts + text + b"".join(files)
with open(out, "wb") as f:
    f.write(blob)
print(f"{release}: {len(names)} names, {len(files)} files, {len(blob)} bytes")
