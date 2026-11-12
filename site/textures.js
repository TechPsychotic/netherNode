// netherNode site: procedural 16×16 block textures, original to this project.
// Loaded by the page (for the 2D foreground) and by voxel.js (for the 3D worlds).
"use strict";

(function (root) {
  const S = 16;

  function rng(seed) {
    let s = (seed * 2654435761) >>> 0 || 1;
    return () => {
      s ^= s << 13; s >>>= 0; s ^= s >>> 17; s ^= s << 5; s >>>= 0;
      return s / 4294967296;
    };
  }
  // Tileable value noise over the 16×16 tile with `cells` lattice cells per side.
  function noise(r, cells) {
    const g = Array.from({ length: cells * cells }, r);
    const at = (i, j) => g[(((j % cells) + cells) % cells) * cells + (((i % cells) + cells) % cells)];
    return (x, y) => {
      const fx = (x / S) * cells, fy = (y / S) * cells;
      const x0 = Math.floor(fx), y0 = Math.floor(fy);
      const tx = fx - x0, ty = fy - y0;
      const u = tx * tx * (3 - 2 * tx), v = ty * ty * (3 - 2 * ty);
      const a = at(x0, y0), b = at(x0 + 1, y0), c = at(x0, y0 + 1), d = at(x0 + 1, y0 + 1);
      return a + (b - a) * u + (c - a) * v + (a - b - c + d) * u * v;
    };
  }
  // Tileable Voronoi: returns {id, d1, d2, cx, cy} for a pixel.
  function cells(r, n) {
    const pts = Array.from({ length: n }, () => [r() * S, r() * S]);
    return (x, y) => {
      let d1 = 1e9, d2 = 1e9, id = 0, cx = 0, cy = 0;
      pts.forEach(([px, py], i) => {
        let dx = Math.abs(x + 0.5 - px), dy = Math.abs(y + 0.5 - py);
        const sx = dx > S / 2 ? (x + 0.5 > px ? -1 : 1) : 0, sy = dy > S / 2 ? (y + 0.5 > py ? -1 : 1) : 0;
        dx = Math.min(dx, S - dx); dy = Math.min(dy, S - dy);
        const d = Math.sqrt(dx * dx + dy * dy);
        if (d < d1) { d2 = d1; d1 = d; id = i; cx = px - sx * S; cy = py - sy * S; } else if (d < d2) d2 = d;
      });
      return { id, d1, d2, cx, cy };
    };
  }
  const tone = (c, f) => [c[0] * f, c[1] * f, c[2] * f];
  const mix = (a, b, t) => [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t, a[2] + (b[2] - a[2]) * t];

  function canvasOf(fn, seed) {
    const d = new Uint8ClampedArray(S * S * 4);
    const r = rng(seed);
    const px = (x, y, c, a = 255) => {
      if (x < 0 || y < 0 || x >= S || y >= S) return;
      const i = (y * S + x) * 4;
      d[i] = c[0]; d[i + 1] = c[1]; d[i + 2] = c[2]; d[i + 3] = a;
    };
    const get = (x, y) => { const i = (((y + S) % S) * S + ((x + S) % S)) * 4; return [d[i], d[i + 1], d[i + 2]]; };
    fn({ px, get, r, n4: noise(r, 4), n8: noise(r, 8), n2: noise(r, 2) });
    return d;
  }
  const each = (f) => { for (let y = 0; y < S; y++) for (let x = 0; x < S; x++) f(x, y); };

  // ---------- base materials ----------
  const BASE = {
    stone({ px, r, n4, n8 }) {
      each((x, y) => px(x, y, tone([126, 126, 130], 0.84 + 0.2 * n4(x, y) + 0.1 * (n8(x, y) - 0.5) + (r() - 0.5) * 0.07)));
      for (let k = 0; k < 8; k++) {
        const x = r() * S | 0, y = r() * S | 0, len = 2 + (r() * 2 | 0), dark = r() < 0.7;
        for (let i = 0; i < len; i++) px(x + i, y + (i === len - 1 && r() < 0.5 ? 1 : 0), tone([126, 126, 130], dark ? 0.74 : 1.12));
      }
    },
    deepslate({ px, r, n4 }) {
      const rows = Array.from({ length: S }, () => 0.88 + r() * 0.16);
      each((x, y) => px(x, y, tone([78, 78, 88], rows[y] * (0.88 + 0.18 * n4(x, y)) + (r() - 0.5) * 0.06)));
      for (let k = 0; k < 6; k++) { const x = r() * S | 0, y = r() * S | 0; px(x, y, [52, 52, 60]); px(x + 1, y, [56, 56, 64]); }
    },
    dirt({ px, r, n4 }) {
      each((x, y) => px(x, y, tone([134, 96, 66], 0.84 + 0.22 * n4(x, y) + (r() - 0.5) * 0.12)));
      for (let k = 0; k < 14; k++) px(r() * S | 0, r() * S | 0, r() < 0.5 ? [102, 72, 48] : [160, 120, 86]);
    },
    grass_top({ px, r, n4, n8 }) {
      each((x, y) => px(x, y, tone([100, 166, 60], 0.8 + 0.2 * n4(x, y) + 0.12 * n8(x, y) + (r() - 0.5) * 0.1)));
      for (let k = 0; k < 10; k++) px(r() * S | 0, r() * S | 0, [72, 128, 42]);
    },
    snow({ px, r, n4 }) {
      each((x, y) => px(x, y, tone([240, 244, 252], 0.93 + 0.06 * n4(x, y) + (r() - 0.5) * 0.03)));
      for (let k = 0; k < 8; k++) px(r() * S | 0, r() * S | 0, [214, 224, 244]);
    },
    sand({ px, r, n4 }) {
      each((x, y) => px(x, y, tone([219, 207, 160], 0.92 + 0.08 * n4(x, y) + (r() - 0.5) * 0.06)));
      for (let k = 0; k < 12; k++) px(r() * S | 0, r() * S | 0, r() < 0.6 ? [196, 182, 134] : [236, 228, 190]);
    },
    gravel({ px, r }) {
      const v = cells(r, 11), cols = Array.from({ length: 11 }, () => [[128, 120, 116], [104, 98, 96], [150, 142, 136], [96, 86, 80], [122, 112, 100]][r() * 5 | 0]);
      each((x, y) => { const c = v(x, y); px(x, y, tone(cols[c.id], c.d2 - c.d1 < 0.9 ? 0.72 : 0.95 + (r() - 0.5) * 0.1)); });
    },
    cobble({ px, r }) {
      const v = cells(r, 7), f = Array.from({ length: 7 }, () => 0.84 + r() * 0.3);
      each((x, y) => {
        const c = v(x, y);
        if (c.d2 - c.d1 < 1.15) return px(x, y, [70, 70, 74]);
        const lean = (x + 0.5 - c.cx) + (y + 0.5 - c.cy);
        px(x, y, tone([124, 124, 128], f[c.id] * (lean < -2 ? 1.12 : lean > 2.5 ? 0.86 : 1) + (r() - 0.5) * 0.06));
      });
    },
    log_side({ px, r, n4 }) {
      const colF = Array.from({ length: S }, () => 0.82 + r() * 0.3);
      each((x, y) => {
        const stripe = x % 4 === 0 || (x % 7 === 3 && r() < 0.6);
        px(x, y, tone(stripe ? [74, 55, 33] : [108, 82, 50], colF[x] * (0.92 + 0.14 * n4(x, y))));
      });
    },
    log_top({ px }) {
      each((x, y) => {
        const d = Math.hypot(x - 7.5, y - 7.5);
        if (Math.max(Math.abs(x - 7.5), Math.abs(y - 7.5)) > 6.6) return px(x, y, [92, 70, 42]);
        px(x, y, Math.floor(d * 1.1) % 2 ? [166, 134, 84] : [144, 112, 66]);
      });
    },
    leaves({ px, r, n8 }) {
      each((x, y) => px(x, y, r() < 0.16 ? [30, 64, 24] : tone([60, 132, 46], 0.7 + 0.5 * n8(x, y) + (r() - 0.5) * 0.15)));
    },
    planks({ px, r, n8 }) {
      const boardF = [0, 1, 2, 3].map(() => 0.9 + r() * 0.14);
      each((x, y) => {
        const b = y >> 2, seam = (b * 7 + 3) % S;
        if (y % 4 === 3) return px(x, y, [104, 78, 46]);
        if (x === seam) return px(x, y, [118, 90, 54]);
        const grain = 0.94 + 0.1 * Math.sin(x * 0.9 + b * 3 + n8(x, y) * 4);
        px(x, y, tone([162, 128, 78], boardF[b] * grain + (y % 4 === 0 ? 0.05 : 0)));
      });
    },
    netherrack({ px, r, n8, n4 }) {
      each((x, y) => px(x, y, tone([112, 50, 48], 0.78 + 0.28 * n8(x, y) + 0.1 * n4(x, y) + (r() - 0.5) * 0.14)));
      for (let k = 0; k < 9; k++) { const x = r() * S | 0, y = r() * S | 0; px(x, y, [76, 28, 28]); px(x + 1, y, [84, 32, 30]); px(x, y + 1, [150, 74, 68]); }
    },
    nether_brick({ px, r }) {
      const bf = new Map();
      each((x, y) => {
        const row = y >> 2, off = row % 2 ? 4 : 0, bx = Math.floor((x + off) / 8);
        if (y % 4 === 3 || (x + off) % 8 === 7) return px(x, y, [34, 16, 20]);
        const key = row * 10 + bx;
        if (!bf.has(key)) bf.set(key, 0.82 + r() * 0.3);
        px(x, y, tone([72, 34, 42], bf.get(key) * (y % 4 === 0 ? 1.15 : 1) + (r() - 0.5) * 0.08));
      });
    },
    glowstone({ px, r }) {
      const v = cells(r, 7), cols = [[255, 218, 132], [236, 168, 88], [255, 240, 186], [214, 146, 70]];
      const pick = Array.from({ length: 7 }, () => cols[r() * 4 | 0]);
      each((x, y) => { const c = v(x, y); px(x, y, c.d2 - c.d1 < 0.8 ? [150, 100, 54] : tone(pick[c.id], 0.94 + r() * 0.1)); });
    },
    magma({ px, r, n4 }) {
      const v = cells(r, 6);
      each((x, y) => { const c = v(x, y); px(x, y, c.d2 - c.d1 < 1.0 ? mix([255, 120, 30], [255, 190, 70], r()) : tone([82, 28, 18], 0.8 + 0.4 * n4(x, y))); });
    },
    soul_sand({ px, r, n8 }) {
      each((x, y) => px(x, y, tone([86, 66, 52], 0.78 + 0.34 * n8(x, y) + (r() - 0.5) * 0.1)));
      for (let k = 0; k < 4; k++) { const x = r() * 13 | 0, y = r() * 12 | 0; px(x, y, [50, 36, 28]); px(x + 2, y, [50, 36, 28]); px(x + 1, y + 2, [46, 32, 24]); }
    },
    endstone({ px, r, n4 }) {
      each((x, y) => px(x, y, tone([222, 224, 166], 0.9 + 0.1 * n4(x, y) + (r() - 0.5) * 0.05)));
      for (let k = 0; k < 9; k++) { const x = r() * S | 0, y = r() * S | 0; px(x, y, [188, 190, 134]); px(x + 1, y + 1, [242, 244, 194]); }
    },
    purpur({ px, r, n4 }) {
      each((x, y) => {
        const e = x % 8 === 0 || y % 8 === 0, hi = x % 8 === 1 || y % 8 === 1;
        px(x, y, e ? [124, 86, 136] : tone([170, 126, 172], (hi ? 1.08 : 0.94) + 0.08 * n4(x, y) + (r() - 0.5) * 0.04));
      });
    },
    purpur_pillar({ px, r }) {
      each((x, y) => px(x, y, x % 5 === 0 ? [124, 86, 136] : tone([172, 128, 174], 0.94 + (x % 5 === 1 ? 0.1 : 0) + (r() - 0.5) * 0.05)));
    },
    obsidian({ px, r, n8 }) {
      each((x, y) => px(x, y, tone([24, 17, 36], 0.75 + 0.6 * n8(x, y))));
      for (let k = 0; k < 7; k++) { const x = r() * 14 | 0, y = r() * 15 | 0; px(x, y, [66, 42, 104]); px(x + 1, y, [84, 56, 130]); }
    },
    window({ px }) {
      each((x, y) => px(x, y, x === 0 || y === 0 || x === 15 || y === 15 || x === 8 || y === 8 ? [116, 64, 128] : [255, 150, 236]));
    },
    lava({ px, r, n4, n8 }) {
      each((x, y) => {
        const a = n8(x, y), b = n4(x, y);
        px(x, y, a > 0.68 ? [255, 214, 90] : a < 0.22 ? [176, 46, 10] : tone([236, 104, 22], 0.85 + 0.3 * b + (r() - 0.5) * 0.06));
      });
    },
    water({ px, r, n4 }) { each((x, y) => px(x, y, tone([46, 86, 196], 0.85 + 0.22 * n4(x, y) + (r() - 0.5) * 0.04))); },
    dripstone({ px, r, n4 }) {
      each((x, y) => px(x, y, tone([136, 108, 92], 0.84 + 0.2 * n4(x, y) + (x % 5 === 2 ? -0.1 : 0) + (r() - 0.5) * 0.06)));
    },
    amethyst({ px, r }) {
      const v = cells(r, 6);
      each((x, y) => { const c = v(x, y); const lean = (x - c.cx) + (y - c.cy); px(x, y, c.d2 - c.d1 < 0.9 ? [92, 54, 160] : lean < 0 ? [214, 180, 255] : [150, 98, 226]); });
    },
    // End portal frame: pale stone with a teal trim, and an eye set into the top.
    end_frame_side({ px, r, n4 }) {
      each((x, y) => px(x, y, y < 4 ? tone([58, 104, 92], 0.85 + 0.25 * n4(x, y)) : y === 4 ? [40, 74, 64] : tone([206, 212, 168], 0.9 + 0.1 * n4(x, y) + (r() - 0.5) * 0.05)));
    },
    end_frame_top({ px, r, n4 }) {
      each((x, y) => {
        const ring = x === 0 || y === 0 || x === 15 || y === 15 || x === 1 || y === 1 || x === 14 || y === 14;
        px(x, y, ring ? tone([58, 104, 92], 0.85 + 0.2 * n4(x, y)) : tone([206, 212, 168], 0.9 + 0.1 * n4(x, y) + (r() - 0.5) * 0.05));
      });
      for (let y = 5; y <= 10; y++) for (let x = 5; x <= 10; x++) {
        const d = Math.max(Math.abs(x - 7.5), Math.abs(y - 7.5));
        px(x, y, d > 2 ? [34, 84, 68] : d > 1 ? [96, 186, 150] : [12, 22, 20]);
      }
      px(6, 6, [200, 255, 230]);
    },
    torch({ px }) {
      each((x, y) => px(x, y, y < 4 ? (y < 2 ? [255, 248, 200] : [255, 186, 70]) : [124, 92, 52]));
    },
  };

  // Ores: a host rock plus clusters with a lit and a shaded side.
  const ORE = {
    coal: [[34, 32, 36], [52, 50, 56], [76, 74, 80]],
    iron: [[160, 112, 86], [212, 166, 132], [238, 210, 186]],
    gold: [[196, 146, 26], [246, 212, 74], [255, 246, 170]],
    diamond: [[22, 150, 146], [84, 230, 222], [200, 255, 250]],
    emerald: [[18, 128, 62], [70, 214, 118], [190, 255, 210]],
    amethyst: [[96, 52, 196], [168, 120, 255], [232, 210, 255]],
    quartz: [[184, 172, 162], [232, 224, 216], [255, 255, 255]],
  };
  function ore(host, kind) {
    return (t) => {
      BASE[host](t);
      const [dk, md, lt] = ORE[kind], { px, r } = t;
      for (let k = 0; k < 4; k++) {
        const cx = 2 + (r() * 11 | 0), cy = 2 + (r() * 11 | 0);
        const shape = [[0, 0], [1, 0], [0, 1], [1, 1], [-1, 0], [0, -1], [2, 1], [1, 2]].filter((_, i) => i < 4 || r() < 0.5);
        shape.forEach(([dx, dy]) => px(cx + dx, cy + dy + 1, tone(md, 0.45))); // shadow under the cluster
        shape.forEach(([dx, dy]) => px(cx + dx, cy + dy, dx + dy < 0 ? lt : dx + dy > 1 ? dk : md));
      }
    };
  }
  // Side faces where a top material drapes over dirt.
  function draped(top, depth) {
    return (t) => {
      BASE.dirt(t);
      const g = canvasOf(BASE[top], 77), { px, r } = t;
      for (let x = 0; x < S; x++) {
        const h = depth + (r() < 0.6) + (r() < 0.3);
        for (let y = 0; y < h; y++) { const i = (y * S + x) * 4; px(x, y, tone([g[i], g[i + 1], g[i + 2]], top === "grass_top" ? 0.9 : 1)); }
        if (r() < 0.25) { const i = (h * S + x) * 4; px(x, h, tone([g[i], g[i + 1], g[i + 2]], 0.75)); }
      }
    };
  }

  const DEFS = {
    ...BASE,
    grass_side: draped("grass_top", 3),
    snow_side: draped("snow", 2),
    coal_ore: ore("stone", "coal"), iron_ore: ore("stone", "iron"), gold_ore: ore("stone", "gold"),
    diamond_ore: ore("stone", "diamond"), emerald_ore: ore("stone", "emerald"), amethyst_ore: ore("stone", "amethyst"),
    deepslate_diamond: ore("deepslate", "diamond"), deepslate_gold: ore("deepslate", "gold"),
    quartz_ore: ore("netherrack", "quartz"),
  };

  const cache = new Map();
  function get(name, v = 0) {
    const key = name + "#" + v;
    if (!cache.has(key)) {
      let h = 7;
      for (const ch of name) h = (h * 31 + ch.charCodeAt(0)) >>> 0;
      cache.set(key, canvasOf(DEFS[name] || DEFS.stone, h + v * 977));
    }
    return cache.get(key);
  }
  // Page only: the texture as a 16×16 canvas.
  const canvases = new Map();
  function canvas(name, v = 0) {
    const key = name + "#" + v;
    if (!canvases.has(key)) {
      const c = document.createElement("canvas");
      c.width = c.height = S;
      c.getContext("2d").putImageData(new ImageData(new Uint8ClampedArray(get(name, v)), S, S), 0, 0);
      canvases.set(key, c);
    }
    return canvases.get(key);
  }

  root.NNTextures = { get, canvas, names: Object.keys(DEFS), ORE, S };
})(typeof self !== "undefined" ? self : this);
