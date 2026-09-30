import datetime
import html
import itertools
import json
import re
import sys
import time
import urllib.request
from pathlib import Path

HERE = Path(__file__).parent
UA = {"User-Agent": "Mozilla/5.0 (X11; Linux x86_64) agentee-stackups"}
TODAY = datetime.date.today().isoformat()

JLC_API = "https://jlcpcb.com/api/overseas-shop-cart/v1/shoppingCart/getImpedanceTemplateSettings"
JLC_PAGE = "https://jlcpcb.com/impedance"
JLC_ER_PAGE = "https://jlcpcb.com/help/article/user-guide-to-the-jlcpcb-impedance-calculator"

NP155F_CORE = [
    (0.08, 3.99), (0.10, 4.36), (0.13, 4.17), (0.15, 4.36), (0.20, 4.36), (0.25, 4.23),
    (0.30, 4.41), (0.35, 4.36), (0.40, 4.36), (0.45, 4.36), (0.50, 4.48), (0.55, 4.41),
    (0.60, 4.36), (0.65, 4.36), (0.70, 4.53),
]
NP155F_CORE_THICK = 4.43
NP155F_PREPREG = {"7628": 4.4, "3313": 4.1, "2313": 4.1, "1080": 3.91, "2116": 4.16}
S1000_CORE = [
    (0.075, 4.14), (0.10, 4.11), (0.13, 4.03), (0.15, 4.35), (0.20, 4.42), (0.25, 4.29),
    (0.30, 4.56),
]
S1000_PREPREG = {"106": 3.92, "1080": 3.99, "2313": 4.31, "3313": 4.31, "2116": 4.29}


def post(url, body):
    req = urllib.request.Request(
        url, json.dumps(body).encode(), {**UA, "Content-Type": "application/json"}
    )
    with urllib.request.urlopen(req, timeout=30) as r:
        return json.load(r)


def get(url):
    with urllib.request.urlopen(urllib.request.Request(url, headers=UA), timeout=60) as r:
        return r.read().decode("utf-8", "ignore")


def nearest(table, t):
    return min(table, key=lambda p: abs(p[0] - t))[1]


def core_er(t, big):
    if big:
        return nearest(S1000_CORE, t)
    if t > 0.70 + 1e-9:
        return NP155F_CORE_THICK
    return nearest(NP155F_CORE, t)


def prepreg_er(m, big):
    own, other = (S1000_PREPREG, NP155F_PREPREG) if big else (NP155F_PREPREG, S1000_PREPREG)
    return own.get(m, other.get(m))


def mm(s):
    return float(s.removesuffix("mm"))


def oz(t):
    return {0.0152: "0.5", 0.03: "1", 0.035: "1", 0.061: "2", 0.07: "2"}.get(round(t, 4), f"{t / 0.035:g}")


def jlc_layers(template, big):
    layers = []
    for row in sorted(template["iaminationList"], key=lambda r: r["sort"]):
        c = json.loads(row["content"])
        kind = row["iaminationType"]
        if kind == 1:
            layers.append({"copper": mm(c["LineThickness"])})
        elif kind == 2:
            m = c["preMaterialType"].split("*")[0]
            layers.append({"prepreg": mm(c["preThickness"]), "material": m, "er": prepreg_er(m, big)})
        elif kind == 3:
            t = mm(c["coreBoardThickness2"])
            layers += [
                {"copper": mm(c["coreBoardThickness1"])},
                {"core": t, "er": core_er(t, big)},
                {"copper": mm(c["coreBoardThickness3"])},
            ]
        elif kind == 9:
            t = mm(c["lightPlateThickness"])
            layers.append({"core": t, "er": core_er(t, big)})
        else:
            raise ValueError(f"unknown JLC lamination row type {kind}: {c}")
    return layers


def fetch_jlc():
    found = {}
    counts = [4, 6, 8, 10, 12, 14, 16, 18, 20]
    plies = [0.4, 0.6, 0.8, 1.0, 1.2, 1.4, 1.6, 1.8, 2.0, 2.2, 2.4, 2.5, 2.8, 3.0, 3.2]
    for n, ply, outer, inner in itertools.product(counts, plies, [1, 2], [0.5, 1, 2]):
        body = {"stencilLayer": n, "stencilPly": ply, "cuprumThickness": outer, "insideCuprumThickness": inner}
        data = post(JLC_API, body).get("data") or []
        for t in data:
            if t["showName"] != t["templateName"] or not t["enableFlag"]:
                continue
            found.setdefault(t["templateName"], (n, ply, t))
        time.sleep(0.2)
        print(f"jlcpcb {n}L {ply}mm {outer}/{inner} oz: {len(data)}", file=sys.stderr)
    out = []
    for name, (n, ply, t) in sorted(found.items()):
        big = n >= 10
        layers = jlc_layers(t, big)
        cu = [l["copper"] for l in layers if "copper" in l]
        if len(cu) != n:
            raise ValueError(f"{name}: {len(cu)} copper layers, expected {n}")
        glass = sorted({l["material"] for l in layers if "material" in l})
        system = "Shengyi S1000-2M" if big else "Nan Ya NP-155F"
        out.append({
            "name": name,
            "description": f"{n} layer, {ply:g} mm, {oz(cu[0])} oz outer, {oz(cu[1])} oz inner, "
            f"{'/'.join(glass)} prepreg, {system}",
            "layers": layers,
        })
    return out


