# Making a Gentle Shell app theme

An app theme changes the desktop app's colors and can put an animated scene
behind the panes (the panes are glass, so it shows through them). Themes live
in this folder; the app notices new or edited files within two seconds, no
restart. The person picks one in Configuration › Theme › Animated backgrounds.

Never put these files in Pi's own `themes` folder: Pi's theme schema rejects
the `background` key.

## Workflow

1. Read two shipped examples close to the request in `examples/` (the theme
   `<name>.json` and its scene `<name>.js`). They are references only: the
   app doesn't load themes from that folder.
2. Sketch the idea in words before writing code: one subject (koi, lanterns,
   an aurora), two or three supporting elements (lily pads, ripples, stars),
   where each one sits, how it moves, and a palette of 3–5 colors.
3. Write `<stem>.json` and `<stem>.js` here.
4. Wait a few seconds and look at `previews/<stem>.png`; read
   `previews/<stem>.txt`. Check it against "Common failures" below.
5. Fix and look again. Iterate at least once, even when the first preview
   seems fine; stop when it looks good and runs at full rate.

`<stem>` is the theme's name lowercased with dashes for everything else:
"Rainy Cafe" → `rainy-cafe`.

## Files

- `<stem>.json`: the theme.
- `<stem>.js`: its scene, if it has its own (optional).
- An image beside it, if it uses one (optional: png, jpg, gif, webp).
- `previews/<stem>.png`: written by the app a few seconds after the theme or
  its scene is saved, active or not. A 2×2 grid of four moments of the scene
  (scene time 2, 8, 20 and 45 s), each a 1280×800 window shown at half size,
  with the sky, image and veil as the app draws them. Open it with your file
  reading tool; it shows images.
- `previews/<stem>.txt`: the theme, when it was rendered, the frame cost
  (average and worst ms), commands and points per frame, the frame rate it
  will run at, and the error with its line if the scene failed.
- `scene-error.txt` appears here when the active theme's scene fails, with
  the error and the line; it goes away once the scene draws.

## The theme file

```json
{
  "name": "Rainy Cafe",
  "bg": "#2a211c",
  "text": "#f2e6d8",
  "accent": "#e0a458",
  "background": {
    "scene": "rainy-cafe.js",
    "sky": ["#1c1512", "#2a211c", "#3a2a20"],
    "speed": 1,
    "veil": 0.5
  }
}
```

- `name`: shown in the picker; unique (a theme with the same name as a
  shipped example replaces it).
- `bg`, `text`, `accent`: `#rrggbb`. Every other UI color is derived from
  them. Keep `text` readable on `bg` (contrast 7:1 or more) and the accent
  3:1 or more. Pi's full format (`vars`, `colors` with `muted`, `dim`,
  `border`, `success`, `error`, `warning`, `toolTitle`, `userMessageBg`,
  `selectedBg`, …, and `export.pageBg`) also works when you want finer control.
- `background` (all optional):
  - `scene`: a built-in scene's name (see below) or a `.js` file beside the
    theme (plain file name, no folders). The examples name built-in scenes;
    your own theme names its own file, `<stem>.js`.
  - `image`: an image file beside the theme, drawn to cover the window
    (animated GIFs play). `dim` (0–0.9) darkens it toward `bg`.
  - `camera` (with an image): a slow "Ken Burns" move over it, so a still
    picture feels filmed: `{"zoom": 0.06, "pan": 0.02, "period": 90,
    "focus": [0.5, 0.5]}`. `zoom` (0–0.3) is how much further in it goes,
    `pan` (0–0.15) how far it drifts sideways (a share of the window
    width), `period` (10–900) the seconds per cycle, `focus` the image
    point (0–1 across, 0–1 down) it moves toward. The image always covers
    the window. Keep it subtle: zoom 0.04–0.1, pan 0.01–0.03, period 60–180.
  - `sky`: colors of a vertical gradient under everything, top to bottom.
  - `speed`: seconds of scene time per second (0 holds it still; up to 16).
  - `cell`: the ASCII cell size in pixels for cell scenes (default 22).
  - `veil`: how much the middle of the window, where the conversation is,
    is calmed toward `bg` (0–1, default 0.5). The sides stay lively.

Built-in scenes: BUILTIN_SCENES.

## Scenes

A scene is JavaScript, run sandboxed: no files, network, timers, DOM or
modules, only computing and drawing. A frame may take at most 250 ms and
memory is capped. The app keeps a scene's script under 8% of one core: a
frame that takes 3 ms or less runs at 24 fps; a heavier one is shown less
often (down to 8 fps) and looks choppier, so keep frames cheap. The file
registers itself:

```js
SCENES['rainy-cafe'] = {
  draw(g, t) { /* pixels */ },
  cell(x, y, cols, rows, t) { /* ASCII cells */ },
  sprites(cols, rows, t) { /* ASCII sprites */ },
  size: 22, // optional: the largest cell size the scene allows
};
```

Use `draw`, the ASCII pair, or both (`draw` paints first). `t` is seconds.
Pixel scenes (`draw`) give the smoothest shapes; `aurora` and `petals` are
pixel examples, the rest are ASCII.

### Pixels: `draw(g, t)`

`g.width`, `g.height` (logical pixels), `g.theme` (`bg`, `text`, `muted`,
`accent`, `info`, `success`, `warning`, `danger`, `light`). Colors are
`'#rgb'`, `'#rrggbb'`, `'#rrggbbaa'` or `'rgba(r, g, b, a)'`; `alpha` is
0–1 and multiplies the color's own.

Shapes:

- `g.rect(x, y, w, h, color, alpha = 1, radius = 0)`
- `g.circle(x, y, r, color, alpha = 1)`
- `g.ellipse(x, y, rx, ry, color, alpha = 1)`
- `g.line(x1, y1, x2, y2, color, alpha = 1, width = 1)`
- `g.poly(points, color, alpha = 1)`: filled and closed; `points` is a flat
  array `[x0, y0, x1, y1, …]`.
- `g.polyline(points, color, alpha = 1, width = 1)`: an open stroke.
- `g.text(x, y, text, color, alpha = 1, size = 13, bold = false)`: centered
  on (x, y), monospace; emoji work too.
- `g.glow(x, y, r, color, alpha = 1)`: a soft radial light.
- `g.gradient(x, y, w, h, angle, from, to, alpha = 1)`: linear; 180 is top
  to bottom, 90 left to right.

Paths: `path()` (anywhere, also at the top of the file) and `g.path()`
return the same builder; every step returns it, so calls chain.

- `moveTo(x, y)`, `lineTo(x, y)`, `quadTo(cx, cy, x, y)`,
  `curveTo(c1x, c1y, c2x, c2y, x, y)`, `close()`.
