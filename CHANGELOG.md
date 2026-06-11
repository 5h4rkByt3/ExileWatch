# Changelog

## [0.1.0] — Unreleased

### Added
- Alt+D global hotkey via evdev (works on KDE Wayland regardless of window focus)
- Layer-shell overlay appears above the game without stealing focus
- PoE2 clipboard item parsing — item class, rarity, mods, base stats
- Real GGG trade API search (PoE2)
- Stat ID matching with weapon/armour local stat overrides
- Pseudo-stat filters: Total Life, Mana, Energy Shield, Elemental Resistance
- Base stat filters: DPS, PDPS, EDPS, APS, Crit, Armour, Evasion, ES, Ward, Block
- Socket count filter (rune_sockets)
- Socketed item mods (runes, soul cores, augments) excluded from search
- Buyout type filter: Instant (IOB), In Person, Both
- Item category filter always sent to prevent cross-type results
- Corrupted item filter
- Currency selector (divine / exalted)
- League selector
- Draggable overlay with saved position
- Escape key and Alt-release close the overlay
