#!/usr/bin/env python3
"""Bjontegaard delta rate (cubic fit of log-rate over PSNR, integrated over the overlap).

usage: bd.py results.csv <anchor_enc> <anchor_preset>
CSV columns: seq,enc,preset,qp,frames,enc_s,fps,kbps,psnr_y,...
"""
import csv
import sys
from collections import defaultdict

import numpy as np


def bd_rate(r1, p1, r2, p2):
    l1, l2 = np.log(r1), np.log(r2)
    c1, c2 = np.polyfit(p1, l1, 3), np.polyfit(p2, l2, 3)
    lo, hi = max(min(p1), min(p2)), min(max(p1), max(p2))
    i1, i2 = np.polyint(c1), np.polyint(c2)
    a1 = np.polyval(i1, hi) - np.polyval(i1, lo)
    a2 = np.polyval(i2, hi) - np.polyval(i2, lo)
    return (np.exp((a2 - a1) / (hi - lo)) - 1) * 100


def main():
    path, aenc, apre = sys.argv[1:4]
    data = defaultdict(list)
    for row in csv.DictReader(open(path)):
        data[(row["seq"], row["enc"], row["preset"])].append(
            (float(row["kbps"]), float(row["psnr_y"]), float(row["fps"]))
        )
    seqs = sorted({k[0] for k in data})
    cfgs = sorted({(k[1], k[2]) for k in data if (k[1], k[2]) != (aenc, apre)})
    print("seq," + ",".join(f"{e}-{p}" for e, p in cfgs))
    tot = defaultdict(list)
    for s in seqs:
        a = sorted(data[(s, aenc, apre)])
        line = [s]
        for e, p in cfgs:
            t = sorted(data.get((s, e, p), []))
            if len(a) < 4 or len(t) < 4:
                line.append("")
                continue
            v = bd_rate([x[0] for x in a], [x[1] for x in a], [x[0] for x in t], [x[1] for x in t])
            tot[(e, p)].append(v)
            line.append(f"{v:+.1f}%")
        print(",".join(line))
    print("mean," + ",".join(f"{np.mean(tot[c]):+.1f}%" if tot[c] else "" for c in cfgs))
    print()
    print("enc,mean_fps")
    for e, p in [(aenc, apre)] + cfgs:
        f = [x[2] for (s, ee, pp), vs in data.items() if (ee, pp) == (e, p) for x in vs]
        print(f"{e}-{p},{np.mean(f):.2f}")


if __name__ == "__main__":
    main()
