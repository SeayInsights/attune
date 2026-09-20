"""Summarise licenses across the cargo dependency tree.

Reads `cargo metadata --format-version 1` on stdin.
"""

import collections
import json
import sys

RESTRICTIVE = ("GPL", "AGPL", "LGPL", "MPL", "CDDL", "EPL", "SSPL", "CC-BY-NC", "Proprietary")

# PowerShell prefixes piped output with a UTF-8 BOM; utf-8-sig tolerates both.
meta = json.loads(sys.stdin.buffer.read().decode("utf-8-sig"))
packages = meta["packages"]

counts = collections.Counter()
missing = []
flagged = []

for pkg in packages:
    lic = pkg.get("license")
    if not lic:
        missing.append((pkg["name"], pkg["version"], pkg.get("license_file")))
        counts["<none declared>"] += 1
        continue
    counts[lic] += 1
    # LGPL/AGPL contain "GPL"; match on the whole string and let the reader judge.
    if any(token in lic for token in RESTRICTIVE):
        flagged.append((pkg["name"], pkg["version"], lic))

print(f"Total packages in tree: {len(packages)}\n")

print("=== DISTINCT LICENSES ===")
for lic, n in counts.most_common():
    print(f"{n:5d}  {lic}")

print("\n=== COPYLEFT / RESTRICTIVE MATCHES ===")
if flagged:
    for name, ver, lic in sorted(flagged):
        print(f"  {name} {ver}  ->  {lic}")
else:
    print("  None.")

print("\n=== NO LICENSE FIELD DECLARED ===")
if missing:
    for name, ver, lic_file in sorted(missing):
        print(f"  {name} {ver}  (license_file: {lic_file})")
else:
    print("  None.")
