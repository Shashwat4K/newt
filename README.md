# newt

A terminal emulator for macOS.

`newt` runs your login shell in a real PTY with xterm-compatible emulation, in a native window with tabs, split panes, scrollback, selection, and find. It is fast enough and complete enough to use as a daily driver.

The longer-term goal is a terminal multiplexer built for AI agents — sessions that share context across tabs, token usage and cost shown alongside the terminal, and agent activity visualized. Those features are not built yet.

## Status

Early, but usable. Everything below works and has been tested against `zsh` with oh-my-zsh and powerlevel10k, `vim`, `htop`, `less`, `ssh`, and `tmux`.

There are no binary downloads yet — building from source takes about a minute.

## Requirements

- macOS 14 or later
- [Rust](https://rustup.rs) 1.85 or later
- Xcode (not just the Command Line Tools)

If `xcodebuild -version` fails, point the toolchain at Xcode:

```sh
sudo xcode-select -s /Applications/Xcode.app
```

## Install

```sh
git clone <this repository>
cd newt-proto
./scripts/build.sh release
cp -R newt.app /Applications/
```

`newt` then behaves like any other app: launch it from `/Applications`, Spotlight, or the Dock.

To try it without installing, run `open newt.app` from the repository.

**First launch.** The app is ad-hoc signed, so macOS may refuse to open it the first time. Right-click the app and choose *Open*, and it will remember from then on. You may also be asked for access to your Desktop, Documents, or Downloads folders — a terminal inherits those permissions on behalf of whatever you run inside it.

To update, rebuild and replace the installed copy:

```sh
./scripts/build.sh release
rm -rf /Applications/newt.app && cp -R newt.app /Applications/
```

Replace the whole bundle rather than copying files into it; editing an installed app in place breaks its signature and macOS will refuse to launch it.

## Usage

Open `newt` and you get your login shell. Everything you would expect from a terminal works: typing, Control and Option chords, arrow and function keys, mouse reporting for full-screen programs, and bracketed paste.

| Action | Shortcut |
|---|---|
| New window | ⌘N |
| New tab | ⌘T |
| Split right | ⌘D |
| Split down | ⇧⌘D |
| Close pane | ⌘W |
| Move between panes | ⌥⌘[ and ⌥⌘] |
| Copy | ⌘C |
| Paste | ⌘V |
| Find | ⌘F |
| Find next / previous | ⌘G and ⇧⌘G |
| Scroll | mouse wheel, ⇧PageUp / ⇧PageDown |
| Scroll to top / bottom | ⇧Home / ⇧End |

Select text by dragging. Double-click selects a word, triple-click selects a line, and holding ⌥ while dragging selects a rectangular block.

Find is a literal search, not a regular expression — searching for `a.out` matches those characters, not `about`. Matches are selected and scrolled into view, so ⌘C copies whatever the search found.

`newt` uses a Nerd Font when one is installed — `MesloLGS NF`, `JetBrainsMono Nerd Font`, or `FiraCode Nerd Font` — falling back to SF Mono and then Menlo. Prompt themes like powerlevel10k need one for their icons.

## Known limitations

- **No preferences.** Font, size, and colors are fixed. There is no settings window.
- **No themes or color scheme editing.**
- **Windows and tabs are not restored** when you quit and reopen.
- **Input method composition is invisible while in progress.** Text commits correctly, but CJK input does not show the in-progress candidate in the grid.
- **Pane focus cycles in creation order**, not by position on screen, so ⌥⌘] can jump somewhere unexpected in a complex layout.
- **Other Macs will refuse to run it.** The build is ad-hoc signed, which is enough for the machine that built it but not for distribution.

## License

Not yet chosen.
