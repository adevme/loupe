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

Loupe does not ship with ASIO, and the SDK is not vendored here. Steinberg's
ASIO SDK is offered either under Steinberg's own licence, which forbids passing
the SDK on without a signed agreement, or under the GPL version 3; and parts of
it (`common/combase.*`, `common/wxdebug.h`) are old Microsoft code carrying no
redistribution grant at all. So the SDK stays off this repository and the
downloads stay free of it.

On Windows, ASIO is the only way Loupe can honour the buffer size your interface
is set to. WASAPI shared mode decides the buffer itself, which on a 128-frame
interface still came out at 1056 frames, so monitoring your own voice while
recording is not usable without ASIO.

To build a copy with ASIO for your own use:

1. Download the SDK from <https://www.steinberg.net/asiosdk> and unpack it.
2. Point `CPAL_ASIO_DIR` at the folder holding `common/`, `host/` and `driver/`.
3. `cargo build --release --features asio`, or `installer/build-windows.sh` for
   the whole installer.

Cross-building that from Linux works: `--target x86_64-pc-windows-gnu` with
`x86_64-w64-mingw32-g++` and `clang` installed. Keep the build to yourself.
