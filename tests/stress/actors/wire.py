import asyncio, sys, hc, json
port = int(sys.argv[1])
wire = {"i": -9223372036854775808, "f": 1e-7, "b": True, "s": "q\"\\\n\t\u0000\u001f\u007fé 😀/", "o": None, "oo": "",
        "list": [9223372036854775807, 0, -1], "nested": [[], ["a", ""], [""]],
        "map": {"": {"name": "n", "tags": [], "score": None}, "k\"ey": {"name": "", "tags": ["x", "y"], "score": 2.5}},
        "shape": {"type": "dot"}, "shapes": [{"type": "circle", "radius": 1.5}, {"type": "rect", "width": 2.0, "height": 0.5}, {"type": "dot"}],
        "inner": {"name": "in", "tags": ["t"], "score": -0.0}, "raw": [0, 255, 10, 13]}
async def main():
    for w in (wire, dict(wire, shape={"type": "circle", "radius": -0.0}, o=5, oo=None, s="x" * 70000)):
        r = await hc.req(port, "POST", "/wire", json.dumps(w).encode(), timeout=10)
        back = json.loads(r[1])
        print(r[0], "round trip equal:", back == w, "" if back == w else {k: (w[k], back.get(k)) for k in w if back.get(k) != w[k] and k != "s"})
asyncio.run(main())
