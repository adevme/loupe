<p align="center"><img src="crates/app/assets/text-logo.png" alt="Loupe" width="300"></p>

<p align="center">A digital audio workstation.</p>

## What it runs on

Linux, Windows and macOS, from the same source.

## Why open source

Making music should not depend on a company's pricing meeting. A tool people
build their work inside of should outlive whoever started it.

## The mission

**Make music reachable for everyone.** Not cheaper. Reachable. Someone with an
old laptop and something to say should be able to get to the end of a song.

## Branches

`main` is the last release, so building it gives you the same Loupe as the
downloads. `develop` is where the work goes, and only reaches `main` when a
version goes out.

## Licence

Loupe is free software under the [GNU General Public License, version 2](LICENSE)
or any later version. It includes [Rubber Band](https://breakfastquay.com/rubberband/)
for time stretching, which is under the same licence, and [LAME](https://lame.sourceforge.io/)
for writing MP3s, which is under the Lesser GPL. Both are in `vendor/`, each with its
own licence beside it.

### ASIO

Loupe does not ship with ASIO. Steinberg's ASIO SDK is not free software, and a
build that links it cannot be passed on under this licence. On Windows, pick the
WASAPI driver in Settings for low latency. If you have agreed to Steinberg's
terms you can build your own copy with `cargo build --features asio`, but please
keep that build to yourself.
