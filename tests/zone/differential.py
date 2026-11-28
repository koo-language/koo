#!/usr/bin/env python3
"""The time zones of Koo against Python's `zoneinfo`, both reading the same files: those
that `compiler/koo-rt/src/tzdata.bin` carries, which this unpacks for Python.

    python3 -I tests/zone/differential.py path/to/koo [--random 40] [--seed 1]

For every zone it asks the offset, the abbreviation, the date and the clock at the
second before, at and after every change of the zone's clocks, and at random instants
from 1677 to 2262, the years a `Time` holds, which reach past the last change of every
zone and so into the rule at the end of its file; and the instant at which the clocks
show the times on either side of each change, where they are shown twice or never, and
at random. Where the clocks show a time twice it is the first (Python's `fold=0`); where
they never show it, the time as the offset before the change has it.
"""
import os, random, struct, subprocess, sys, tempfile
from datetime import datetime, timedelta, timezone

args = sys.argv[1:]
koo = args[0]


def option(name, default):
    return args[args.index(name) + 1] if name in args else default


RANDOM = int(option("--random", "40"))
SEED = int(option("--seed", str(random.SystemRandom().randrange(1 << 30))))
rng = random.Random(SEED)
root = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
LOW, HIGH = -9223372036 + 86400 * 2, 9223372036 - 86400 * 2

blob = open(os.path.join(root, "compiler/koo-rt/src/tzdata.bin"), "rb").read()
names, files = struct.unpack(">HH", blob[16:20])
entries = [struct.unpack(">HBBH", blob[20 + i * 6 : 26 + i * 6]) for i in range(names)]
starts_at = 20 + names * 6
starts = [struct.unpack(">I", blob[starts_at + i * 4 : starts_at + i * 4 + 4])[0] for i in range(files + 1)]
text_at = starts_at + (files + 1) * 4
data_at = text_at + entries[-1][0] + entries[-1][1]
zones = {}
for at, length, _, number in entries:
    name = blob[text_at + at : text_at + at + length].decode()
    zones[name] = blob[data_at + starts[number] : data_at + starts[number + 1]]

place = tempfile.mkdtemp(prefix="koo-zones-")
for name, data in zones.items():
    path = os.path.join(place, name)
    os.makedirs(os.path.dirname(path), exist_ok=True)
    open(path, "wb").write(data)
os.environ["PYTHONTZPATH"] = place
from zoneinfo import ZoneInfo  # noqa: E402  (after the path is set)


def changes(data):
    """The instants of the changes in a TZif file's second part, and the offsets."""
    counts = struct.unpack(">6I", data[20:44])
    ut, std, leap, time, kind, chars = counts
    second = 44 + time * 5 + kind * 6 + chars + leap * 8 + std + ut
    ut, std, leap, time, kind, chars = struct.unpack(">6I", data[second + 20 : second + 44])
    at = second + 44
    times = struct.unpack(f">{time}q", data[at : at + time * 8])
    kinds = data[at + time * 8 : at + time * 9]
    types = [struct.unpack(">iBB", data[at + time * 9 + k * 6 : at + time * 9 + k * 6 + 6]) for k in range(kind)]
    return times, [types[k][0] for k in kinds], types[0][0]


questions, expected, corrected = [], [], []
epoch = datetime(1970, 1, 1, tzinfo=timezone.utc)
for name in sorted(zones):
    zone = ZoneInfo(name)
    times, offsets, first = changes(zones[name])
    asked_o = set()
    asked_l = set()
    for i, t in enumerate(times):
        before = offsets[i - 1] if i > 0 else first
        for d in (-1, 0, 1):
            asked_o.add(t + d)
        for off in (before, offsets[i]):
            for d in (-3601, -1, 0, 1, 3600):
                asked_l.add(t + off + d)
    for _ in range(RANDOM):
        asked_o.add(rng.randrange(LOW, HIGH))
        asked_l.add(rng.randrange(LOW, HIGH))
    for t in sorted(asked_o):
        if not LOW <= t <= HIGH:
            continue
        local = (epoch + timedelta(seconds=t)).astimezone(zone)
        offset = local.utcoffset()
        seconds = offset.days * 86400 + offset.seconds
        questions.append(f"o {name} {t}")
        expected.append(f"{seconds} {local.tzname()} {local.date().isoformat()} {local.hour}:{local.minute}:{local.second}")
    for t in sorted(asked_l):
        if not LOW <= t <= HIGH:
            continue
        naive = datetime(1970, 1, 1) + timedelta(seconds=t)
        aware = naive.replace(tzinfo=zone, fold=0)
        moment = aware - aware.utcoffset()
        answer = int((moment.replace(tzinfo=timezone.utc) - epoch).total_seconds())
        shown = lambda u: (epoch + timedelta(seconds=u)).astimezone(zone).replace(tzinfo=None)
        # Where the clocks show the time, the answer must be an instant at which they
        # do. `zoneinfo` has been seen to miss that where a zone's file changes the
        # clocks to the offset they already had (America/Nuuk, 2023-10-29): the first
        # instant at which they show it is then found from the offsets of the file.
        if shown(answer) != naive:
            shown_at = sorted(t - off for off in {offset for offset in offsets + [first]} if shown(t - off) == naive)
            if shown_at:
                answer = shown_at[0]
                corrected.append(f"{name} {naive.isoformat()}")
        questions.append(f"l {name} {t}")
        expected.append(str(answer))

harness = os.path.join(root, "tests/zone/harness.koo")
found = subprocess.run(
    [koo, "run", harness], input="\n".join(questions) + "\n", capture_output=True, text=True, check=False
)
if found.returncode != 0:
    sys.exit(f"the harness failed:\n{found.stderr[-2000:]}")
answers = found.stdout.splitlines()
assert len(answers) == len(questions), (len(answers), len(questions))
wrong = [(q, e, a) for q, e, a in zip(questions, expected, answers) if e != a]
for q, e, a in wrong[:20]:
    print(f"{q}: Python says {e!r}, Koo {a!r}")
if corrected:
    print(f"where zoneinfo's own answer is not shown by the clocks ({len(corrected)}): {', '.join(corrected[:6])}")
print(f"seed {SEED}: {len(questions)} questions of {len(zones)} zones, {len(wrong)} answered otherwise")
sys.exit(1 if wrong else 0)
