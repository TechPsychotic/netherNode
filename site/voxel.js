// netherNode site: builds one zone's block world in a Web Worker and turns it into meshes.
// Each zone has a straight walkway along +x at z = PZ where the story happens. Lighting is
// baked into vertex colours the way block games do it: block light and sky light flood
// outwards one level per block, and every vertex gets smooth light plus corner shading.
"use strict";
importScripts("textures.js");

const TEXN = NNTextures.names, TI = Object.fromEntries(TEXN.map((n, i) => [n, i]));
const ACOLS = 8, AROWS = Math.ceil(TEXN.length / ACOLS);

// ---------- blocks ----------
const B = {};
const DEFS = [
  ["air"],
  ["stone", ["stone"]], ["dirt", ["dirt"]], ["grass", ["grass_top", "grass_side", "dirt"]], ["snowy", ["snow", "snow_side", "dirt"]],
  ["snow", ["snow"]], ["sand", ["sand"]], ["water", ["water"]], ["lava", ["lava"], 15], ["log", ["log_top", "log_side", "log_top"]],
  ["leaves", ["leaves"]], ["planks", ["planks"]], ["cobble", ["cobble"]], ["gravel", ["gravel"]], ["deepslate", ["deepslate"]],
  ["coal", ["coal_ore"]], ["iron", ["iron_ore"]], ["gold", ["gold_ore"]], ["diamond", ["deepslate_diamond"]], ["emerald", ["emerald_ore"]],
  ["amethyst", ["amethyst"], 10], ["netherrack", ["netherrack"]], ["nbrick", ["nether_brick"]], ["glowstone", ["glowstone"], 15],
  ["magma", ["magma"], 5], ["soulsand", ["soul_sand"]], ["quartz", ["quartz_ore"]], ["endstone", ["endstone"]], ["purpur", ["purpur"]],
  ["obsidian", ["obsidian"]], ["window", ["window"], 12], ["torch", ["torch"], 14, [0.4375, 0, 0.4375, 0.5625, 0.625, 0.5625]],
  ["nfence", ["nether_brick"], 0, [0.375, 0, 0.375, 0.625, 1, 0.625]], ["pfence", ["planks"], 0, [0.375, 0, 0.375, 0.625, 1, 0.625]],
  ["pillar", ["purpur", "purpur_pillar", "purpur"]], ["dripstone", ["dripstone"]], ["goldd", ["deepslate_gold"]],
];
const N = DEFS.length, TEX = [], EMIT = new Uint8Array(N), SHAPE = [], OPAQUE = new Uint8Array(N);
DEFS.forEach(([name, faces, emit, shape], id) => {
  B[name] = id;
  if (!faces) return;
  const [t, s = t, b = t] = faces;
  TEX[id] = [TI[t], TI[s], TI[b]];
  EMIT[id] = emit || 0;
  SHAPE[id] = shape || null;
  OPAQUE[id] = shape || name === "water" ? 0 : 1;
});
const BLOCKS_LIGHT = (id) => OPAQUE[id] && !EMIT[id] && id !== B.leaves;

// ---------- noise ----------
function h3(x, y, z, s) {
  let h = Math.imul(x | 0, 374761393) ^ Math.imul(y | 0, 668265263) ^ Math.imul(z | 0, 1440662683) ^ Math.imul(s | 0, 1274126177);
  h = Math.imul(h ^ (h >>> 13), 1274126177);
  h ^= h >>> 16;
  return (h >>> 0) / 4294967296;
}
const sm = (t) => t * t * (3 - 2 * t);
function n2(x, z, s) {
  const xi = Math.floor(x), zi = Math.floor(z), u = sm(x - xi), v = sm(z - zi);
  const a = h3(xi, 0, zi, s), b = h3(xi + 1, 0, zi, s), c = h3(xi, 0, zi + 1, s), d = h3(xi + 1, 0, zi + 1, s);
  return a + (b - a) * u + (c - a) * v + (a - b - c + d) * u * v;
}
function n3(x, y, z, s) {
  const xi = Math.floor(x), yi = Math.floor(y), zi = Math.floor(z), u = sm(x - xi), v = sm(y - yi), w = sm(z - zi);
  const c000 = h3(xi, yi, zi, s), c100 = h3(xi + 1, yi, zi, s), c010 = h3(xi, yi + 1, zi, s), c110 = h3(xi + 1, yi + 1, zi, s);
  const c001 = h3(xi, yi, zi + 1, s), c101 = h3(xi + 1, yi, zi + 1, s), c011 = h3(xi, yi + 1, zi + 1, s), c111 = h3(xi + 1, yi + 1, zi + 1, s);
  const a0 = c000 + (c100 - c000) * u, a1 = c010 + (c110 - c010) * u, b0 = c001 + (c101 - c001) * u, b1 = c011 + (c111 - c011) * u;
  const p = a0 + (a1 - a0) * v, q = b0 + (b1 - b0) * v;
  return p + (q - p) * w;
}
function fbm2(x, z, s, o = 4) { let t = 0, a = 0.5, f = 1, n = 0; for (let i = 0; i < o; i++) { t += a * n2(x * f, z * f, s + i * 17); n += a; a *= 0.5; f *= 2; } return t / n; }
function fbm3(x, y, z, s, o = 3) { let t = 0, a = 0.5, f = 1, n = 0; for (let i = 0; i < o; i++) { t += a * n3(x * f, y * f, z * f, s + i * 31); n += a; a *= 0.5; f *= 2; } return t / n; }
const smooth = (e0, e1, x) => sm(Math.min(1, Math.max(0, (x - e0) / (e1 - e0))));