def pcbway_rows(block):
    s = block.index("Thickness after lamination(mm)") + 1
    e = next(i for i in range(s, len(block)) if block[i].startswith("Thickness after lamination:"))
    rows = []
    for t in block[s:e]:
        if re.fullmatch(r"L\d+-CU|PP|CORE", t):
            rows.append([t])
        else:
            rows[-1].append(t)
    return rows


def pcbway_layers(rows):
    layers = []
    i = 0
    while i < len(rows):
        r = rows[i]
        if r[0].endswith("-CU"):
            plated = next((x for x in r if x.startswith("(Plating to")), None)
            if plated:
                layers.append({"copper": round(float(re.search(r"[\d.]+", plated).group()) * 0.035, 4)})
            else:
                layers.append({"copper": float(r[2])})
            i += 1
        elif r[0] == "CORE":
            layers.append({"core": float(r[3]), "er": float(r[2].removeprefix("DK:"))})
            i += 1
        else:
            plies = [r]
            i += 1
            while i < len(rows) and rows[i][0] == "PP" and len(rows[i]) == 4:
                plies.append(rows[i])
                i += 1
            pressed = float(r[4])
            raw = [float(p[3]) for p in plies]
            for p, t in zip(plies, raw):
                layers.append({
                    "prepreg": round(pressed * t / sum(raw), 4),
                    "material": p[1].split()[0],
                    "er": float(p[2].removeprefix("DK:")),
                })
    return layers


def fetch_pcbway():
    url = "https://www.pcbway.com/multi-layer-laminated-structure.html"
    page = get(url)
    text = re.sub(r"<script.*?</script>|<style.*?</style>", "", page, flags=re.S)
    lines = [l.strip() for l in html.unescape(re.sub(r"<[^>]+>", "\n", text)).split("\n") if l.strip()]
    starts = [i for i, l in enumerate(lines) if re.fullmatch(r"\d+-layers PCB", l)]
    out, seen = [], {}
    for a, b in zip(starts, starts[1:] + [len(lines)]):
        block = lines[a:b]
        field = lambda key: block[block.index(key) + 1]
        n = int(block[0].split("-")[0])
        thick = field("Thickness:").removesuffix("MM")
        outer = field("Finished Outer Copper:").removesuffix("OZ")
        inner = field("Inner Copper:").removesuffix("OZ")
        ratio = field("Inner layer Residual copper ratio:").removesuffix("%")
        layers = pcbway_layers(pcbway_rows(block))
        cu = [l for l in layers if "copper" in l]
        if len(cu) != n:
            raise ValueError(f"PCBWay {n}L {thick} mm: {len(cu)} copper layers")
        glass = sorted({l["material"] for l in layers if "material" in l})
        base = f"pcbway-{n}l-{thick}mm-{outer}oz-{inner}oz-{ratio}-{'-'.join(glass)}"
        seen[base] = seen.get(base, 0) + 1
        name = base if seen[base] == 1 else f"{base}-{chr(ord('a') + seen[base] - 1)}"
        out.append({
            "name": name,
            "description": f"{n} layer, {thick} mm, {outer} oz outer, {inner} oz inner, "
            f"{ratio}% inner copper, {'/'.join(glass)} prepreg",
            "layers": layers,
        })
    return url, out


def toml_value(v):
    if isinstance(v, str):
        return json.dumps(v)
    if isinstance(v, float):
        return repr(round(v, 5))
    return str(v)


def write(path, header, stackups):
    lines = [f"{k} = {toml_value(v)}" for k, v in header.items()]
    for s in stackups:
        lines += ["", "[[stackups]]", f"name = {toml_value(s['name'])}"]
        if s.get("aliases"):
            lines.append("aliases = [" + ", ".join(toml_value(a) for a in s["aliases"]) + "]")
        lines.append(f"description = {toml_value(s['description'])}")
        lines.append("layers = [")
        for l in s["layers"]:
            lines.append("  { " + ", ".join(f"{k} = {toml_value(v)}" for k, v in l.items()) + " },")
        lines.append("]")
    path.write_text("\n".join(lines) + "\n")


JLC_ALIASES = {
    "JLC04161H-7628": ["jlcpcb-4l-1.6mm-7628"],
    "JLC04161H-3313": ["jlcpcb-4l-1.6mm-3313"],
}

JLC_TWO_LAYER = {
    "name": "jlcpcb-2l-1.6mm",
    "description": "2 layer, 1.6 mm, 1 oz, FR4",
    "layers": [{"copper": 0.035}, {"core": 1.51, "er": 4.5}, {"copper": 0.035}],
}


def main():
    which = set(sys.argv[1:]) or {"jlcpcb", "pcbway"}
    if "jlcpcb" in which:
        stackups = fetch_jlc()
        for s in stackups:
            if s["name"] in JLC_ALIASES:
                s["aliases"] = JLC_ALIASES[s["name"]]
        write(HERE / "jlcpcb.toml", {
            "fab": "jlcpcb",
            "source": f"{JLC_API} (the data behind {JLC_PAGE}); er from {JLC_ER_PAGE}",
            "fetched": TODAY,
            "mask_thickness": 0.0152,
            "mask_er": 3.8,
        }, [JLC_TWO_LAYER] + stackups)
    if "pcbway" in which:
        url, stackups = fetch_pcbway()
        write(HERE / "pcbway.toml", {"fab": "pcbway", "source": url, "fetched": TODAY}, stackups)


if __name__ == "__main__":
    main()
