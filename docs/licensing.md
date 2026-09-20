# Licensing and redistribution

What Attune may ship, and what it may not. Written for the packaging work in the
"Package with Inno Setup and driver detection" work order, so that a blocker is
found now rather than at release time.

This is an engineering record, not legal advice.

## Repository-level licenses

### `LICENSE` -- upstream project, MIT

```
Copyright (c) 2022 - 2024 Craig McLure, Nathan Adams
```

Standard MIT. Permits use, copy, modify, merge, publish, distribute, sublicense
and sell, with one obligation: the copyright notice and permission notice must be
included in all copies or substantial portions.

**Satisfied by:** `NOTICE` at the repository root, reproducing the license in full.
The installer must also ship `NOTICE`.

**Verdict: redistribution permitted, including in an installer.**

### `LICENSE-3RD-PARTY` -- Music Tribe Brands CA Ltd., 2022

```
Copyright 2022 Music Tribe Brands CA Ltd.
```

This is a grant from the hardware vendor covering Music Tribe intellectual
property present in the repository. It matters more than its filename suggests:
it means the device protocol material in this project rests on a written vendor
permission, not on a reverse-engineering argument.

The grant permits use, copy, modify, merge, publish, distribute, sublicense, and
permitting others to do the same, subject to the notice being preserved.

**One difference from standard MIT, and it is deliberate enough to record:** the
grant does **not** include the words "and/or sell". Every other MIT verb is
present. For free and open-source distribution this changes nothing. If Attune is
ever sold, this clause should be re-read before doing so.

**Satisfied by:** `LICENSE-3RD-PARTY` retained unmodified, and reproduced in `NOTICE`.

**Verdict: redistribution permitted. Commercial sale is not clearly granted.**

## Things that must NOT ship in the installer

| Item | In repo? | Why not |
|---|---|---|
| TC-Helicon GoXLR USB driver | No | Required at runtime on Windows, but no redistribution licence has been granted to this project. The installer must **detect** it and link the operator to the vendor download. |
| GoXLR device firmware | No | Vendor property. Attune never flashes firmware and must not carry firmware images. |
| The official GoXLR app, or anything derived from it | No | Attune inherits a clean-room reimplementation. Nothing here is derived from decompiling the vendor application, and nothing should be. |

## Trademark

"GoXLR" is a trademark of Music Tribe / TC-Helicon. Attune uses it only to say
which hardware it works with -- nominative use. The product name must not contain
it, and the README carries an explicit statement of non-affiliation.

## Inherited files worth a decision

- **`.github/FUNDING.yml`** -- inherited from upstream and points sponsorship at
  the upstream maintainer. On a fork this is at best confusing to visitors. Decide
  explicitly whether to remove it, retarget it, or keep it as a deliberate way of
  supporting upstream.

## Dependency tree

**Audited 2026-09-20** against `cargo metadata --format-version 1`. **639 packages.**

### Summary

| Family | Count | Redistributable in an installer? |
|---|---|---|
| MIT and/or Apache-2.0 (incl. all `OR` spellings) | ~540 | **Yes**, attribution only |
| Unicode-3.0 | 18 | **Yes**, attribution only |
| BSD-2/3-Clause, ISC, Zlib, 0BSD, CC0-1.0, Unlicense, BSL-1.0, NCSA, CDLA-Permissive-2.0 | ~50 | **Yes**, attribution only |
| Apache-2.0 WITH LLVM-exception | 6 | **Yes**, attribution only |
| **MPL-2.0** | **14** | **Yes, with conditions** -- see below |

No package in the tree fails to declare a license. Nothing is GPL, AGPL or
proprietary. Nothing blocks redistribution.

### The one real obligation: MPL-2.0

This is the finding that matters, and it contradicts the assumption that the tree
was uniformly MIT/Apache:

| Package | Why it is here |
|---|---|
| `symphonia`, `symphonia-core`, `symphonia-bundle-flac`, `symphonia-bundle-mp3`, `symphonia-codec-adpcm`, `symphonia-codec-pcm`, `symphonia-codec-vorbis`, `symphonia-format-mkv`, `symphonia-format-ogg`, `symphonia-format-riff`, `symphonia-metadata`, `symphonia-utils-xiph` | Audio decoding, pulled in by `goxlr-audio` for the sampler |
| `option-ext` | Transitive, via `dirs-sys` |

MPL-2.0 is **file-level (weak) copyleft**, not project-level. Concretely:

- Attune may link these crates and ship the result under its own MIT licence.
  MPL-2.0 explicitly permits combination into a "Larger Work".
- The obligation attaches to **MPL-licensed files that you modify**. If any
  symphonia source file is changed, that file must be made available under
  MPL-2.0.
- Source for the MPL portions must be made available to recipients. Because these
  are unmodified public crates.io releases, linking to the upstream repositories
  in the installer's licence screen satisfies this in practice.

**Practical rule for packaging: do not vendor-and-patch symphonia.** Consume it
unmodified. If a fix is ever needed, upstream it or wrap it rather than editing
the file in place -- that keeps the obligation at "attribute and link" instead of
"publish our modified source".

### Not an issue, but worth recording

`r-efi 5.3.0` declares `MIT OR Apache-2.0 OR LGPL-2.1-or-later`. Because the terms
are disjunctive, Attune takes it under MIT and the LGPL branch never applies. It
appears in a copyleft keyword scan and should not be treated as a finding.

### Re-running this audit

```
cargo metadata --format-version 1 | py docs/license_audit.py
```

Re-run it before every release. A new transitive dependency can introduce an
obligation silently, and the point of this file is that nobody has to rediscover
that at release time.
