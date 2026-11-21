"""Extract only testvec/input/thetest*.wav from the ETSI LC3plus zip into a flat directory.
Usage: python3 -I lc3plus_wav.py <zip> <outdir> (absolute paths; the zip is untrusted data)."""
import os
import re
import sys
import zipfile

zpath, out = sys.argv[1], sys.argv[2]
os.makedirs(out, exist_ok=True)
n = 0
with zipfile.ZipFile(zpath) as z:
    for info in z.infolist():
        m = re.fullmatch(r"ETSI_Release/[^/]+/testvec/input/(thetest[0-9]+\.wav)", info.filename)
        if m:
            with z.open(info) as src, open(os.path.join(out, m.group(1)), "wb") as dst:
                dst.write(src.read())
            n += 1
if n == 0:
    sys.exit("lc3plus_wav: no testvec/input/thetest*.wav in the zip")
