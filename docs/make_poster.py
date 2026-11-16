#!/usr/bin/env python3
"""Generate docs/poster.png (1920x1440) from the pets' own animation frames.

Run from anywhere:  python3 docs/make_poster.py
Requires Pillow and the fonts listed below (Arch: gsfonts, noto-fonts-cjk).
"""
from PIL import Image, ImageDraw, ImageFont, ImageFilter
import random, os

W, H = 1920, 1440
SERIF_B = "/usr/share/fonts/gsfonts/C059-Bold.otf"
SANS_B = "/usr/share/fonts/gsfonts/NimbusSans-Bold.otf"
SANS = "/usr/share/fonts/gsfonts/NimbusSans-Regular.otf"
CJK_B = "/usr/share/fonts/noto-cjk/NotoSansCJK-Bold.ttc"
CJK = "/usr/share/fonts/noto-cjk/NotoSansCJK-Regular.ttc"

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
OUT = os.path.join(ROOT, "docs", "poster.png")

# key, dir, display, sub, anims, accent, hero-clip, [mini clips]
PETS = [
    ("claude", "claude-girl", "CLAUDE GIRL", "Claude 少女", 166, (232, 137, 92),
     "点击回应-开心跃动", ["原地左转奔跑", "写代码", "可爱宅舞"]),
    ("dsh", "dsh-girl", "DSH GIRL", "DSH 少女", 101, (108, 168, 240),
     "点击回应-开心跃动", ["悠闲哼歌", "原地小憩沉眠", "工作状态-雀跃庆祝"]),
    ("codex", "codex-girl", "CODEX GIRL", "Codex 少女", 90, (196, 170, 250),
     "开心", ["奔跑", "写代码", "听音乐"]),
    ("grok", "grok-girl", "GROK GIRL", "Grok 少女", 101, (246, 196, 96),
     "点击回应-开心跃动", ["原地左转奔跑", "写代码", "可爱宅舞"]),
    ("qwen", "qwen-girl", "QWEN GIRL", "Qwen 少女", 104, (128, 200, 246),
     "开心跃动", ["原地左转奔跑", "写代码", "可爱宅舞"]),
    ("gemini", "gemini-girl", "GEMINI GIRL", "Gemini 少女", 100, (186, 156, 250),
     "点击回应-开心跃动", ["原地左转奔跑", "写代码", "可爱宅舞"]),
    ("glm", "glm-girl", "GLM GIRL", "GLM 少女", 97, (232, 168, 210),
     "点击回应-开心跃动", ["原地左转奔跑", "写代码", "可爱宅舞"]),
    ("kimi", "kimi-girl", "KIMI GIRL", "Kimi 少女", 105, (152, 196, 236),
     "点击回应-开心跃动", ["原地左转奔跑", "写代码", "可爱宅舞"]),
]

MINI_LABELS = {
    "原地左转奔跑": "RUN", "奔跑": "RUN",
    "写代码": "CODE", "可爱宅舞": "DANCE", "悠闲哼歌": "MUSIC", "听音乐": "MUSIC",
    "原地小憩沉眠": "SLEEP", "睡觉": "SLEEP", "工作状态-雀跃庆祝": "PARTY", "庆祝": "PARTY",
}


def F(path, size):
    return ImageFont.truetype(path, size)


def make_background():
    img = Image.new("RGB", (W, H), (8, 7, 16))
    px = img.load()
    for y in range(H):
        t = y / H
        r, g, b = int(8 + 22 * t ** 2), int(7 + 13 * t ** 2), int(16 + 34 * t ** 2)
        for x in range(W):
            px[x, y] = (r, g, b)

    glow = Image.new("RGB", (W, H), (0, 0, 0))
    gd = ImageDraw.Draw(glow)
    gd.ellipse([-600, 600, 1100, 2000], fill=(80, 32, 22))
    gd.ellipse([1000, -500, 2500, 900], fill=(32, 24, 70))
    glow = glow.filter(ImageFilter.GaussianBlur(260))
    img = Image.blend(img, Image.blend(img, glow, 0.6), 0.8)

    sd = ImageDraw.Draw(img, "RGBA")
    rnd = random.Random(20261116)
    for _ in range(1100):
        x, y = rnd.randrange(W), rnd.randrange(H)
        s = 1 if rnd.random() < 0.9 else 2
        sd.ellipse([x, y, x + s, y + s], fill=(255, 250, 235, rnd.randint(18, 105)))
    for _ in range(45):
        x, y = rnd.randrange(W), rnd.randrange(H)
        sd.ellipse([x, y, x + 3, y + 3], fill=(255, 255, 255, rnd.randint(90, 165)))
    return img


def cell(pet_dir, name, idx=40):
    path = os.path.join(ROOT, pet_dir, "media", "anim", name + ".webp")
    im = Image.open(path).convert("RGBA")
    c, r = idx % 8, idx // 8
    out = im.crop((c * 640, r * 360, (c + 1) * 640, (r + 1) * 360))
    bb = out.getbbox()
    return out.crop(bb) if bb else out


def fit(im, height):
    return im.resize((int(im.width * height / im.height), height), Image.LANCZOS)


