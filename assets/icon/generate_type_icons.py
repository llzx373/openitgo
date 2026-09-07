#!/usr/bin/env python3
"""生成 Windows 文件关联分组图标（压缩包/图片/影视，带右下角字母角标）。

用法：python assets/icon/generate_type_icons.py（路径写死，无参数）。
产物：Archive.ico / Image.ico / Media.ico，直接生成在 assets/icon/ 并入库，
由 openitgo.rc 以资源 ID 2/3/4 编入 exe，供 ProgID DefaultIcon 引用。

底图为 1024x1024.png，右下角叠加圆形角标（直径约边长 45%，浅色描边，
内嵌白色粗体字母：A=压缩包 Archive / P=图片 Image / V=影视 Media），
再按多尺寸缩放写出 ico（尺寸与 AppIcon.ico 对齐）。
"""

from pathlib import Path
from PIL import Image, ImageDraw, ImageFont

BASE = Path(__file__).parent / "1024x1024.png"
DST_DIR = Path(__file__).parent

# 与 AppIcon.ico 内嵌尺寸对齐。
ICO_SIZES = [(16, 16), (32, 32), (48, 48), (64, 64), (128, 128), (256, 256)]

FONT_PATH = Path("C:/Windows/Fonts/arialbd.ttf")

# (产物文件名, 角标字母, 角标底色)
TYPES = [
    ("Archive.ico", "A", (0xE8, 0x82, 0x1E)),  # #E8821E 橙
    ("Image.ico", "P", (0x2F, 0xA8, 0x4F)),  # #2FA84F 绿
    ("Media.ico", "V", (0x2B, 0x7C, 0xD3)),  # #2B7CD3 蓝
]

# 角标几何参数（相对边长的比例）。
BADGE_DIAMETER_RATIO = 0.45
BADGE_MARGIN_RATIO = 0.03
BADGE_STROKE_RATIO = 0.015
BADGE_FONT_RATIO = 0.60  # 字号 ≈ 角标直径的 60%

STROKE_COLOR = (255, 255, 255, 230)


def load_font(size: int) -> ImageFont.FreeTypeFont:
    """加载加粗字体；找不到时回退默认字体并警告。"""
    if FONT_PATH.exists():
        return ImageFont.truetype(str(FONT_PATH), size)
    print(f"警告：未找到字体 {FONT_PATH}，回退到默认点阵字体（角标文字可能偏小）")
    return ImageFont.load_default()


def make_badged(base: Image.Image, letter: str, color: tuple) -> Image.Image:
    """在底图右下角叠加圆形字母角标。"""
    img = base.copy()
    side = img.width
    diameter = int(side * BADGE_DIAMETER_RATIO)
    margin = int(side * BADGE_MARGIN_RATIO)
    stroke = max(2, int(side * BADGE_STROKE_RATIO))
    cx = side - margin - diameter // 2
    cy = side - margin - diameter // 2

    draw = ImageDraw.Draw(img)
    bbox = (cx - diameter // 2, cy - diameter // 2, cx + diameter // 2, cy + diameter // 2)
    # 先画描边（外圈），再缩进画底色。
    draw.ellipse(bbox, fill=STROKE_COLOR)
    inner = (bbox[0] + stroke, bbox[1] + stroke, bbox[2] - stroke, bbox[3] - stroke)
    draw.ellipse(inner, fill=(*color, 255))

    font = load_font(int(diameter * BADGE_FONT_RATIO))
    draw.text((cx, cy), letter, font=font, fill=(255, 255, 255, 255), anchor="mm")
    return img


def main() -> None:
    if not BASE.exists():
        raise FileNotFoundError(f"底图不存在: {BASE}")

    base = Image.open(BASE).convert("RGBA")

    for filename, letter, color in TYPES:
        badged = make_badged(base, letter, color)
        out = DST_DIR / filename
        badged.save(out, "ICO", sizes=ICO_SIZES)
        print(f"Generated {filename} sizes={ICO_SIZES}")


if __name__ == "__main__":
    main()
