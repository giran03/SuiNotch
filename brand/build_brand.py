"""Builds the SuiNotch logo masters (symbol, small-size cut, lockups, app icon) as SVG.

The wordmark is Outfit SemiBold (SIL Open Font License) converted to outlines, with the
tittle of the "i" replaced by a droplet. Run: python -I brand/build_brand.py <Outfit.ttf>
"""

import math
import sys
from pathlib import Path

from fontTools.pens.boundsPen import BoundsPen
from fontTools.pens.svgPathPen import SVGPathPen
from fontTools.pens.transformPen import TransformPen
from fontTools.ttLib import TTFont
from fontTools.varLib.instancer import instantiateVariableFont

OUT = Path(__file__).resolve().parent / "final"
SUI_BLUE = "#3BA7FF"
INK = "#0A0B0D"
WHITE = "#FFFFFF"

# Symbol grid (256 canvas): the notch pill and Sui inside it.
PILL = (20.0, 72.0, 216.0, 112.0)  # x, y, w, h -> radius h/2
DROP = dict(cx=128.0, tip=87.0, cy=145.0, r=29.0)  # tip = cy - 2r: sides at exactly 60°
EYES = dict(dy=1.0, gap=10.5, r=5.6)
SMALL_PILL = (8.0, 64.0, 240.0, 128.0)
SMALL_DROP = dict(cx=128.0, tip=78.0, cy=150.0, r=36.0)


def fmt(v):
    return f"{v:.2f}".rstrip("0").rstrip(".")


def pill_path(x, y, w, h):
    r = h / 2
    return (f"M{fmt(x + r)} {fmt(y)}H{fmt(x + w - r)}A{fmt(r)} {fmt(r)} 0 0 1 {fmt(x + w - r)} {fmt(y + h)}"
            f"H{fmt(x + r)}A{fmt(r)} {fmt(r)} 0 0 1 {fmt(x + r)} {fmt(y)}Z")


def drop_path(cx, tip, cy, r):
    """Droplet: straight tangents from the tip into a circle. Tip points up."""
    d = cy - tip
    a = math.acos(r / d)  # angle at the circle centre between "up" and the tangent point
    tx, ty = r * math.sin(a), cy - r * math.cos(a)
    return (f"M{fmt(cx)} {fmt(tip)}L{fmt(cx + tx)} {fmt(ty)}"
            f"A{fmt(r)} {fmt(r)} 0 1 1 {fmt(cx - tx)} {fmt(ty)}Z")


def eyes(drop, e, fill):
    cx, cy = drop["cx"], drop["cy"] + e["dy"]
    return "".join(f'<circle cx="{fmt(cx + s * e["gap"])}" cy="{fmt(cy)}" r="{fmt(e["r"])}" fill="{fill}"/>' for s in (-1, 1))


def svg(view, body, title="SuiNotch logo"):
    return f'<svg xmlns="http://www.w3.org/2000/svg" viewBox="{view}"><title>{title}</title>{body}</svg>\n'


def symbol_mono(fill="#000000", small=False):
    p, d = (SMALL_PILL, SMALL_DROP) if small else (PILL, DROP)
    body = f'<path fill="{fill}" fill-rule="evenodd" d="{pill_path(*p)}{drop_path(**d)}"/>'
    if not small:
        body += eyes(d, EYES, fill)
    return body


def symbol_color(small=False, pill=SUI_BLUE, drop=WHITE, eye=INK):
    p, d = (SMALL_PILL, SMALL_DROP) if small else (PILL, DROP)
    body = f'<path fill="{pill}" d="{pill_path(*p)}"/><path fill="{drop}" d="{drop_path(**d)}"/>'
    if not small:
        body += eyes(d, EYES, eye)
    return body


class Wordmark:
    """'SuiNotch' as outlines, with a droplet for the dot of the i."""

    def __init__(self, font_path, weight=600, tracking=-8):
        self.font = instantiateVariableFont(TTFont(font_path), {"wght": weight})
        self.glyphs = self.font.getGlyphSet()
        self.cmap = self.font.getBestCmap()
        self.hmtx = self.font["hmtx"]
        self.tracking = tracking
        self.cap = self.font["OS/2"].sCapHeight

    def bounds(self, name):
        pen = BoundsPen(self.glyphs)
        self.glyphs[name].draw(pen)
        return pen.bounds

    def outline(self, scale, ox, baseline):
        names = [self.cmap[ord(c)] for c in "Su"] + ["dotlessi"] + [self.cmap[ord(c)] for c in "Notch"]
        pen = SVGPathPen(self.glyphs)
        x = 0.0
        i_box = None
        for k, name in enumerate(names):
            self.glyphs[name].draw(TransformPen(pen, (scale, 0, 0, -scale, ox + x * scale, baseline)))
            if name == "dotlessi":
                i_box = (x, self.bounds(name))
            x += self.hmtx[name][0] + (self.tracking if k < len(names) - 1 else 0)
        ix, ib = i_box
        full = self.bounds(self.cmap[ord("i")])
        dot_h = full[3] - ib[3]
        cx = ox + (ix + (ib[0] + ib[2]) / 2) * scale
        drop_h = dot_h * 1.3 * scale
        r = drop_h / 3  # tip = centre - 2r keeps the 60° sides of the symbol
        bottom = baseline - (full[3] - dot_h) * scale - dot_h * 0.08 * scale
        bottom = min(bottom, baseline - ib[3] * scale - dot_h * 0.25 * scale)
        tip = bottom - drop_h
        return pen.getCommands() + drop_path(cx, tip, bottom - r, r), x * scale

    def width(self, scale):
        return self.outline(scale, 0, 0)[1]


