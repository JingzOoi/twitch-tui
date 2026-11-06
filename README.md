<div align="center">

# twitch-tui

Twitch chat in the terminal.

[![crates][s1]][l1] [![CI][s2]][l2] [![pre-commit.ci status][s3]][l3] [![unsafe][s4]][l4]

[s1]: https://img.shields.io/crates/v/twitch-tui.svg
[l1]: https://crates.io/crates/twitch-tui
[s2]: https://github.com/Xithrius/twitch-tui/actions/workflows/ci.yml/badge.svg
[l2]: https://github.com/Xithrius/twitch-tui/actions/workflows/ci.yml
[s3]: https://results.pre-commit.ci/badge/github/Xithrius/twitch-tui/main.svg
[l3]: https://results.pre-commit.ci/latest/github/Xithrius/twitch-tui/main
[s4]: https://img.shields.io/badge/unsafe-forbidden-success.svg
[l4]: https://github.com/rust-secure-code/safety-dance/

<img src="assets/preview.png" />

</div>

## About this fork

This is a personal fork of [Xithrius/twitch-tui](https://github.com/Xithrius/twitch-tui), changed to better fit my own use. For everything else, see the upstream project and its documentation.

### Changes in this fork

- Reply to chat messages: press `v` to select a message (scroll keys move the selection), `r` to reply, `Esc` to cancel.
- Fixed Shared Chat messages being dropped because `source_badges` failed to deserialize.
- Show stream time instead of the clock time: set `stream_time = true` under `[frontend]`, and messages show how far into the stream they were sent (`HH:MM:SS`). Offline channels still show the clock time.

## Feature list

- Read/send/search messages
- Switch channels
- Create and toggle filters
- Command, channel, and mention suggestions
- Customize functionality and looks to your liking using a [config file](https://github.com/Xithrius/twitch-tui/blob/main/default-config.toml)

## Links

- [Documentation](https://xithrius.github.io/twitch-tui/)
- [Setup](https://xithrius.github.io/twitch-tui/guide/installation)

## More information

If you have any problems, do not hesitate to [submit an issue](https://github.com/Xithrius/twitch-tui/issues/new/choose).

Combine this application with [streamlink](https://github.com/streamlink/streamlink) to rid the need of a browser while watching streams.

This project follows the guidelines of [Semantic Versioning](https://semver.org/).

[![](https://raw.githubusercontent.com/dch82/Nixpkgs-Badges/main/nixpkgs-badge-dark.svg)](https://search.nixos.org/packages?size=1&show=twitch-tui)