function World(sx, sy, sz) {
  const b = new Uint8Array(sx * sy * sz);
  const inb = (x, y, z) => x >= 0 && y >= 0 && z >= 0 && x < sx && y < sy && z < sz;
  const I = (x, y, z) => (y * sz + z) * sx + x;
  return {
    sx, sy, sz, b, I, inb,
    get: (x, y, z) => (inb(x, y, z) ? b[I(x, y, z)] : 0),
    set: (x, y, z, v) => { if (inb(x, y, z)) b[I(x, y, z)] = v; },
    box(x0, y0, z0, x1, y1, z1, v) { for (let y = y0; y <= y1; y++) for (let z = z0; z <= z1; z++) for (let x = x0; x <= x1; x++) this.set(x, y, z, v); },
  };
}

function tree(w, x, y, z, s) {
  const h = 4 + ((h3(x, 1, z, s) * 3) | 0);
  for (let k = 0; k < h; k++) w.set(x, y + k, z, B.log);
  for (let dy = h - 3; dy <= h + 1; dy++) {
    const r = dy >= h ? 1 : 2;
    for (let dx = -r; dx <= r; dx++) for (let dz = -r; dz <= r; dz++) {
      if (Math.abs(dx) === r && Math.abs(dz) === r && h3(x + dx, dy, z + dz, s) < 0.6) continue;
      if (!w.get(x + dx, y + dy, z + dz)) w.set(x + dx, y + dy, z + dz, B.leaves);
    }
  }
}

