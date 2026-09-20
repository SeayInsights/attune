# Attune

Measurement-based microphone and headphone tuning for TC-Helicon GoXLR hardware.

> **Status: early development.** The control layer is being built. Nothing here is
> ready to use yet. Watch the repository rather than downloading it.

## What this is

Attune is a **fork of the [GoXLR Utility](https://github.com/GoXLR-on-Linux/goxlr-utility)**,
extended from a configuration tool into a tuning system.

The GoXLR already does the hard part: it splits your audio into separate buses and
runs your mic through dedicated onboard DSP. What it does not do is tell you *what
those settings should be*. That is normally decided by ear — and your ears are
listening through headphones with their own frequency response, which means the
judgement is made through a distorted instrument and the error is invisible from
inside it.

Attune measures instead.

- **Mic tuner.** Records you through the GoXLR, analyses noise floor, dynamics,
  plosives, sibilance and resonance, derives gate / compressor / EQ / de-esser
  settings, writes them to the device, and iterates. Blind A/B comparison learns
  your preference rather than chasing a generic target curve.
- **Per-bus headphone EQ.** Because the GoXLR exposes Game, Music, Chat and System
  as separate Windows endpoints, Attune can run a *different* correction curve on
  each one simultaneously — a competitive profile on Game while Music runs a
  music-tuned curve. Includes cut-only curve generation for high-impedance
  headphones, where boosting costs amplifier headroom you may not have.
- **Per-app routing.** Assigns applications to GoXLR buses automatically.
- **Profiles.** One switch changes mic settings, EQ curves and routing together.
- **Bring your own AI.** Attune exposes an MCP server. Point your own model at it
  if you want conversational tuning. No API keys, no model code and no vendor
  dependency ship with this software, and everything works with no AI configured.

### What it deliberately does not do

Attune **configures** your mic; it never processes it. The GoXLR's onboard DSP stays
in the signal path, which is what keeps monitoring at hardware latency. Pulling the
mic into Windows to process it in software would mean hearing yourself 20–50 ms
late, and no tuning quality is worth that.

## Requirements

- A TC-Helicon GoXLR or GoXLR Mini
- Windows 11
- The official TC-Helicon USB driver

Attune does **not** bundle the vendor driver — no redistribution licence has been
granted for it. If you already have the official GoXLR app installed, you have the
driver. Otherwise the installer will point you at it.

## Relationship to upstream

This is a friendly fork, not a competitor. The upstream GoXLR Utility is excellent
and Attune would not exist without it — it solved the device protocol, the daemon
architecture and the control surface, all of which Attune inherits.

Upstream is in maintenance mode: bug fixes, no new features. Attune adds the
measurement and tuning layer upstream explicitly chose not to pursue. Additions
live in separate `attune-*` crates so upstream fixes can continue to be merged in.

If you want a faithful replacement for the official GoXLR app and nothing more,
**use upstream** — it is mature, stable and well supported, and this is not.

## Licence

MIT, inherited from upstream. See [`LICENSE`](LICENSE), [`LICENSE-3RD-PARTY`](LICENSE-3RD-PARTY),
[`NOTICE`](NOTICE) and [`docs/licensing.md`](docs/licensing.md).

## Disclaimer

Attune is **not affiliated with, endorsed by, or sponsored by Music Tribe,
TC-Helicon, or any of their subsidiaries.** "GoXLR" is a trademark of its
respective owner, used here solely to identify the hardware this software works
with.

This software is provided with no warranty and no liability for any problems
arising from its use.
