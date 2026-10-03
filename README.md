# Loupe

A digital audio workstation. It opens audio, lays it out on a timeline, lets you
cut, fade and balance it, hosts your plugins, records takes and writes a
finished mix — the whole song in one place, with the detail work kept close at
hand.

- Written in Rust, drawing its own interface. Starts fast, stays responsive.
- Linux, Windows and macOS from the same source. No second-class twin.
- Hosts VST3, CLAP, LV2 and Audio Unit plugins, each in its own process, so one that falls over does not take your session with it.

## The philosophy

Most workstations make the track the smallest thing you can treat on its own.
Everything on a track shares one chain, so a single word that sits wrong means
a new track, a new chain, and a session that grows faster than the song does.

Loupe starts from the other end. The small thing comes first: a clip, a take, a
line. Routing is plain and open — any track can feed any other, folders are just
tracks with tracks inside them, and nothing is hidden behind a mode you have to
learn before you can work.

The rest follows from three rules:

- **Speed is a feature.** Lag breaks the thought you were having. Nothing in the
  way of the work.
- **Show the work, not the chrome.** If a control is not doing anything, it is
  not on the screen.
- **No lock-in.** Projects are plain text. Audio is referenced where it lives.
  You can read a Loupe project in any editor, and you can leave whenever you
  like.

## Why open source

Making music should not depend on a company's pricing meeting.

Tools people build their work inside of should be inspectable, forkable and
outlive whoever started them. If Loupe is useful, it should stay useful — even
if I stop. Open source is the only way to promise that honestly.

It also means the people who use it can fix it. A workstation is a deeply
personal thing; everybody holds it slightly differently. Being able to change
yours is not a niche request.

## The mission

**Make music reachable for everyone.**

Not cheaper. Reachable. No licence server, no subscription, no tier that hides
the feature you happen to need, no second computer because yours is the wrong
brand. Someone with an old laptop and something to say should be able to
download Loupe and get to the end of a song.

That is the whole point.
