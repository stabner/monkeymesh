import json
import urllib.request


def get(url):
    with urllib.request.urlopen(url, timeout=8) as r:
        return json.load(r)


def retarget(port, name):
    d = get(f"http://127.0.0.1:{port}/v1/proposals")
    e = d.get("active_envelopes") or {}
    print(name, "retarget", d.get("retarget"))
    print(
        name,
        "floor",
        e.get("min_difficulty_floor"),
        "interval",
        e.get("retarget_interval"),
        "step",
        e.get("retarget_step"),
        "bias",
        e.get("suggested_cpu_diff_bias"),
    )
    print(
        name,
        "adapt_h",
        d.get("last_auto_adapt_at_height"),
        "pid",
        d.get("last_auto_adapt_proposal_id"),
    )


def blk(port, h, name):
    d = get(f"http://127.0.0.1:{port}/v1/getblock?height={h}")
    hdr = d.get("header") or d
    print(
        name,
        "h",
        hdr.get("height"),
        "diff",
        hdr.get("difficulty"),
        "ts",
        hdr.get("timestamp"),
        "id",
        d.get("id") or d.get("hash") or d.get("tip"),
    )


retarget(18080, "SEED")
retarget(18081, "EDGE")
blk(18080, 1719, "SEED_1719")
blk(18081, 1719, "EDGE_1719")
blk(18081, 1720, "EDGE_1720")
blk(18080, 1700, "SEED_1700")
blk(18081, 1700, "EDGE_1700")
