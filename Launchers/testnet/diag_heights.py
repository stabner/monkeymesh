import json
import urllib.request

for name, port in (("seed", 18080), ("edge", 18081)):
    with urllib.request.urlopen(f"http://127.0.0.1:{port}/v1/getnodeinfo", timeout=8) as r:
        d = json.load(r)
    print(name, "height", d.get("height"), "next", d.get("next_difficulty"), "tip", str(d.get("tip"))[:16])