// ---------- zones ----------
// Each returns the world, how to light it, and the walkway: ground[x] is the y a player stands at.
const ZONES = {
  summit(seed) {
    const w = World(210, 100, 170), PZ = 40, sea = 26;
    const ridge = (x) => Math.round(52 + 4 * Math.sin(x / 19) + 2 * Math.sin(x / 7.3));
    const H = new Int16Array(w.sx * w.sz);
    for (let z = 0; z < w.sz; z++) for (let x = 0; x < w.sx; x++) {
      const dz = z - PZ, ad = Math.abs(dz);
      let h = 14 + 22 * fbm2(x / 60, z / 60, seed) + 40 * Math.exp(-((dz / 13) ** 2)) * (0.8 + 0.4 * fbm2(x / 20, z / 20, seed + 7))
        + 52 * smooth(95, 160, z) * (0.5 + 0.8 * fbm2(x / 30, z / 30, seed + 3, 5));
      if (ad <= 2) h = ridge(x); else if (z < 95) h = Math.min(h, ridge(x) - (ad - 2) * (dz > 0 ? 0.75 : 1));
      H[z * w.sx + x] = Math.min(w.sy - 8, Math.round(h));
    }
    const hAt = (x, z) => H[Math.min(w.sz - 1, Math.max(0, z)) * w.sx + Math.min(w.sx - 1, Math.max(0, x))];
    for (let z = 0; z < w.sz; z++) for (let x = 0; x < w.sx; x++) {
      const h = hAt(x, z);
      const steep = Math.max(Math.abs(h - hAt(x + 1, z)), Math.abs(h - hAt(x - 1, z)), Math.abs(h - hAt(x, z + 1)), Math.abs(h - hAt(x, z - 1))) > 2;
      const snowy = h > 68 - 4 * n2(x / 9, z / 9, seed + 9);
      for (let y = 0; y <= h; y++) {
        const d = h - y;
        let v = B.stone;
        if (d === 0) v = h <= sea + 1 ? B.sand : snowy ? (steep ? B.snow : B.snowy) : steep ? B.stone : B.grass;
        else if (d < 4 && !steep) v = h <= sea + 1 ? B.sand : B.dirt;
        else if (h3(x, y, z, seed) < 0.01) v = B.coal;
        w.set(x, y, z, v);
      }
      for (let y = h + 1; y <= sea; y++) w.set(x, y, z, B.water);
    }
    for (let cz = 0; cz < w.sz / 5; cz++) for (let cx = 0; cx < w.sx / 5; cx++) {
      if (h3(cx, 7, cz, seed) > 0.45) continue;
      const x = cx * 5 + ((h3(cx, 8, cz, seed) * 4) | 0), z = cz * 5 + ((h3(cx, 9, cz, seed) * 4) | 0), h = hAt(x, z);
      if (w.get(x, h, z) === B.grass && h < 62 && Math.abs(z - PZ) > 5) tree(w, x, h + 1, z, seed);
    }
    // The hole the explorer digs (main.js covers it with a breakable block).
    for (let y = ridge(58) - 7; y <= ridge(58); y++) w.set(58, y, PZ, 0);
    const ground = Array.from({ length: w.sx }, (_, x) => ridge(x) + 1);
    return { w, PZ, ground, sky: true, ambient: [0.3, 0.32, 0.4], skyCol: [0.78, 0.76, 0.72], blockCol: [1, 0.85, 0.6], actor: [1, 1, 1] };
  },

  mineshaft(seed) {
    const w = World(190, 40, 58), PZ = 29, floor = 16, top = 20;
    for (let y = 0; y < w.sy; y++) for (let z = 0; z < w.sz; z++) for (let x = 0; x < w.sx; x++) {
      let v = y < 10 ? B.deepslate : B.stone;
      const n = fbm3(x / 9, y / 6, z / 9, seed + 2);
      if (n > 0.7) v = B.gravel; else if (n < 0.24) v = B.dirt;
      const r = h3(x, y, z, seed);
      if (r < 0.014) v = B.coal; else if (r < 0.022) v = B.iron; else if (r < 0.026) v = y < 10 ? B.diamond : B.gold; else if (r < 0.0275) v = B.emerald;
      if (Math.abs(z - PZ) > 6 && fbm3(x / 16, y / 10, z / 16, seed + 5) > 0.67) v = y < 7 ? B.lava : 0;
      w.set(x, y, z, v);
    }
    w.box(0, floor + 1, PZ - 2, w.sx - 1, top, PZ + 2, 0);
    for (let x = 0; x < w.sx; x++) for (let z = PZ - 2; z <= PZ + 2; z++) w.set(x, floor, z, h3(x, 3, z, seed) < 0.5 ? B.planks : B.cobble);
    for (let x = 3; x < w.sx; x += 6) {
      for (let y = floor + 1; y < top; y++) { w.set(x, y, PZ - 2, B.pfence); w.set(x, y, PZ + 2, B.pfence); }
      for (let z = PZ - 2; z <= PZ + 2; z++) w.set(x, top, z, B.planks);
      if (x % 12 === 3) { w.set(x + 1, floor + 1, PZ + 2, B.torch); w.set(x + 7, floor + 1, PZ - 2, B.torch); }
    }
    for (let x0 = 12; x0 < w.sx - 4; x0 += 16) {
      for (const dir of [1, -1]) {
        const zs = dir > 0 ? PZ + 3 : 3, ze = dir > 0 ? w.sz - 4 : PZ - 3;
        w.box(x0 - 1, floor + 1, zs, x0 + 1, floor + 3, ze, 0);
        for (let z = zs; z <= ze; z++) w.set(x0, floor, z, B.planks);
        for (let z = zs + 1; z <= ze; z += 4) {
          for (let y = floor + 1; y <= floor + 2; y++) { w.set(x0 - 1, y, z, B.pfence); w.set(x0 + 1, y, z, B.pfence); }
          w.box(x0 - 1, floor + 3, z, x0 + 1, floor + 3, z, B.planks);
          if (z % 8 === 1) w.set(x0, floor + 1, z + 1, B.torch);
        }
      }
    }
    for (let y = 1; y <= floor; y++) w.set(76, y, PZ, 0); // a hole down to the caves
    return { w, PZ, ground: Array(w.sx).fill(floor + 1), sky: false, ambient: [0.13, 0.13, 0.17], blockCol: [1.2, 0.98, 0.74], actor: [0.95, 0.86, 0.76] };
  },

  caves(seed) {
    const w = World(200, 72, 112), PZ = 24, floor = 28;
    for (let y = 0; y < w.sy; y++) for (let z = 0; z < w.sz; z++) for (let x = 0; x < w.sx; x++) {
      const shape = 0.38 * (1 - ((y - 38) / 28) ** 2) + (z > PZ ? 0.08 : -0.1);
      const open = fbm3(x / 24, y / 14, z / 24, seed) + shape > 0.7;
      let v = 0;
      if (!open) {
        v = y < 30 ? B.deepslate : B.stone;
        const r = h3(x, y, z, seed);
        if (r < 0.007) v = y < 30 ? B.diamond : B.iron; else if (r < 0.013) v = y < 30 ? B.goldd : B.coal;
      } else if (y < 15) v = B.lava;
      w.set(x, y, z, v);
    }
    // The walkway, and room for the camera beside it.
    w.box(0, floor + 1, PZ - 11, w.sx - 1, floor + 11, PZ + 3, 0);
    for (let x = 0; x < w.sx; x++) for (let z = PZ - 3; z <= PZ + 3; z++) for (let y = floor - 3; y <= floor; y++) {
      const id = w.get(x, y, z);
      if (!id || id === B.lava) w.set(x, y, z, y === floor ? B.stone : B.deepslate);
    }
    for (let k = 0; k < 1100; k++) {
      const x = (h3(k, 1, 2, seed) * w.sx) | 0, z = (h3(k, 3, 4, seed) * w.sz) | 0;
      if (Math.abs(z - PZ) < 4) continue;
      const up = k % 2 === 0, len = 2 + ((h3(k, 5, 6, seed) * 9) | 0);
      let y = up ? w.sy - 2 : 16;
      if (up) { while (y > 18 && !(w.get(x, y, z) && !w.get(x, y - 1, z))) y--; }
      else { while (y < w.sy - 2 && !(w.get(x, y, z) && !w.get(x, y + 1, z))) y++; }
      if (y <= 18 || y >= w.sy - 2) continue;
      for (let i = 1; i <= len; i++) {
        const yy = up ? y - i : y + i;
        if (w.get(x, yy, z)) break;
        w.set(x, yy, z, B.dripstone);
        if (i < len / 3) for (const [dx, dz] of [[1, 0], [-1, 0], [0, 1], [0, -1]]) if (!w.get(x + dx, yy, z + dz)) w.set(x + dx, yy, z + dz, B.dripstone);
      }
    }
    for (let i = 1; i < w.b.length - 1; i++) if (OPAQUE[w.b[i]] && h3(i, 9, 9, seed) < 0.005 && (w.b[i + 1] === 0 || w.b[i - 1] === 0)) w.b[i] = B.amethyst;
    for (let x = 6; x < w.sx; x += 9) { w.set(x, floor + 1, PZ + 3, B.torch); w.set(x + 4, floor + 1, PZ - 3, B.torch); }
    return { w, PZ, ground: Array(w.sx).fill(floor + 1), sky: false, ambient: [0.24, 0.25, 0.33], blockCol: [1.2, 0.95, 0.74], actor: [0.9, 0.86, 0.9] };
  },

  nether(seed) {
    const w = World(200, 92, 150), PZ = 24, sea = 32, shore = 36, deck = 44;
    for (let z = 0; z < w.sz; z++) for (let x = 0; x < w.sx; x++) {
      const floorH = 18 + 22 * fbm2(x / 34, z / 34, seed) - 4;
      const ceil = 84 + 6 * fbm2(x / 30, z / 30, seed + 4) - 16 * Math.max(0, n2(x / 4, z / 4, seed + 6) - 0.55) * 4;
      for (let y = 0; y < w.sy; y++) {
        let solid = y < floorH || y > ceil;
        if (!solid && z > PZ + 5 && fbm3(x / 20, y / 30, z / 20, seed + 2) > 0.68) solid = true;
        let v = 0;
        if (solid) {
          v = B.netherrack;
          const r = h3(x, y, z, seed);
          if (r < 0.012) v = B.quartz;
          if (y >= floorH - 1 && y > sea + 1 && n2(x / 7, z / 7, seed + 8) > 0.66) v = B.soulsand;
          if (y < floorH && y > sea - 3 && y < sea + 2 && r > 0.8) v = B.magma;
        } else if (y <= sea) v = B.lava;
        w.set(x, y, z, v);
      }
    }
    for (let k = 0; k < 170; k++) {
      const x = (h3(k, 1, 1, seed) * w.sx) | 0, z = (h3(k, 2, 1, seed) * w.sz) | 0;
      let y = w.sy - 2; while (y > sea + 8 && !(w.get(x, y, z) && !w.get(x, y - 1, z))) y--;
      if (y <= sea + 8) continue;
      for (let i = 0; i < 4 + ((h3(k, 3, 1, seed) * 10) | 0); i++) {
        const dx = ((h3(k, i, 4, seed) - 0.5) * 4) | 0, dz = ((h3(k, i, 5, seed) - 0.5) * 4) | 0, dy = 1 + ((h3(k, i, 6, seed) * 3) | 0);
        if (!w.get(x + dx, y - dy, z + dz)) w.set(x + dx, y - dy, z + dz, B.glowstone);
      }
    }
    for (let k = 0; k < 10; k++) {
      const x = 10 + ((h3(k, 9, 2, seed) * (w.sx - 20)) | 0), z = PZ + 14 + ((h3(k, 9, 3, seed) * (w.sz - PZ - 24)) | 0);
      let y = w.sy - 2; while (y > sea && !(w.get(x, y, z) && !w.get(x, y - 1, z))) y--;
      for (y = y - 1; y > sea && !w.get(x, y, z); y--) w.set(x, y, z, B.lava);
    }
    // One continuous world: a netherrack cliff (x < BRIDGE) that leads straight onto a fortress bridge.
    const BRIDGE = 58, xb = 84;
    w.box(0, deck + 1, PZ - 13, w.sx - 1, deck + 12, PZ + 4, 0);
    for (let x = 0; x < BRIDGE; x++) for (let z = PZ - 13; z <= PZ + 3; z++) for (let y = sea - 2; y <= deck; y++) {
      const edge = x > BRIDGE - 4 && (z < PZ - 3 || z > PZ + 3);
      if (!edge || y < deck - (x - (BRIDGE - 4)) * 3) w.set(x, y, z, h3(x, y, z, seed) < 0.03 ? B.quartz : B.netherrack);
    }
    for (let x = BRIDGE - 2; x < w.sx; x++) {
      w.box(x, deck - 1, PZ - 3, x, deck, PZ + 3, B.nbrick);
      if (x >= BRIDGE) { w.set(x, deck + 1, PZ - 3, B.nfence); w.set(x, deck + 1, PZ + 3, B.nfence); }
    }
    for (let x = BRIDGE + 4; x < w.sx; x += 13) for (let y = deck - 2; y > sea - 6; y--) w.box(x, y, PZ - 2, x + 2, y, PZ + 2, B.nbrick);
    w.box(xb - 4, deck - 1, PZ + 4, xb + 4, deck + 10, w.sz - 10, 0);
    w.box(xb - 3, deck - 1, PZ + 3, xb + 3, deck, w.sz - 12, B.nbrick);
    for (let z = PZ + 4; z < w.sz - 12; z++) { w.set(xb - 3, deck + 1, z, B.nfence); w.set(xb + 3, deck + 1, z, B.nfence); }
    for (let x = xb - 2; x <= xb + 2; x++) w.set(x, deck + 1, PZ + 3, 0);
    for (let z = PZ + 14; z < w.sz - 12; z += 13) for (let y = deck - 2; y > sea - 6; y--) w.box(xb - 2, y, z, xb + 2, y, z + 2, B.nbrick);
    for (const [tx, tz] of [[66, PZ + 7], [104, PZ + 7], [xb - 4, 96]]) {
      w.box(tx - 1, deck - 1, tz - 1, tx + 9, deck + 24, tz + 9, 0);
      for (let y = sea - 4; y < deck + 20; y++) for (let i = 0; i < 9; i++) for (const [x, z] of [[tx + i, tz], [tx + i, tz + 8], [tx, tz + i], [tx + 8, tz + i]]) {
        w.set(x, y, z, y > deck && y % 5 === 2 && i % 3 === 1 ? 0 : B.nbrick);
      }
      w.box(tx, deck + 20, tz, tx + 8, deck + 20, tz + 8, B.nbrick);
      for (let i = 0; i < 9; i += 2) { w.set(tx + i, deck + 21, tz, B.nfence); w.set(tx + i, deck + 21, tz + 8, B.nfence); w.set(tx, deck + 21, tz + i, B.nfence); w.set(tx + 8, deck + 21, tz + i, B.nfence); }
    }
    return { w, PZ, ground: Array(w.sx).fill(deck + 1), sky: false, ambient: [0.36, 0.2, 0.15], blockCol: [1.2, 0.8, 0.5], actor: [1, 0.8, 0.7] };
  },

  end(seed) {
    const w = World(210, 112, 210), PZ = 40, top = 44;
    const island = (cx, cy, cz, rx, rz, depth, flat) => {
      for (let z = cz - rz; z <= cz + rz; z++) for (let x = cx - rx; x <= cx + rx; x++) {
        const e = ((x - cx) / rx) ** 2 + ((z - cz) / rz) ** 2;
        if (e >= 1) continue;
        const t = flat ? cy : cy + Math.round(2 * fbm2(x / 12, z / 12, seed));
        const bottom = cy - Math.round((1 - e) ** 0.7 * depth * (0.7 + 0.5 * n2(x / 6, z / 6, seed + 1)));
        for (let y = bottom; y <= t; y++) w.set(x, y, z, B.endstone);
      }
    };
    island(105, top, 72, 104, 62, 34, true);
    for (let z = 10; z < 135; z++) for (let x = 0; x < w.sx; x++) if (Math.abs(z - PZ) > 6 && w.get(x, top, z) === B.endstone && n2(x / 5, z / 5, seed + 3) > 0.8) w.set(x, top + 1, z, B.endstone);
    for (let k = 0; k < 10; k++) {
      const a = (k / 10) * Math.PI * 2, px = Math.round(132 + Math.cos(a) * 40), pz = Math.round(112 + Math.sin(a) * 22);
      const r = 2 + (k % 3), ht = 64 + ((h3(k, 1, 1, seed) * 34) | 0);
      for (let y = top - 2; y <= ht; y++) for (let dz = -r; dz <= r; dz++) for (let dx = -r; dx <= r; dx++) if (dx * dx + dz * dz <= r * r + 1) w.set(px + dx, y, pz + dz, B.obsidian);
    }
    island(30, 52, 186, 26, 20, 18);
    for (let k = 0; k < 8; k++) island(20 + ((h3(k, 4, 4, seed) * 170) | 0), 34 + ((h3(k, 5, 5, seed) * 46) | 0), 140 + ((h3(k, 6, 6, seed) * 60) | 0), 5 + (k % 4) * 3, 4 + (k % 3) * 3, 8);
    const tower = (tx, by, tz, levels) => {
      let s = 11, y = by;
      for (let l = 0; l < levels; l++) {
        const hgt = 7 + (l % 2) * 2, x0 = tx - (s >> 1), z0 = tz - (s >> 1);
        for (let yy = y; yy < y + hgt; yy++) for (let i = 0; i < s; i++) for (const [x, z] of [[x0 + i, z0], [x0 + i, z0 + s - 1], [x0, z0 + i], [x0 + s - 1, z0 + i]]) {
          const corner = (x === x0 || x === x0 + s - 1) && (z === z0 || z === z0 + s - 1);
          w.set(x, yy, z, corner ? B.pillar : yy - y === 3 && i % 3 === 1 && i > 0 && i < s - 1 ? B.window : B.purpur);
        }
        w.box(x0 - 1, y + hgt, z0 - 1, x0 + s, y + hgt, z0 + s, B.purpur);
        y += hgt + 1;
        s = Math.max(5, s - 2);
      }
      w.box(tx, y, tz, tx, y + 3, tz, B.pillar);
      w.set(tx, y + 4, tz, B.window);
    };
    // The End city stands on the main island, where the walk ends.
    tower(76, top + 1, PZ + 16, 6); tower(97, top + 1, PZ + 24, 5); tower(70, top + 1, PZ + 34, 4); tower(32, 53, 188, 3);
    // The obsidian platform you arrive on.
    w.box(25, top, PZ - 2, 31, top, PZ + 2, B.obsidian);
    return { w, PZ, ground: Array(w.sx).fill(top + 1), sky: true, ambient: [0.36, 0.32, 0.46], skyCol: [0.5, 0.44, 0.58], blockCol: [1.1, 0.8, 1.1], actor: [0.9, 0.84, 1] };
  },
};

