import asyncio, sys, hc, json
port = int(sys.argv[1])
allbytes = bytes(range(256))
utf8 = "".join(chr(c) for c in list(range(0, 0x250)) + [0x2028, 0x2029, 0xfffd, 0xffff, 0x10000, 0x1f600, 0x10ffff]).encode()
wire = {"i": -9223372036854775808, "f": 1e-7, "b": True, "s": "q\"\\\n\t\u0000\u001f\u007fé 😀/", "o": None, "oo": "",
        "list": [9223372036854775807, 0, -1], "nested": [[], ["a", ""], [""]],
        "map": {"": {"name": "n", "tags": [], "score": None}, "k\"ey": {"name": "", "tags": ["x", "y"], "score": 2.5}},
        "shape": {"type": "dot"}, "shapes": [{"type": "circle", "radius": 1.5}, {"type": "rect", "width": 2.0, "height": 0.5}, {"type": "dot"}], "inner": {"name": "in", "tags": ["t"], "score": -0.0}, "raw": [0, 255, 10, 13]}
seq = [
 ("GET", "/add/5"), ("GET", "/get"), ("GET", "/limited/0"), ("GET", "/limited/500"), ("GET", "/limited/7"),
 ("GET", "/check/5"), ("GET", "/check/500"), ("GET", "/nested/3"), ("GET", "/inner/get"), ("GET", "/inner/via"),
 ("GET", "/opt"), ("GET", "/shape"),
 ("GET", "/cycle"), ("GET", "/pingecho"), ("GET", "/cycle"), ("GET", "/pingecho"),
 ("GET", "/selfie"), ("GET", "/selfie/one"),
 ("GET", "/boom"), ("GET", "/get"), ("GET", "/add/2"),
 ("GET", "/nested/1"), ("GET", "/nested_boom"), ("GET", "/outer/count"), ("GET", "/inner/get"),
 ("POST", "/wire", json.dumps(wire).encode()),
 ("POST", "/wire", json.dumps(dict(wire, shape={"type": "circle", "radius": -0.0}, o=5, oo=None, s="x" * 70000)).encode()),
 ("POST", "/text", allbytes), ("POST", "/text", utf8), ("POST", "/text", b""), ("POST", "/text", b"\xff\xfe"),
 ("POST", "/text", b"null"), ("POST", "/text", b'"\\'),
 ("POST", "/bytes", allbytes), ("POST", "/bytes", b""),
 ("POST", "/floats", b"[1.5, -2.25, 3]"),
 ("POST", "/ints", b"[9223372036854775807, -9223372036854775808, 0]"),
 ("GET", "/temp/5"), ("GET", "/room/a"), ("GET", "/room/a"), ("GET", "/room/b"), ("GET", "/rooms"),
 ("GET", "/room/a/close"), ("GET", "/room/a"), ("GET", "/rooms"),
 ("GET", "/log/5"), ("GET", "/log/6"), ("GET", "/trylog/7"), ("GET", "/log"),
 ("GET", "/victim/add/1"), ("GET", "/victim/stop"), ("GET", "/victim/add/1"), ("GET", "/victim/stop"), ("GET", "/victim/add/1"),
 ("GET", "/get"),
]
async def main():
    for s in seq:
        r = await hc.req(port, s[0], s[1], s[2] if len(s) > 2 else b"", timeout=10)
        body = r[1]
        shown = body.decode("latin1") if len(body) < 300 else f"<{len(body)} bytes sha {__import__('hashlib').sha1(body).hexdigest()}>"
        echo = ""
        if s[1] in ("/text", "/bytes") and r[0] == 200:
            echo = " SAME" if body == s[2] else " DIFFERENT"
        print(s[0], s[1], "->", r[0], repr(shown), echo)
asyncio.run(main())