def main():
    img = make_background()
    d = ImageDraw.Draw(img, "RGBA")

    # header
    d.text((86, 46), "◆  LINUX DESKTOP PETS", font=F(SANS_B, 17), fill=(152, 142, 192))
    t = "WAYLAND NATIVE"
    d.text((W - 86 - d.textlength(t, font=F(SANS_B, 17)), 46), t,
           font=F(SANS_B, 17), fill=(152, 142, 192))

    # RUST wordmark
    rust = F(SERIF_B, 200)
    x, y = 74, 82
    for ch, col in (("R", (242, 208, 178)), ("U", (218, 218, 238)),
                    ("S", (192, 200, 242)), ("T", (168, 178, 238))):
        d.text((x, y), ch, font=rust, fill=col)
        x += d.textlength(ch, font=rust) - 7

    d.text((80, 300), "DESKTOP PET OCTET", font=F(SERIF_B, 46), fill=(240, 234, 228))
    d.text((82, 360), "八位少女 · 八种陪伴", font=F(CJK_B, 27), fill=(228, 216, 238))
    d.text((82, 404), "写代码的时候，让她们陪你待在 Linux 桌面上。  </>",
           font=F(CJK, 21), fill=(158, 150, 184))

    # right column
    colx = 1180
    d.text((colx, 96), "WHY THIS OCTET", font=F(SANS_B, 15), fill=(152, 142, 192))
    y = 138
    for num, title, sub in (
        ("01", "LINUX NATIVE", "为 Linux 桌面而生 · 轻量常驻"),
        ("02", "WAYLAND NATIVE", "layer-shell · niri / Hyprland"),
        ("03", "RUST POWERED", "单二进制 · 无 Electron · 低占用"),
    ):
        d.rectangle([colx, y + 4, colx + 3, y + 60], fill=(152, 122, 222))
        d.text((colx + 22, y), num, font=F(SANS_B, 15), fill=(152, 122, 222))
        d.text((colx + 56, y - 5), title, font=F(SANS_B, 23), fill=(242, 240, 248))
        d.text((colx + 56, y + 28), sub, font=F(CJK, 15), fill=(150, 144, 178))
        y += 80

    # ── two rows of four ──────────────────────────────────────────
    hero_h = 212
    mh, mw = 84, 54
    row_tops = [442, 872]
    col_w = W / 4

    for i, (key, pdir, disp, sub, anims, accent, hero, minis) in enumerate(PETS):
        row, col = i // 4, i % 4
        cx = col_w * col + col_w / 2
        top = row_tops[row]

        # sprite
        sp = fit(cell(pdir, hero), hero_h)
        sx = int(cx - sp.width / 2)
        sy = top

        halo = Image.new("RGBA", (sp.width + 110, sp.height + 110), (0, 0, 0, 0))
        ImageDraw.Draw(halo).ellipse([55, 55, sp.width + 55, sp.height + 55],
                                     fill=accent + (46,))
        halo = halo.filter(ImageFilter.GaussianBlur(38))
        img.paste(halo, (sx - 55, sy - 55), halo)
        img.paste(sp, (sx, sy), sp)

        # name plate
        pw, ph = 226, 54
        px, py = int(cx - pw / 2), top + hero_h + 12
        d.rounded_rectangle([px, py, px + pw, py + ph], radius=9,
                            fill=(14, 12, 26, 232), outline=accent + (180,), width=2)
        f1 = F(SANS_B, 19)
        d.text((cx - d.textlength(disp, font=f1) / 2, py + 7), disp, font=f1,
               fill=(248, 246, 252))
        f2 = F(CJK, 12)
        st = f"{sub} · {anims} ANIMATIONS"
        d.text((cx - d.textlength(st, font=f2) / 2, py + 33), st, font=f2, fill=accent)

        # mini frames
        gap = 8
        total = mw * 3 + gap * 2
        x0 = cx - total / 2
        my = py + ph + 8
        for j, m in enumerate(minis):
            mx = int(x0 + j * (mw + gap))
            d.rounded_rectangle([mx, my, mx + mw, my + mh], radius=7,
                                fill=(13, 11, 24, 220), outline=accent + (120,), width=1)
            try:
                mm = cell(pdir, m)
                inner = mm.resize((mw - 10, int(mm.height * (mw - 10) / mm.width)),
                                  Image.LANCZOS)
                if inner.height > mh - 26:
                    inner = fit(mm, mh - 26)
                img.paste(inner, (int(mx + (mw - inner.width) / 2),
                                  int(my + (mh - 18 - inner.height) / 2 + 1)), inner)
            except Exception:
                pass
            lab = MINI_LABELS.get(m, m[:6])
            f3 = F(SANS_B, 10)
            lw = d.textlength(lab, font=f3)
            d.rectangle([mx + 2, my + mh - 16, mx + mw - 2, my + mh - 2],
                        fill=(13, 11, 24, 240))
            d.text((mx + mw / 2 - lw / 2, my + mh - 15), lab, font=f3, fill=(200, 192, 222))

    # footer
    fy = 1352
    d.line([(86, fy - 22), (W - 86, fy - 22)], fill=(44, 38, 70), width=1)
    total_anims = sum(p[4] for p in PETS)
    d.text((86, fy), "RUST · LINUX · WAYLAND · MIT", font=F(SANS_B, 17), fill=(134, 128, 166))
    mid = f"{total_anims} HAND-DRAWN ANIMATIONS"
    d.text((W / 2 - d.textlength(mid, font=F(SANS_B, 19)) / 2, fy - 2), mid,
           font=F(SANS_B, 19), fill=(226, 218, 238))
    rt = "niri · Hyprland"
    d.text((W - 86 - d.textlength(rt, font=F(SANS_B, 17)), fy), rt,
           font=F(SANS_B, 17), fill=(134, 128, 166))
    credit = "ART · hunmer/aboutme-pets      CODE · adarsh-67r/clawd      WAYLAND HOST · linxuan-sys"
    d.text((W / 2 - d.textlength(credit, font=F(SANS, 13)) / 2, fy + 32), credit,
           font=F(SANS, 13), fill=(106, 100, 134))

    img.save(OUT, optimize=True)
    print("wrote", OUT, img.size, os.path.getsize(OUT) // 1024, "KB")


if __name__ == "__main__":
    main()
