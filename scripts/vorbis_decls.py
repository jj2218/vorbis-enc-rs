"""List aoTuV's encoder setup declarations for scripts/gen_vorbis_tables.c: every file-scope
`static const` in vorbisenc.c and the modes/ and books/ headers it includes, as `DECL(kind,
name)` lines (kind: the C type, `_A` when an array; pointer arrays get their own kinds). The
generator prints each with the compiler's own values and `sizeof` lengths, so only declaration
heads are parsed here, never initializers.
Usage: python3 -I vorbis_decls.py <aoTuV lib dir> <out.h>"""

import pathlib
import re
import sys

lib, out = pathlib.Path(sys.argv[1]), pathlib.Path(sys.argv[2])
files = [lib / "vorbisenc.c", *sorted(lib.glob("modes/*.h")), *sorted(lib.glob("books/*/*.h"))]
PATTERNS = [
    (re.compile(r"static const int \*(\w+)\[\]"), "PINT_A"),
    (re.compile(r"static const static_codebook\s*\*\s*const\s*\*\s*const\s+(\w+)\["), "PPSC_A"),
    (re.compile(r"static const static_codebook\s*\*\s*const\s+(\w+)\["), "PSC_A"),
    (re.compile(r"static const ve_setup_data_template \*const (\w+)\[\]"), "PSETUP_A"),
    (re.compile(r"static const (\w+) (\w+)(\[[^]]*\])?\s*="), None),
]
lines, seen = [], set()
for f in files:
    for line in f.read_text(errors="replace").split("\n"):
        if not line.startswith("static const"):
            continue
        for pat, kind in PATTERNS:
            m = pat.match(line)
            if m:
                if kind is None:
                    name = m.group(2)
                    kind = m.group(1) + ("_A" if m.group(3) else "")
                else:
                    name = m.group(1)
                assert name not in seen, name
                seen.add(name)
                lines.append(f"DECL({kind}, {name})")
                break
        else:
            assert "get_setup_template" in line, line
out.write_text("\n".join(lines) + "\n")
print(f"vorbis_decls: {len(lines)} declarations -> {out}")