// ---------- light ----------
function flood(w, L, seedFn) {
  const { sx, sy, sz, b } = w, plane = sx * sz;
  let q = new Int32Array(1 << 20), head = 0, tail = 0;
  const push = (i) => { if (tail >= q.length) { const n = new Int32Array(q.length * 2); n.set(q.subarray(0, tail)); q = n; } q[tail++] = i; };
  seedFn(push);
  while (head < tail) {
    const i = q[head++], l = L[i] - 1;
    if (l <= 0) continue;
    const x = i % sx, z = ((i / sx) | 0) % sz, y = (i / plane) | 0;
    const t = (j) => { if (L[j] < l && !BLOCKS_LIGHT(b[j])) { L[j] = l; push(j); } };
    if (x > 0) t(i - 1); if (x < sx - 1) t(i + 1);
    if (z > 0) t(i - sx); if (z < sz - 1) t(i + sx);
    if (y > 0) t(i - plane); if (y < sy - 1) t(i + plane);
  }
}
function light(zone) {
  const { w } = zone, { sx, sy, sz, b } = w;
  const BL = new Uint8Array(b.length), SL = new Uint8Array(b.length);
  flood(w, BL, (push) => { for (let i = 0; i < b.length; i++) if (EMIT[b[i]]) { BL[i] = EMIT[b[i]]; push(i); } });
  if (zone.sky) flood(w, SL, (push) => {
    for (let z = 0; z < sz; z++) for (let x = 0; x < sx; x++) for (let y = sy - 1; y >= 0; y--) {
      const i = (y * sz + z) * sx + x;
      if (BLOCKS_LIGHT(b[i])) break;
      SL[i] = 15; push(i);
    }
  });
  zone.BL = BL; zone.SL = SL;
}