def write(name, text):
    OUT.mkdir(parents=True, exist_ok=True)
    (OUT / name).write_text(text, encoding="utf-8")


def main(font_path):
    wm = Wordmark(font_path)

    write("symbol.svg", svg("0 0 256 256", symbol_mono()))
    write("symbol-small.svg", svg("0 0 256 256", symbol_mono(small=True)))
    write("symbol-color.svg", svg("0 0 256 256", symbol_color()))
    write("symbol-small-color.svg", svg("0 0 256 256", symbol_color(small=True)))
    write("symbol-reversed.svg", svg("0 0 256 256", symbol_color(pill=WHITE, drop=INK, eye=WHITE)))

    icon = f'<rect width="256" height="256" rx="58" fill="{INK}"/>' + symbol_color()
    write("app-icon.svg", svg("0 0 256 256", icon, "SuiNotch app icon"))
    icon_small = f'<rect width="256" height="256" rx="58" fill="{INK}"/>' + symbol_color(small=True)
    write("app-icon-small.svg", svg("0 0 256 256", icon_small, "SuiNotch app icon"))

    # Horizontal lockup: pill height 1.25x cap height, gap 0.32x pill height.
    cap_px = 92.0
    scale = cap_px / wm.cap
    pill_h = cap_px * 1.25
    s = pill_h / PILL[3]
    gap = pill_h * 0.32
    height = 256.0
    baseline = height / 2 + cap_px / 2
    text_x = PILL[2] * s + gap
    word, word_w = wm.outline(scale, text_x, baseline)
    total_w = text_x + word_w
    sym_tf = f'translate({fmt(-PILL[0] * s)} {fmt(height / 2 - (PILL[1] + PILL[3] / 2) * s)}) scale({fmt(s)})'

    def horizontal(sym, word_fill):
        return svg(f"0 0 {fmt(total_w)} {fmt(height)}", f'<g transform="{sym_tf}">{sym}</g><path fill="{word_fill}" d="{word}"/>')

    write("lockup-horizontal.svg", horizontal(symbol_mono(), "#000000"))
    write("lockup-horizontal-color.svg", horizontal(symbol_color(), INK))
    write("lockup-horizontal-reversed.svg", horizontal(symbol_color(), WHITE))

    # Stacked lockup: symbol centred above the wordmark.
    st_cap = 74.0
    st_scale = st_cap / wm.cap
    st_word_w = wm.width(st_scale)
    st_w = max(st_word_w, PILL[2] * 1.25) + 40
    k = 1.25
    sym_top = 20.0
    st_sym_tf = f'translate({fmt(st_w / 2 - 128 * k)} {fmt(sym_top - PILL[1] * k)}) scale({fmt(k)})'
    st_base = sym_top + PILL[3] * k + 40 + st_cap
    st_word, _ = wm.outline(st_scale, (st_w - st_word_w) / 2, st_base)
    st_h = st_base + 24
    write("lockup-stacked.svg", svg(f"0 0 {fmt(st_w)} {fmt(st_h)}", f'<g transform="{st_sym_tf}">{symbol_mono()}</g><path d="{st_word}"/>'))
    write("lockup-stacked-color.svg", svg(f"0 0 {fmt(st_w)} {fmt(st_h)}", f'<g transform="{st_sym_tf}">{symbol_color()}</g><path fill="{INK}" d="{st_word}"/>'))

    wm_cap = 120.0
    w_word, w_w = wm.outline(wm_cap / wm.cap, 8, 128 + wm_cap / 2)
    write("wordmark.svg", svg(f"0 0 {fmt(w_w + 16)} 256", f'<path d="{w_word}"/>'))
    print("wrote", sorted(p.name for p in OUT.glob("*.svg")))


if __name__ == "__main__":
    main(sys.argv[1])
