# ExileWatch

A Path of Exile 2 trade price-check overlay for Linux. Built because no native Linux option existed — Awakened PoE Trade and Exile Exchange 2 are Windows-only, and running them under Wine on Wayland is unreliable.

Press **Alt+D** while hovering over any item in-game, and ExileWatch reads your clipboard, queries the GGG trade API, and shows live price results in a floating overlay — without leaving the game.

## Features

- **Global hotkey** (Alt+D) via evdev — works on KDE Wayland regardless of window focus
- **Layer-shell overlay** — appears above the game, closes when you press Escape or release Alt
- **PoE2 trade search** — stat ID matching, local weapon/armour stat overrides, pseudo-stat filters
- **Affix filters** — toggle individual mods, set min/max values per affix
- **Base stat filters** — DPS, PDPS, EDPS, APS, Crit, Armour, Evasion, Energy Shield, Ward, Block
- **Pseudo-stat filters** — Total Life, Total Mana, Total Energy Shield, Total Elemental Resistance
- **Socket filter** — augment socket count (rune sockets)
- **Socketed mods excluded** — rune, soul core, and augment bonuses are stripped before searching
- **Buyout type** — Instant (Ange's store), In Person (online sellers), or Both
- **Corrupted filter** — optionally restrict to corrupted or non-corrupted items
- **Currency selector** — Divine Orbs or Exalted Orbs
- **Draggable overlay** — position saved between sessions

## Requirements

- Linux with Wayland (KDE Plasma recommended)
- Your user must be in the `input` group for the global hotkey to work:
  ```
  sudo usermod -aG input $USER
  ```
  Log out and back in after running this.
- A PoE2 session cookie (`POESESSID`) — used to authenticate trade API requests

## Installation

Download the latest release from the [Releases](https://github.com/5h4rkByt3/ExileWatch/releases) page.

### AppImage
```bash
chmod +x ExileWatch_*.AppImage
./ExileWatch_*.AppImage
```

### deb (Ubuntu / Debian)
```bash
sudo dpkg -i ExileWatch_*.deb
exilewatch
```

### rpm (Fedora / openSUSE)
```bash
sudo rpm -i ExileWatch-*.rpm
exilewatch
```

## Setup

1. Launch ExileWatch — it starts minimised in the background.
2. On first run, open the overlay with Alt+D, go to **Settings**, and paste your `POESESSID` cookie value. You can find this in your browser's cookies on the PoE2 trade site after logging in.
3. Select your league.

## Usage

1. In Path of Exile 2, hover over any item and press **Alt+D**.
2. The overlay appears with the item's affixes pre-populated. Toggle which mods to include in the search and adjust min/max values as needed.
3. Select your preferred buyout type and currency, then click **Search**.
4. Results show listed prices. Click the trade URL to open the listing in your browser.
5. Press **Alt+D** to dismiss the overlay.

## Building from source

Requires Rust, Node.js 20+, and the following system libraries:

```bash
# Ubuntu / Debian
sudo apt-get install -y \
  libwebkit2gtk-4.1-dev librsvg2-dev patchelf \
  libgtk-3-dev libssl-dev \
  libayatana-appindicator3-dev libgtk-layer-shell-dev

# Arch
sudo pacman -S webkit2gtk-4.1 librsvg patchelf gtk3 openssl \
  libayatana-appindicator gtk-layer-shell
```

```bash
git clone https://github.com/5h4rkByt3/ExileWatch.git
cd ExileWatch
npm install
npm run tauri dev
```

## Known limitations

- KDE taskbar briefly flickers when the overlay appears — this is KWin behaviour and cannot be fixed from the app side
- PoE1 support is not yet implemented
- The global hotkey requires evdev access (`input` group membership)