// ---------- meshing ----------
const CURVE = new Float32Array(16).map((_, l) => (l === 0 ? 0 : Math.pow(0.8, 15 - l)));
const AOF = [0.42, 0.6, 0.8, 1];
const SHADE = [0.8, 0.8, 1, 0.5, 0.62, 0.62]; // +x -x +y -y +z -z
const DIRS = [[1, 0, 0], [-1, 0, 0], [0, 1, 0], [0, -1, 0], [0, 0, 1], [0, 0, -1]];
const CORNERS = [[0, 0], [1, 0], [1, 1], [0, 1]];

function mesh(zone) {
  const { w, BL, SL, ambient, skyCol = [0, 0, 0], blockCol } = zone, { sx, sy, sz, b } = w;
  const CH = 32, chunks = [];
  const opaqueAt = (x, y, z) => (x < 0 || y < 0 || z < 0 || x >= sx || y >= sy || z >= sz ? 0 : OPAQUE[b[(y * sz + z) * sx + x]]);
  const lightAt = (x, y, z) => {
    if (x < 0 || y < 0 || z < 0 || x >= sx || y >= sy || z >= sz) return zone.sky ? 15 * 16 : 0;
    const i = (y * sz + z) * sx + x;
    return SL[i] * 16 + BL[i];
  };
  const newChunk = () => ({ pos: [], uv: [], col: [], idx: [], n: 0 });
  const water = newChunk();
  const tileUV = (t, u, v) => [((t % ACOLS) + 0.002 + u * 0.996) / ACOLS, (((t / ACOLS) | 0) + 0.002 + (1 - v) * 0.996) / AROWS];
  const col = (sky, blk, f) => {
    const s = CURVE[sky], k = CURVE[blk];
    return [0, 1, 2].map((c) => Math.min(255, (ambient[c] + skyCol[c] * s + blockCol[c] * k) * f * 255));
  };
  const quad = (ch, verts, uvs, cols, flip) => {
    const base = ch.n;
    for (let i = 0; i < 4; i++) { ch.pos.push(...verts[i]); ch.uv.push(...uvs[i]); ch.col.push(...cols[i]); }
    if (flip) ch.idx.push(base, base + 1, base + 3, base + 1, base + 2, base + 3);
    else ch.idx.push(base, base + 1, base + 2, base, base + 2, base + 3);
    ch.n += 4;
  };

  function box(ch, x, y, z, id) {
    const [x0, y0, z0, x1, y1, z1] = SHAPE[id];
    const l = lightAt(x, y, z), c = EMIT[id] ? [255, 255, 255] : col(l >> 4, l & 15, 1);
    const t = TEX[id][1];
    const P = (a, b2, c2) => [x + a, y + b2, z + c2];
    const faces = [
      [P(x1, y0, z0), P(x1, y1, z0), P(x1, y1, z1), P(x1, y0, z1), 0.8], [P(x0, y0, z1), P(x0, y1, z1), P(x0, y1, z0), P(x0, y0, z0), 0.8],
      [P(x0, y1, z0), P(x0, y1, z1), P(x1, y1, z1), P(x1, y1, z0), 1], [P(x0, y0, z0), P(x1, y0, z0), P(x1, y0, z1), P(x0, y0, z1), 0.5],
      [P(x1, y0, z1), P(x1, y1, z1), P(x0, y1, z1), P(x0, y0, z1), 0.62], [P(x0, y0, z0), P(x0, y1, z0), P(x1, y1, z0), P(x1, y0, z0), 0.62],
    ];
    for (const [a, b2, c2, d, s] of faces) {
      const cc = EMIT[id] ? c : c.map((v) => v * s);
      quad(ch, [a, b2, c2, d], [tileUV(t, x0, y0), tileUV(t, x0, y1), tileUV(t, x1, y1), tileUV(t, x1, y0)], [cc, cc, cc, cc], false);
    }
  }

  for (let x = 0; x < sx; x++) {
    const ci = (x / CH) | 0;
    if (!chunks[ci]) chunks[ci] = newChunk();
    const ch = chunks[ci];
    for (let y = 0; y < sy; y++) for (let z = 0; z < sz; z++) {
      const id = b[(y * sz + z) * sx + x];
      if (!id) continue;
      if (SHAPE[id]) { box(ch, x, y, z, id); continue; }
      const isWater = id === B.water;
      for (let f = 0; f < 6; f++) {
        const [dx, dy, dz] = DIRS[f];
        const nid = w.get(x + dx, y + dy, z + dz);
        if (isWater) { if (nid !== 0) continue; } else if (OPAQUE[nid]) continue;
        const a = f >> 1, sgn = f % 2 === 0 ? 1 : -1, bA = (a + 1) % 3, cA = (a + 2) % 3;
        const tex = TEX[id][a === 1 ? (sgn > 0 ? 0 : 2) : 1];
        const c = [x + dx, y + dy, z + dz];
        const verts = [], uvs = [], cols = [], aos = [];
        const order = sgn > 0 ? CORNERS : CORNERS.slice().reverse();
        for (const [cb, cc] of order) {
          const p = [x, y, z];
          p[a] += sgn > 0 ? 1 : 0; p[bA] += cb; p[cA] += cc;
          if (isWater && a === 1 && sgn > 0) p[1] -= 0.12;
          verts.push(p);
          // u/v chosen so side textures stay upright.
          let u, v;
          if (a === 0) { u = cc; v = cb; } else if (a === 2) { u = cb; v = cc; } else { u = cc; v = cb; }
          uvs.push(tileUV(tex, u, v));
          if (EMIT[id] >= 10) { cols.push([255, 255, 255]); aos.push(3); continue; }
          const sb = cb ? 1 : -1, sc = cc ? 1 : -1;
          const s1 = c.slice(), s2 = c.slice(), cr = c.slice();
          s1[bA] += sb; s2[cA] += sc; cr[bA] += sb; cr[cA] += sc;
          const o1 = opaqueAt(s1[0], s1[1], s1[2]), o2 = opaqueAt(s2[0], s2[1], s2[2]), o3 = opaqueAt(cr[0], cr[1], cr[2]);
          const ao = o1 && o2 ? 0 : 3 - (o1 + o2 + o3);
          let sky = 0, blk = 0, n = 0;
          const take = (q, o) => { if (o) return; const l = lightAt(q[0], q[1], q[2]); sky += l >> 4; blk += l & 15; n++; };
          take(c, 0); take(s1, o1); take(s2, o2); take(cr, o3 || (o1 && o2));
          sky = n ? Math.round(sky / n) : 0; blk = n ? Math.round(blk / n) : 0;
          aos.push(ao);
          cols.push(isWater ? col(sky, blk, 0.9) : col(sky, blk, SHADE[f] * AOF[ao]));
        }
        // Flip the diagonal so corner shading doesn't streak.
        quad(isWater ? water : ch, verts, uvs, cols, aos[0] + aos[2] < aos[1] + aos[3]);
      }
    }
  }
  const pack = (ch) => ({ pos: new Float32Array(ch.pos), uv: new Float32Array(ch.uv), col: new Uint8Array(ch.col), idx: new Uint32Array(ch.idx) });
  return { chunks: chunks.filter((c) => c && c.n).map(pack), water: water.n ? pack(water) : null };
}

self.onmessage = (e) => {
  const { zone: name, seed } = e.data;
  const t0 = Date.now();
  const zone = ZONES[name](seed);
  light(zone);
  const m = mesh(zone);
  const buffers = [];
  for (const c of [...m.chunks, m.water].filter(Boolean)) buffers.push(c.pos.buffer, c.uv.buffer, c.col.buffer, c.idx.buffer);
  self.postMessage({ zone: name, PZ: zone.PZ, ground: zone.ground, actor: zone.actor, size: [zone.w.sx, zone.w.sy, zone.w.sz], mesh: m, ms: Date.now() - t0 }, buffers);
};
