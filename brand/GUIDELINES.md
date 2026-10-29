# SuiNotch logo guidelines

**SuiNotch** by **CafeTako** · support@cafetako.com

## The mark

The SuiNotch mark is the notch itself: a pill holding **Sui**, a water droplet (*sui*, 水, means water) with two eyes. Sui is also the app's idle mascot. In the wordmark, a droplet replaces the dot of the *i*.

| File (`brand/final/`) | Use |
|---|---|
| `symbol-color.svg` | Default symbol on light or dark backgrounds |
| `symbol.svg` | One colour, with the droplet as a real cut-out (print, engraving, embroidery) |
| `symbol-small.svg`, `symbol-small-color.svg` | **Required at 32 px and below.** Bigger droplet, no eyes |
| `symbol-reversed.svg` | White pill for very dark photos |
| `lockup-horizontal-*.svg` | Primary logo: `color` on light backgrounds, `reversed` on dark ones |
| `lockup-stacked*.svg` | Square spaces (splash screens, social avatars with a name) |
| `wordmark.svg` | Name only, when the symbol is already nearby |
| `app-icon.svg`, `app-icon-small.svg` | App and store icons; the small one is for 32 px and below |

Exported one-colour variants (black, white, Sui Blue) and PNGs are in `brand/exports/`. App icon PNGs and the multi-size `suinotch.ico` are in `brand/icons/`. The exe embeds `assets/suinotch.ico`.

## Colour

| Name | HEX | RGB | Role |
|---|---|---|---|
| Sui Blue | `#3BA7FF` | 59 167 255 | The pill; primary accent |
| Ink | `#0A0B0D` | 10 11 13 | Wordmark, eyes, app-icon tile, the notch itself |
| White | `#FFFFFF` | 255 255 255 | The droplet |

The pill is always Sui Blue, Ink or White. Accent colours that users choose in Settings tint the app's UI, never the logo.

## Clear space and minimum size

- Clear space on every side is **half the pill's height**.
- Minimum sizes: the symbol at **16 px** (small cut) and the horizontal lockup at **96 px** wide on screen or **25 mm** in print.

## Construction

- 256-unit grid. The pill is 216 × 112 with full round ends.
- The droplet has straight sides at **exactly 60°**: its tip sits 2r above the circle's centre.
- Wordmark: Outfit SemiBold (SIL Open Font License) converted to outlines, tracking −8/1000. Regenerate everything with `python -I brand/build_brand.py <Outfit.ttf>`, then `python -I brand/pack_ico.py`.

## Don't

- Recolour the droplet, or put the eyes on the small cut.
- Stretch, rotate or outline the pill, or add a shadow or gradient to it.
- Retype the wordmark in a live font, or restore the round dot on the *i*.
- Place the colour symbol on Sui Blue or on busy mid-tone photos (use `symbol-reversed` or a solid backing).

## Open items

- A trademark search has not been done. Run one (name and mark) before any public release.
