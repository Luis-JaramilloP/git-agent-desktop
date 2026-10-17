"""Genera los iconos de la aplicacion (PNG + ICO) para Tauri e Inno Setup."""

from pathlib import Path

from PIL import Image, ImageDraw

ROOT = Path(__file__).resolve().parents[1]
ICONS = ROOT / "src-tauri" / "icons"
INSTALLER_ASSETS = ROOT / "installer" / "assets"

SIZE = 1024


def lerp(a, b, t):
    return tuple(round(x + (y - x) * t) for x, y in zip(a, b))


def rounded_mask(size, radius):
    mask = Image.new("L", (size, size), 0)
    draw = ImageDraw.Draw(mask)
    draw.rounded_rectangle((0, 0, size - 1, size - 1), radius=radius, fill=255)
    return mask


def build_base():
    img = Image.new("RGBA", (SIZE, SIZE), (0, 0, 0, 0))
    gradient = Image.new("RGBA", (SIZE, SIZE))

    top_left = (34, 108, 196)
    bottom_right = (139, 84, 214)
    px = gradient.load()
    for y in range(SIZE):
        for x in range(0, SIZE, 4):
            t = (x + y) / (2 * SIZE)
            color = lerp(top_left, bottom_right, t)
            for dx in range(4):
                if x + dx < SIZE:
                    px[x + dx, y] = (*color, 255)

    mask = rounded_mask(SIZE, int(SIZE * 0.22))
    img.paste(gradient, (0, 0), mask)

    draw = ImageDraw.Draw(img)
    white = (255, 255, 255, 235)
    line_w = int(SIZE * 0.052)

    # Nodos del grafo de commits (mismo glifo que la cabecera de la interfaz).
    cy_top = int(SIZE * 0.28)
    cy_bottom = int(SIZE * 0.72)
    cy_right = int(SIZE * 0.50)
    cx_left = int(SIZE * 0.33)
    cx_right = int(SIZE * 0.67)
    node_r = int(SIZE * 0.085)

    # Tronco vertical
    draw.line((cx_left, cy_top, cx_left, cy_bottom), fill=white, width=line_w)
    # Rama hacia el nodo derecho
    draw.line((cx_left, cy_right, cx_right, cy_right), fill=white, width=line_w)

    for cy in (cy_top, cy_bottom):
        draw.ellipse(
            (cx_left - node_r, cy - node_r, cx_left + node_r, cy + node_r),
            fill=(255, 255, 255, 255),
        )

    draw.ellipse(
        (cx_right - node_r, cy_right - node_r, cx_right + node_r, cy_right + node_r),
        fill=(255, 255, 255, 255),
    )

    # Punto de acento (aprobacion humana)
    accent_r = int(SIZE * 0.045)
    draw.ellipse(
        (cx_right - accent_r, cy_right - accent_r, cx_right + accent_r, cy_right + accent_r),
        fill=(86, 211, 100, 255),
    )

    return img


def main():
    ICONS.mkdir(parents=True, exist_ok=True)
    INSTALLER_ASSETS.mkdir(parents=True, exist_ok=True)

    base = build_base()

    sizes = {
        "32x32.png": 32,
        "128x128.png": 128,
        "128x128@2x.png": 256,
        "icon.png": 512,
        "Square30x30Logo.png": 30,
        "Square44x44Logo.png": 44,
        "Square71x71Logo.png": 71,
        "Square89x89Logo.png": 89,
        "Square107x107Logo.png": 107,
        "Square142x142Logo.png": 142,
        "Square150x150Logo.png": 150,
        "Square284x284Logo.png": 284,
        "Square310x310Logo.png": 310,
        "StoreLogo.png": 50,
    }

    for name, size in sizes.items():
        base.resize((size, size), Image.LANCZOS).save(ICONS / name, format="PNG")

    # ICO multi-tamano para Windows (Tauri + Inno Setup)
    ico_sizes = [16, 24, 32, 48, 64, 128, 256]
    base.save(ICONS / "icon.ico", format="ICO", sizes=[(s, s) for s in ico_sizes])
    base.resize((256, 256), Image.LANCZOS).save(
        INSTALLER_ASSETS / "app_icon.ico", format="ICO",
        sizes=[(s, s) for s in ico_sizes],
    )
    base.resize((256, 256), Image.LANCZOS).save(INSTALLER_ASSETS / "app_icon.png", format="PNG")

    print("Iconos generados:")
    for path in sorted(ICONS.iterdir()):
        print(f"  {path.name} ({path.stat().st_size} bytes)")
    for path in sorted(INSTALLER_ASSETS.iterdir()):
        print(f"  installer/assets/{path.name} ({path.stat().st_size} bytes)")


if __name__ == "__main__":
    main()