- `arc(cx, cy, r, from, to)`: angles in radians; joins the current subpath.
- `ellipse(cx, cy, rx, ry)`, `rect(x, y, w, h)`: each its own closed subpath.
- Draw it with `p.fill(color, alpha = 1)`, `p.stroke(color, alpha = 1,
  width = 1)` or `p.fillGradient(angle, from, to, alpha = 1)` (linear over
  the path's bounds; 180 is top to bottom).

Transforms apply to everything drawn after them:

- The transform resets to none at the start of every frame.
- `g.save()`, `g.restore()`: push and pop the current transform.
- `g.translate(x, y)`, `g.rotate(radians)`, `g.scale(sx, sy = sx)`,
  `g.resetTransform()`.
- Text keeps upright glyphs: its position moves and its size scales.
- `g.gradient` follows translate and scale but not rotation.
- Under rotation or uneven scale, `rect` and `ellipse` are drawn as polygons.

A path is in its own coordinates; the current transform applies when it is
drawn. Build a shape once in unit size and draw it many times:

```js
// A petal 1 unit long, pointing right, notched at the tip; built once.
const PETAL = path()
  .moveTo(0, 0).curveTo(.3, -.45, .75, -.35, 1, -.1).lineTo(.88, 0).lineTo(1, .1)
  .curveTo(.75, .35, .3, .45, 0, 0).close();

SCENES['petal-wind'] = {
  draw(g, t) {
    const W = g.width, H = g.height;
    for (let i = 0; i < 70; i++) {
      const near = hash(i, 1) > .7, size = near ? 22 : 9 + 6 * hash(i, 2);
      const fall = near ? 34 : 14, span = H + 80;
      const y = (hash(i, 3) * span + t * fall) % span - 40;
      const x = (hash(i, 4) * (W + 80) + t * fall * .6 + 50 * noise(i, t * .08)) % (W + 80) - 40;
      g.save();
      g.translate(x, y);
      g.rotate(t * (.4 + hash(i, 5)) + i);
      g.scale(size, size * (.35 + .65 * Math.abs(Math.sin(t * .9 + i)))); // tumbling
      PETAL.fillGradient(180, '#ffe1ec', '#f08cb4', near ? .9 : .45);
      g.restore();
    }
  },
};
```

A shape that changes every frame (a stalk bending, a wave line) is built in
`draw`:

```js
// Seaweed at both edges, bent by a current; each stalk has its own phase.
for (let i = 0; i < 10; i++) {
  const x0 = i < 5 ? W * (.02 + i * .035) : W * (.82 + (i - 5) * .035);
  const h = H * (.25 + .2 * hash(i, 1)), pts = [];
  for (let k = 0; k <= 10; k++) {
    const u = k / 10;
    pts.push(x0 + fbm(i * 3.7, u * 1.4 - t * .12) * 70 * u, H - h * u);
  }
  g.polyline(pts, '#3f8a5a', .8, 5 - 3 * hash(i, 2));
}
```

Helpers for both kinds of scene:

- `hash(a, b, c?)`: stable random 0–1 per input; use it to give each element
  its own size, speed and phase.
- `noise(x, y, z = 0)`: smooth Perlin noise, -1…1. Continuous, unlike
  `hash`: feed it time for wandering motion.
- `fbm(x, y, z = 0, octaves = 4)`: layered noise for clouds, water, wind.
- `lerp(a, b, k)`, `clamp(v, lo, hi)`, `smoothstep(e0, e1, x)`, `TAU` (2π).
- Colors: `rgb(color)` gives `[r, g, b]` in 0–1, `hex([r, g, b])` the
  reverse, `mix(a, b, k)` blends two such arrays, `shade(color, k)` moves a
  color toward white (k > 0) or black (k < 0).

There are no blend modes (no additive light); use `glow` for light.

### ASCII: `cell` and `sprites`

The window is a grid of `cols` × `rows` cells. This is the format of
claude-desktop-themes' live wallpapers: their scene files work unchanged.

- `cell(x, y, cols, rows, t)` returns, for one cell, a tile
  `{ bg, fg, a }` (a rounded square with a glyph), a loose character
  `{ fg, a, ch? }`, or nothing. Without `ch` the glyph is picked and
  shimmers on its own.
- `sprites(cols, rows, t)` returns pictures placed in cell units:
  `{ x, y, key, art, pal }` where `art` is rows of characters and `pal` maps
  each character to a cell (`.` is empty), or `{ x, y, key, w, h, field }`
  where `field(gx, gy)` returns `{ c: cell }` or nothing. Optional: `a`
  (opacity), `shadow` (a color), `glow: { r, color, a }`, and for `art`
  sprites `halo` (a color).
- ASCII helpers: `glyph(set, x, y, t)`, `ell(x, y, cx, cy, rx, ry)` (≤ 1
  inside an ellipse), `drift(cols, f, t, v, w)`, `near`,
  `twinkle(x, y, t, fg, odds)`, `rising(x, y, t, v, odds)`,
  `sketch(w, h, [[row, col, 'text'], …])`, `shape(w, h, (x, y) => ch)`,
  `flip(art)`, `frame(frames, t, rate, phase?)`, `tint(bg, fg, alphas)`,
  `GLYPHS`, `LOOSE`.

`koi-pond` shows a `field` sprite whose body follows a moving spine (the
tail beats, the fins paddle); `lantern-night` shows `art` sprites with
animation frames, glow, reflections, and near and far versions for depth.

## Realistic themes start from a picture

Shapes drawn in code look like illustrations, however careful. When the
user wants something realistic (a photo, a painting, a film still), the
picture has to come from them, and the scene brings it to life:

1. Ask the user for an image (png, jpg or webp; wide, at least 1920 px; a
   GIF already moves). Copy it beside the theme and name it in `image`.
2. Look at it before writing anything: where the light sources are (windows,
   lamps, sun, moon), what could move (smoke, water, leaves, clouds, fog,
   fireflies, snow, rain), its palette, and where the calm middle falls.
   Write that down as image coordinates, 0–1 across and down.
3. Add a `camera` (subtle) and pick `bg`, `text` and `accent` from the
   picture's own colors, so the UI belongs to it.
4. Draw only what the picture lacks, pinned to it with
   `const [x, y] = g.fromImage(u, v)` so it stays put while the camera
   moves; size it by `g.image.w` so it scales with the picture:
   - a window or lantern that flickers: a `glow` in the picture's light
     color, alpha breathing with `noise(t * 2)`, never a hard blink;
   - smoke from a chimney: a column of soft, growing, fading `glow`s or
     ellipses drifting with `fbm`, rising from the chimney's point;
   - mist over water or a valley: wide, faint ellipses sliding slowly in
     two or three layers;
   - fireflies, snow, rain, falling petals or leaves: particles whose size,
     speed and alpha shrink with distance, colored from the picture.
5. Match the picture: same light direction and color, nothing brighter than
   its brightest part, nothing sharper than its own detail. Effects should
   be discovered, not announced; if an effect is the first thing you see in
   the preview, it is too strong.

In a scene, `g.image` is `{ x, y, w, h }` (where the picture is this frame,
camera included) or `null` without an image, and `g.fromImage(u, v)`
returns the window point of the picture point (u, v). A theme with an image
and a camera but no scene of its own still moves.

## Design

### Composition

- One hero: the thing the theme is named after, clearly the largest and
  most detailed. Everything else supports it.
- Put the hero and the big shapes at the sides and corners, on the thirds
  of the window (x near 0.15 or 0.85 of `g.width`). The middle holds the
  conversation and the veil calms it: keep it to small, sparse, slow things.
- Leave negative space. Empty sky or water is part of the picture.
- Size and place everything relative to `g.width` and `g.height` (or `cols`
  and `rows`) so a small window still works.

### Depth

Use three layers, far, mid and near:

| Layer | Size  | Speed  | Alpha    | Color                         |
|-------|-------|--------|----------|-------------------------------|
| Far   | small | slow   | 0.2–0.4  | mixed 50–70% toward the sky   |
| Mid   | mid   | mid    | 0.5–0.7  | mixed 20–30% toward the sky   |
| Near  | large | fast   | 0.8–1    | full color, the highlights    |

Draw far first. Few near elements, more far ones. `lantern-night` drifts far
lanterns small, dim and slow and near ones large, bright and faster.

### Palette

- 3–5 colors, derived from the theme's `bg` and `accent` so the scene and
  the UI belong together:

  ```js
  const T = g.theme;
  const deep = shade(T.bg, -.35), haze = hex(mix(rgb(T.accent), rgb(T.bg), .6));
  const body = shade(T.accent, -.15), light = shade(T.accent, .45);
  ```

- One accent highlight used sparingly (a single glowing lantern, a gold
  koi), not on everything.
- The `sky` gradient sets the mood: 3–5 close shades of `bg`, lighter where
  the light comes from.
- The UI's text sits on top: keep the scene's midtones away from the
  `text` color and avoid bright, busy detail behind the middle.

### Motion

- Organic, not mechanical: drive positions and angles with `noise(i, t * k)`
  or several sines of different speeds, not one straight line.
- Give each element its own phase and speed from `hash(i, …)` so nothing
  moves in lockstep.
- Bodies move, not only slide: tails beat, petals tumble (scale one axis by
  `|sin|`), flames flicker, branches sway, lights breathe.
- Ambient cycles of 10–60 s for the big things (a drifting glow, a swell);
  only small things move fast. Ease starts and stops with `smoothstep`.
- Nothing strobes or blinks hard: twinkles fade in and out.
- The scene must look alive at t = 2 s: place elements spread over their
  path at t = 0 (`(hash(i) * span + t * v) % span`), never all entering from
  one edge.

### Detail

- Shade shapes: `fillGradient` from a light top to a darker bottom, or a
  darker copy offset a few pixels behind as a shadow.
- Add a small highlight on the near layer (a light ellipse on a fish's back,
  a rim on a lantern).
- Glow sparingly: a few large, faint glows set the light; small glows mark
  a few light sources. Dozens of glows look foggy and cost time.
- Prefer one curved path over a stack of rects and lines.

### Common failures (check the preview for these)

- Everything the same size, speed or alpha: no depth.
- A uniform grid or tiling, or elements spaced evenly.
- Too dense: the picture reads as noise; halve the count.
- Muddy contrast: the scene's colors too close to each other or to `bg`, or
  bright detail fighting the text.
- Flat shapes built from lines and rects where a path would do.
- Something big in the middle of the window.
- The first frames empty or all four moments looking the same.
- A shape cut off at the window edge in an obviously wrong way.

## Performance

- Build static paths, palettes and positions once at the top of the file
  and reuse them; draw them with transforms.
- A few hundred to about 2000 shapes per frame; no per-pixel loops.
- In ASCII scenes, `cell` runs for every cell: skip shapes by their box
  first and compute per-frame values once, not per cell (`koi-pond` does
  both).
- Read the frame cost and frame rate in `previews/<stem>.txt`. Under 3 ms
  runs at the full 24 fps; above that, cut counts, glows or path points.

## Sharing

The share button on a theme's card saves it as one `<name>.gentletheme` file
with its scene and image inside. Anyone imports it in Configuration › Theme
(Import…, a link, or dropping the file).
