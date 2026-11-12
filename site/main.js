// netherNode site: 3D scroll story. A player walks through block worlds (built by voxel.js)
// while a camera choreographed by the scroll position frames each feature card.
// All art is original: textures come from textures.js and skins are painted below.
import * as THREE from "three";

const $ = (s, r = document) => r.querySelector(s);
const $$ = (s, r = document) => [...r.querySelectorAll(s)];
const clamp = (v, a = 0, b = 1) => Math.min(b, Math.max(a, v));
const ease = (t) => t * t * (3 - 2 * t);
const lerp = (a, b, t) => a + (b - a) * t;
const DEG = Math.PI / 180;
const REDUCE = matchMedia("(prefers-reduced-motion: reduce)").matches;
const REPO = "TechPsychotic/netherNode";
const SEED = 11;

THREE.ColorManagement.enabled = false;

// ---------- renderer ----------
const canvas = $("#world");
let renderer;
try {
  renderer = new THREE.WebGLRenderer({ canvas, antialias: false, powerPreference: "high-performance" });
} catch {
  document.body.classList.add("no-webgl");
}
if (renderer) {
  renderer.outputColorSpace = THREE.LinearSRGBColorSpace;
  renderer.setPixelRatio(Math.min(devicePixelRatio || 1, innerWidth < 700 ? 1.25 : 1.5));
}
const camera = new THREE.PerspectiveCamera(62, innerWidth / innerHeight, 0.1, 500);

const nearest = (tex) => {
  tex.magFilter = THREE.NearestFilter; tex.minFilter = THREE.NearestFilter;
  tex.generateMipmaps = false; tex.flipY = false; tex.colorSpace = THREE.NoColorSpace;
  return tex;
};

// Texture atlas, laid out exactly as voxel.js expects (8 columns, same order).
const ACOLS = 8, NAMES = NNTextures.names, AROWS = Math.ceil(NAMES.length / ACOLS);
const atlasCanvas = document.createElement("canvas");
atlasCanvas.width = ACOLS * 16; atlasCanvas.height = AROWS * 16;
NAMES.forEach((n, i) => atlasCanvas.getContext("2d").drawImage(NNTextures.canvas(n), (i % ACOLS) * 16, ((i / ACOLS) | 0) * 16));
const atlas = nearest(new THREE.CanvasTexture(atlasCanvas));
const worldMat = new THREE.MeshBasicMaterial({ map: atlas, vertexColors: true });
const waterMat = new THREE.MeshBasicMaterial({ map: atlas, vertexColors: true, transparent: true, opacity: 0.75, depthWrite: false });
const texCache = new Map();
const blockTex = (name) => {
  if (!texCache.has(name)) texCache.set(name, nearest(new THREE.CanvasTexture(NNTextures.canvas(name))));
  return texCache.get(name);
};

// ---------- zones ----------
const ZONE_LOOK = {
  summit: { fog: 0xc9c2e6, near: 70, far: 230 },
  mineshaft: { fog: 0x0b0a10, near: 8, far: 56 },
  caves: { fog: 0x10111c, near: 18, far: 96 },
  nether: { fog: 0x6e1c12, near: 22, far: 118 },
  end: { fog: 0x1d102e, near: 60, far: 240 },
};
const zones = {};
for (const [name, look] of Object.entries(ZONE_LOOK)) {
  const scene = new THREE.Scene();
  scene.background = new THREE.Color(look.fog);
  scene.fog = new THREE.Fog(look.fog, look.near, look.far);
  zones[name] = { name, scene, ready: false, loading: false, data: null, group: null, actor: new THREE.Color(1, 1, 1) };
}

// Summit sky: dawn turning to day as you scroll.
const skyUniforms = { uDay: { value: 0 } };
const sky = new THREE.Mesh(new THREE.SphereGeometry(320, 32, 16), new THREE.ShaderMaterial({
  side: THREE.BackSide, depthWrite: false, fog: false, uniforms: skyUniforms,
  vertexShader: "varying vec3 vP; void main(){ vP = normalize(position); gl_Position = projectionMatrix * modelViewMatrix * vec4(position,1.0); }",
  fragmentShader: `uniform float uDay; varying vec3 vP;
    void main(){
      float h = clamp(vP.y * 1.4 + 0.1, 0.0, 1.0);
      vec3 dawnTop = vec3(0.16,0.11,0.36), dawnHor = vec3(1.0,0.72,0.58);
      vec3 dayTop = vec3(0.32,0.5,0.9), dayHor = vec3(0.78,0.84,0.98);
      vec3 c = mix(mix(dawnHor, dawnTop, h), mix(dayHor, dayTop, h), uDay);
      vec3 sunDir = normalize(vec3(0.5, mix(0.06, 0.45, uDay), 0.85));
      float s = dot(normalize(vP), sunDir);
      c += vec3(1.0,0.86,0.6) * (smoothstep(0.995, 0.997, s) * 1.2 + pow(max(s,0.0), 24.0) * 0.35);
      gl_FragColor = vec4(c, 1.0);
    }`,
}));
zones.summit.scene.add(sky);
const summitHorizon = (day) => new THREE.Color().setRGB(lerp(0.86, 0.78, day), lerp(0.74, 0.84, day), lerp(0.74, 0.98, day));

// Blocky clouds.
{
  const c = document.createElement("canvas");
  c.width = c.height = 64;
  const g = c.getContext("2d");
  let s = 7;
  const r = () => ((s = (s * 16807) % 2147483647) / 2147483647);
  g.fillStyle = "#fff";
  for (let i = 0; i < 26; i++) g.fillRect(r() * 64 | 0, r() * 64 | 0, 2 + (r() * 8 | 0), 1 + (r() * 5 | 0));
  const tex = nearest(new THREE.CanvasTexture(c));
  tex.wrapS = tex.wrapT = THREE.RepeatWrapping;
  tex.repeat.set(5, 5);
  const clouds = new THREE.Mesh(new THREE.PlaneGeometry(900, 900), new THREE.MeshBasicMaterial({ map: tex, transparent: true, opacity: 0.85, depthWrite: false, side: THREE.DoubleSide }));
  clouds.rotation.x = -Math.PI / 2;
  clouds.position.set(105, 88, 80);
  clouds.userData.tick = (dt) => { tex.offset.x += dt * 0.0016; };
  zones.summit.scene.add(clouds);
  zones.summit.clouds = clouds;
}
// Stars over the End.
{
  const n = 900, pos = new Float32Array(n * 3);
  for (let i = 0; i < n; i++) {
    const u = Math.random() * 2 - 1, a = Math.random() * Math.PI * 2, rr = Math.sqrt(1 - u * u);
    pos.set([Math.cos(a) * rr * 280 + 105, Math.abs(u) * 200 - 30, Math.sin(a) * rr * 280 + 100], i * 3);
  }
  const geo = new THREE.BufferGeometry();
  geo.setAttribute("position", new THREE.BufferAttribute(pos, 3));
  zones.end.scene.add(new THREE.Points(geo, new THREE.PointsMaterial({ color: 0xe8d8ff, size: 1.6, sizeAttenuation: false, fog: false })));
}

// ---------- world building in workers ----------
const pool = (() => {
  const workers = [];
  const n = Math.max(1, Math.min(2, (navigator.hardwareConcurrency || 2) - 1));
  for (let i = 0; i < n; i++) {
    try {
      const w = new Worker("voxel.js");
      w.busy = false;
      w.onmessage = (e) => { w.busy = false; built(e.data); next(); };
      w.onerror = () => { w.busy = false; w.dead = true; next(); };
      workers.push(w);
    } catch { /* no workers: the page still works without 3D worlds */ }
  }
  let queue = [];
  function next() {
    for (const w of workers) {
      if (w.busy || w.dead || !queue.length) continue;
      const z = queue.shift();
      zones[z].loading = true;
      w.busy = true;
      w.postMessage({ zone: z, seed: SEED });
    }
  }
  return {
    want(list) {
      queue = list.filter((z) => !zones[z].ready && !zones[z].loading);
      next();
    },
  };
})();

function built(d) {
  const z = zones[d.zone];
  const group = new THREE.Group();
  const add = (c, mat) => {
    const g = new THREE.BufferGeometry();
    g.setAttribute("position", new THREE.BufferAttribute(c.pos, 3));
    g.setAttribute("uv", new THREE.BufferAttribute(c.uv, 2));
    g.setAttribute("color", new THREE.BufferAttribute(c.col, 3, true));
    g.setIndex(new THREE.BufferAttribute(c.idx, 1));
    g.computeBoundingSphere();
    const m = new THREE.Mesh(g, mat);
    m.matrixAutoUpdate = false;
    group.add(m);
  };
  d.mesh.chunks.forEach((c) => add(c, worldMat));
  if (d.mesh.water) add(d.mesh.water, waterMat);
  z.scene.add(group);
  z.group = group; z.data = d; z.ready = true; z.loading = false;
  z.actor.setRGB(...d.actor);
  if (z === activeZone) $("#loading").hidden = true;
  onZoneReady(z);
}

// ---------- skins (standard 64×64 player layout) ----------
// Box UV rectangles for a w×h×d part whose texture starts at (u, v).
function boxRects(u, v, w, h, d) {
  return { top: [u + d, v, w, d], bottom: [u + d + w, v, w, d], right: [u, v + d, d, h], front: [u + d, v + d, w, h], left: [u + d + w, v + d, d, h], back: [u + 2 * d + w, v + d, w, h] };
}
const PARTS = {
  head: { size: [8, 8, 8], uv: [0, 0], over: [32, 0] },
  body: { size: [8, 12, 4], uv: [16, 16], over: [16, 32] },
  armR: { size: [4, 12, 4], uv: [40, 16], over: [40, 32] },
  armL: { size: [4, 12, 4], uv: [32, 48], over: [48, 48], legacy: [40, 16] },
  legR: { size: [4, 12, 4], uv: [0, 16], over: [0, 32] },
  legL: { size: [4, 12, 4], uv: [16, 48], over: [0, 48], legacy: [0, 16] },
};

function paintSkin(p) {
  const c = document.createElement("canvas");
  c.width = c.height = 64;
  const g = c.getContext("2d");
  let s = p.seed || 3;
  const r = () => ((s = (s * 16807) % 2147483647) / 2147483647);
  const shade = (hex, f) => { const n = parseInt(hex.slice(1), 16); return `rgb(${(n >> 16) * f | 0},${((n >> 8) & 255) * f | 0},${(n & 255) * f | 0})`; };
  const fillNoise = (x, y, w, h, hex, amt = 0.08) => { for (let j = 0; j < h; j++) for (let i = 0; i < w; i++) { g.fillStyle = shade(hex, 1 - amt + r() * amt * 2); g.fillRect(x + i, y + j, 1, 1); } };
  const px = (x, y, col) => { g.fillStyle = col; g.fillRect(x, y, 1, 1); };
  const part = (key, painter) => {
    const P = PARTS[key], [w, h, d] = P.size, rects = boxRects(P.uv[0], P.uv[1], w, h, d);
    for (const [face, [x, y, fw, fh]] of Object.entries(rects)) painter(face, x, y, fw, fh);
  };
  // Head: a hood around the face.
  part("head", (face, x, y, w, h) => {
    fillNoise(x, y, w, h, p.hood);
    if (face !== "front") return;
    fillNoise(x + 1, y + 1, 6, 7, p.skin, 0.04);
    for (let i = 1; i < 7; i++) px(x + i, y + 1, p.hair);
    px(x + 1, y + 2, p.hair); px(x + 6, y + 2, p.hair); px(x + 3, y + 2, p.hair);
    px(x + 1, y + 4, "#ffffff"); px(x + 2, y + 4, p.eye); px(x + 5, y + 4, p.eye); px(x + 6, y + 4, "#ffffff");
    px(x + 3, y + 5, shade(p.skin, 0.86)); px(x + 4, y + 5, shade(p.skin, 0.86));
    px(x + 3, y + 6, p.mouth); px(x + 4, y + 6, p.mouth);
    for (let i = 0; i < 8; i++) px(x + i, y + 7, shade(p.hood, 0.8));
    px(x + 1, y + 7, p.skin); px(x + 6, y + 7, p.skin);
    for (let i = 2; i < 6; i++) px(x + i, y + 7, shade(p.skin, 0.9));
  });
  part("body", (face, x, y, w, h) => {
    fillNoise(x, y, w, h, p.hood);
    if (face === "front") {
      px(x + 3, y, p.cord); px(x + 3, y + 1, p.cord); px(x + 3, y + 2, p.cord); px(x + 4, y, p.cord); px(x + 4, y + 1, p.cord); px(x + 4, y + 3, p.cord);
      fillNoise(x + 1, y + 7, 6, 3, shade(p.hood, 0.78).replace(/rgb\((\d+),(\d+),(\d+)\)/, (_, a, b, cc) => "#" + [a, b, cc].map((v) => (+v).toString(16).padStart(2, "0")).join("")), 0.05);
      for (let i = 0; i < 8; i++) px(x + i, y + 11, shade(p.hood, 0.7));
    }
    if (face === "back") for (let i = 1; i < 7; i++) { px(x + i, y, shade(p.hood, 0.75)); px(x + i, y + 1, shade(p.hood, 0.85)); }
  });
  for (const arm of ["armR", "armL"]) part(arm, (face, x, y, w, h) => {
    if (face === "top") return fillNoise(x, y, w, h, p.hood);
    if (face === "bottom") return fillNoise(x, y, w, h, p.skin, 0.04);
    fillNoise(x, y, w, 9, p.hood);
    for (let i = 0; i < w; i++) px(x + i, y + 8, shade(p.hood, 0.72));
    fillNoise(x, y + 9, w, 3, p.skin, 0.04);
  });
  for (const leg of ["legR", "legL"]) part(leg, (face, x, y, w, h) => {
    if (face === "top") return fillNoise(x, y, w, h, p.pants);
    if (face === "bottom") return fillNoise(x, y, w, h, p.shoes);
    fillNoise(x, y, w, 9, p.pants);
    fillNoise(x, y + 9, w, 3, p.shoes, 0.05);
    if (face === "front") px(x + 1, y + 9, shade(p.shoes, 2.4));
  });
  return c;
}
const PALETTES = {
  hero: { hood: "#6a3fe0", hair: "#3b2516", skin: "#e0a97f", eye: "#2a1b54", mouth: "#a8664a", pants: "#2e2b3d", shoes: "#141019", cord: "#efe7ff", seed: 3 },
  kai: { hood: "#1fa79a", hair: "#1a1310", skin: "#8d5a3b", eye: "#120b08", mouth: "#5e3420", pants: "#3a3a46", shoes: "#18181e", cord: "#e6fffb", seed: 5 },
  juno: { hood: "#e0603f", hair: "#d8a542", skin: "#f0c39b", eye: "#2b4a1a", mouth: "#c0785a", pants: "#3b3350", shoes: "#1b1626", cord: "#fff1e6", seed: 9 },
  pip: { hood: "#d64d8f", hair: "#2a1a3a", skin: "#c99572", eye: "#1a0f22", mouth: "#8f553c", pants: "#26324a", shoes: "#10141e", cord: "#ffe6f2", seed: 13 },
};
// Monsters reuse the player layout with their own looks.
function paintMob(kind) {
  const c = document.createElement("canvas");
  c.width = c.height = 64;
  const g = c.getContext("2d");
  let s = kind.length * 7;
  const r = () => ((s = (s * 16807) % 2147483647) / 2147483647);
  const noise = (x, y, w, h, base, amt = 24) => { for (let j = 0; j < h; j++) for (let i = 0; i < w; i++) { const d = (r() - 0.5) * amt; g.fillStyle = `rgb(${base[0] + d | 0},${base[1] + d | 0},${base[2] + d | 0})`; g.fillRect(x + i, y + j, 1, 1); } };
  const px = (x, y, col) => { g.fillStyle = col; g.fillRect(x, y, 1, 1); };
  const skin = kind === "zombie" ? [82, 140, 66] : [214, 210, 196];
  for (const [key, P] of Object.entries(PARTS)) {
    const [w, h, d] = P.size;
    for (const [face, [x, y, fw, fh]] of Object.entries(boxRects(P.uv[0], P.uv[1], w, h, d))) {
      if (kind === "zombie") {
        if (key === "body") noise(x, y, fw, fh, [92, 80, 54]);
        else if (key.startsWith("leg")) noise(x, y, fw, fh, [52, 56, 86]);
        else if (key.startsWith("arm") && face !== "bottom") { noise(x, y, fw, fh, skin); noise(x, y, fw, 3, [92, 80, 54]); }
        else noise(x, y, fw, fh, skin);
        if (key === "head" && face === "front") { px(x + 1, y + 3, "#0f1a0b"); px(x + 2, y + 3, "#0f1a0b"); px(x + 5, y + 3, "#0f1a0b"); px(x + 6, y + 3, "#0f1a0b"); px(x + 2, y + 4, "#d23a2a"); px(x + 5, y + 4, "#d23a2a"); for (let i = 2; i < 6; i++) px(x + i, y + 6, "#24401c"); }
        if (key === "body" && face === "front") for (let i = 0; i < 9; i++) px(x + (r() * 8 | 0), y + 8 + (r() * 4 | 0), "#4c7f3c");
      } else {
        g.clearRect(x, y, fw, fh);
        if (key === "head") noise(x, y, fw, fh, skin, 18);
        else if (key === "body") { for (let j = 0; j < fh; j += 2) noise(x + 1, y + j, fw - 2, 1, skin, 14); noise(x + (fw >> 1) - 1, y, 2, fh, skin, 10); }
        else noise(x + (fw >> 1) - 1, y, 2, fh, skin, 14);
        if (key === "head" && face === "front") { for (const [i, j] of [[1, 3], [2, 3], [5, 3], [6, 3], [1, 4], [2, 4], [5, 4], [6, 4]]) px(x + i, y + j, "#141418"); for (let i = 1; i < 7; i++) px(x + i, y + 6, i % 2 ? "#2a2a2e" : "#bdb9ad"); }
      }
    }
  }
  return c;
}

const S16 = 0.9 / 16; // one skin pixel in world units (a player is 1.8 blocks tall)

function partGeometry(key, slim, legacy, size, uvOrigin, inflate = 0) {
  const P = PARTS[key];
  const [w0, h, d] = size || P.size, w = slim && key.startsWith("arm") ? 3 : w0;
  const geo = new THREE.BoxGeometry((w + inflate) * S16, (h + inflate) * S16, (d + inflate) * S16);
  const origin = uvOrigin || (legacy && P.legacy ? P.legacy : P.uv);
  const R = boxRects(origin[0], origin[1], w, h, d);
  const order = [R.left, R.right, R.top, R.bottom, R.front, R.back]; // BoxGeometry: +x, -x, +y, -y, +z, -z
  const uv = geo.attributes.uv, H = legacy ? 32 : 64;
  order.forEach(([x, y, fw, fh], f) => {
    const u0 = x / 64, u1 = (x + fw) / 64, v0 = y / H, v1 = (y + fh) / H;
    uv.setXY(f * 4, u0, v0); uv.setXY(f * 4 + 1, u1, v0); uv.setXY(f * 4 + 2, u0, v1); uv.setXY(f * 4 + 3, u1, v1);
  });
  return geo;
}

// Pixel items, extruded into 3D like held items in block games.
function item(rows, palette) {
  const pos = [], col = [];
  const cube = (x, y, color) => {
    const c = new THREE.Color(color);
    const g = new THREE.BoxGeometry(1, 1, 1.4);
    g.translate(x, -y, 0);
    const p = g.attributes.position.array, idx = g.index.array;
    for (const i of idx) { pos.push(p[i * 3], p[i * 3 + 1], p[i * 3 + 2]); col.push(c.r, c.g, c.b); }
  };
  rows.forEach((row, y) => [...row].forEach((ch, x) => { if (palette[ch]) cube(x, y, palette[ch]); }));
  const geo = new THREE.BufferGeometry();
  geo.setAttribute("position", new THREE.Float32BufferAttribute(pos, 3));
  geo.setAttribute("color", new THREE.Float32BufferAttribute(col, 3));
  geo.translate(-8, 8, 0);
  geo.scale(S16 * 0.9, S16 * 0.9, S16 * 0.9);
  return new THREE.Mesh(geo, new THREE.MeshBasicMaterial({ vertexColors: true }));
}
const PICK = item([
  "....dddddd......", "...dlllmmmd.....", "..dllddddmmd....", ".dld....hd.md...", ".dd.....hh..d...", "........hh......",
  ".......hh.......", "......hh........", ".....hh.........", "....hh..........", "...hh...........", "..hh............", ".hh.............", "................",
], { d: "#3b1d86", l: "#e4ccff", m: "#9b5cff", h: "#7a5230" });
const SWORD = item([
  ".............dd.", "............dld.", "...........dlmd.", "..........dlmd..", ".........dlmd...", "........dlmd....", ".......dlmd.....",
  "..d...dlmd......", "...d.dlmd.......", "....dlmd........", "....hgd.........", "...hh.dd........", "..hh............", ".gg.............",
], { d: "#3b1d86", l: "#efe4ff", m: "#9b5cff", h: "#7a5230", g: "#5a2fd0" });

function nameTag(text) {
  const c = document.createElement("canvas");
  const g = c.getContext("2d");
  g.font = "28px Minecraft, sans-serif";
  c.width = Math.ceil(g.measureText(text).width) + 20; c.height = 40;
  g.font = "28px Minecraft, sans-serif";
  g.fillStyle = "rgba(0,0,0,.45)"; g.fillRect(0, 0, c.width, c.height);
  g.fillStyle = "#fff"; g.textBaseline = "middle"; g.fillText(text, 10, 21);
  const tex = new THREE.CanvasTexture(c);
  tex.colorSpace = THREE.NoColorSpace;
  const sp = new THREE.Sprite(new THREE.SpriteMaterial({ map: tex, depthWrite: false, fog: false }));
  sp.scale.set((c.width / c.height) * 0.3, 0.3, 1);
  sp.position.y = 2.15;
  return sp;
}

// A player: limbs on pivots so they can swing. Faces +z in its own space.
class Player {
  constructor(skinCanvas, { tag, slim = false, mob = null } = {}) {
    this.root = new THREE.Group();
    this.tex = nearest(new THREE.CanvasTexture(skinCanvas));
    this.mat = new THREE.MeshBasicMaterial({ map: this.tex, alphaTest: 0.5 });
    this.overMat = new THREE.MeshBasicMaterial({ map: this.tex, alphaTest: 0.5, side: THREE.DoubleSide });
    this.parts = {};
    this.mob = mob;
    this.build(skinCanvas, slim);
    if (tag) this.root.add(nameTag(tag));
    this.phase = 0;
    this.tint = new THREE.Color(1, 1, 1);
  }
  build(skinCanvas, slim) {
    const legacy = skinCanvas.height === 32;
    for (const key of Object.keys(this.parts)) this.root.remove(this.parts[key]);
    const thin = this.mob === "skeleton";
    const pivot = (key, at, offset) => {
      const g = new THREE.Group();
      g.position.set(at[0] * S16, at[1] * S16, at[2] * S16);
      const size = thin && key !== "head" && key !== "body" ? [2, 12, 2] : undefined;
      const m = new THREE.Mesh(partGeometry(key, slim, legacy, size), this.mat);
      m.position.set(offset[0] * S16, offset[1] * S16, offset[2] * S16);
      g.add(m);
      const P = PARTS[key];
      if (!legacy && !this.mob) {
        const o = new THREE.Mesh(partGeometry(key, slim, false, null, P.over, key === "head" ? 1 : 0.5), this.overMat);
        o.position.copy(m.position);
        g.add(o);
      } else if (legacy && key === "head") {
        const o = new THREE.Mesh(partGeometry(key, false, true, null, [32, 0], 1), this.overMat);
        o.position.copy(m.position);
        g.add(o);
      }
      this.root.add(g);
      this.parts[key] = g;
      return g;
    };
    const aw = slim ? 3 : 4;
    pivot("legR", [-2, 12, 0], [0, -6, 0]);
    pivot("legL", [2, 12, 0], [0, -6, 0]);
    pivot("body", [0, 18, 0], [0, 0, 0]);
    pivot("head", [0, 24, 0], [0, 4, 0]);
    pivot("armR", [-4 - aw / 2, 22, 0], [0, -4, 0]);
    pivot("armL", [4 + aw / 2, 22, 0], [0, -4, 0]);
    this.tools = { pick: PICK.clone(), sword: SWORD.clone() };
    for (const t of Object.values(this.tools)) {
      t.position.set(0, -10 * S16, 3 * S16);
      t.rotation.set(0, Math.PI / 2, -Math.PI / 4);
      t.visible = false;
      this.parts.armR.add(t);
    }
  }
  setSkin(canvasEl, slim) { this.tex.image = canvasEl; this.tex.needsUpdate = true; this.build(canvasEl, slim); }
  tool(name) { for (const [k, t] of Object.entries(this.tools)) t.visible = k === name; }
  setTint(c) { this.mat.color.copy(c); this.overMat.color.copy(c); }
  // pose: walk/idle/mine/fight/fall; walk animation is driven by distance travelled.
  animate(pose, dist, t) {
    const P = this.parts;
    const swing = pose === "walk" ? Math.sin(dist * 2.4) : 0;
    const amp = pose === "walk" ? 0.75 : 0;
    P.legR.rotation.x = swing * amp; P.legL.rotation.x = -swing * amp;
    P.armL.rotation.x = swing * amp * 0.8;
    P.armL.rotation.z = 0.05;
    P.armR.rotation.z = -0.05;
    P.head.rotation.set(0, 0, 0);
    if (this.mob === "zombie" || this.mob === "skeleton") { P.armR.rotation.x = -Math.PI / 2 + Math.sin(t * 3) * 0.05; P.armL.rotation.x = -Math.PI / 2 - Math.sin(t * 3) * 0.05; return; }
    if (pose === "mine") P.armR.rotation.x = -1.3 + Math.sin(t * 15) * 0.85;
    else if (pose === "fight") P.armR.rotation.x = -1.6 + Math.sin(t * 22) * 1.1;
    else if (pose === "dig") { P.armR.rotation.x = -0.7 + Math.sin(t * 15) * 0.55; P.head.rotation.x = 0.55; return; }
    else if (pose === "fall") { P.armR.rotation.x = P.armL.rotation.x = -2.9; P.legR.rotation.x = 0.3; P.legL.rotation.x = -0.3; }
    else P.armR.rotation.x = -swing * amp * 0.8 + (pose === "idle" ? Math.sin(t * 1.6) * 0.04 : 0);
    if (pose === "idle") P.head.rotation.y = Math.sin(t * 0.5) * 0.35;
  }
}

// ---------- the cast ----------
const hero = new Player(paintSkin(PALETTES.hero));
const buddies = [["kai", "Kai"], ["juno", "Juno"], ["pip", "Pip"]].map(([k, n]) => new Player(paintSkin(PALETTES[k]), { tag: n }));
buddies.forEach((b) => (b.root.visible = false));

function spider() {
  const root = new THREE.Group(), legs = [];
  const tex = (base, eyes) => {
    const c = document.createElement("canvas"); c.width = c.height = 16;
    const g = c.getContext("2d");
    for (let j = 0; j < 16; j++) for (let i = 0; i < 16; i++) { const d = (Math.random() - 0.5) * 18; g.fillStyle = `rgb(${base[0] + d | 0},${base[1] + d | 0},${base[2] + d | 0})`; g.fillRect(i, j, 1, 1); }
    if (eyes) { g.fillStyle = "#ff2b3d"; [[3, 5], [5, 5], [10, 5], [12, 5], [4, 8], [11, 8], [7, 6], [8, 6]].forEach(([x, y]) => g.fillRect(x, y, 2, 1)); }
    return new THREE.MeshBasicMaterial({ map: nearest(new THREE.CanvasTexture(c)) });
  };
  const body = tex([46, 38, 50]), head = tex([58, 48, 64], true);
  const box = (w, h, d, mat) => new THREE.Mesh(new THREE.BoxGeometry(w, h, d), mat);
  const abdomen = box(0.75, 0.55, 0.85, body); abdomen.position.set(0, 0.55, -0.55);
  const thorax = box(0.4, 0.4, 0.4, body); thorax.position.set(0, 0.5, 0);
  const hd = box(0.55, 0.5, 0.5, [body, body, body, body, head, body]); hd.position.set(0, 0.52, 0.42);
  root.add(abdomen, thorax, hd);
  for (let i = 0; i < 8; i++) {
    const side = i % 2 ? 1 : -1, row = (i >> 1) - 1.5;
    const pivot = new THREE.Group();
    pivot.position.set(side * 0.2, 0.5, row * 0.16);
    const leg = box(1, 0.1, 0.1, body); leg.position.x = side * 0.5;
    pivot.add(leg);
    pivot.rotation.z = side * 0.55;
    pivot.rotation.y = row * 0.35 * side;
    legs.push(pivot);
    root.add(pivot);
  }
  root.userData.legs = legs;
  root.userData.mats = [body, head];
  return root;
}
function slime() {
  const root = new THREE.Group();
  const c = document.createElement("canvas"); c.width = c.height = 16;
  const g = c.getContext("2d");
  for (let j = 0; j < 16; j++) for (let i = 0; i < 16; i++) { const d = (Math.random() - 0.5) * 20; g.fillStyle = `rgba(${110 + d | 0},${210 + d | 0},${90 + d | 0},0.72)`; g.fillRect(i, j, 1, 1); }
  const outer = new THREE.Mesh(new THREE.BoxGeometry(1, 1, 1), new THREE.MeshBasicMaterial({ map: nearest(new THREE.CanvasTexture(c)), transparent: true, depthWrite: false }));
  const inner = new THREE.Mesh(new THREE.BoxGeometry(0.6, 0.6, 0.6), new THREE.MeshBasicMaterial({ color: 0x4f9a32 }));
  const eyes = new THREE.Mesh(new THREE.BoxGeometry(0.5, 0.12, 0.05), new THREE.MeshBasicMaterial({ color: 0x1b3d12 }));
  eyes.position.set(0, 0.12, 0.31);
  outer.position.y = inner.position.y = 0.5; eyes.position.y += 0.5;
  root.add(inner, eyes, outer);
  root.userData.mats = [outer.material, inner.material, eyes.material];
  return root;
}
const mobs = [
  { obj: new Player(paintMob("zombie"), { tag: "Missing dependency", mob: "zombie" }) },
  { obj: new Player(paintMob("skeleton"), { tag: "Wrong Java", mob: "skeleton" }) },
  { obj: { root: spider() }, tag: "Filename too long" },
  { obj: { root: slime() }, tag: "OutOfMemoryError" },
];
mobs.forEach((m) => {
  if (m.tag) { const t = nameTag(m.tag); t.position.y = 1.4; m.obj.root.add(t); }
  m.obj.root.visible = false;
  zones.caves.scene.add(m.obj.root);
  m.mats = m.obj.mat ? [m.obj.mat] : m.obj.root.userData.mats;
});

// The dragon: an original blocky flyer circling over the End.
const dragon = (() => {
  const root = new THREE.Group();
  const mat = (base) => {
    const c = document.createElement("canvas"); c.width = c.height = 16;
    const g = c.getContext("2d");
    for (let j = 0; j < 16; j++) for (let i = 0; i < 16; i++) { const d = (Math.random() - 0.5) * 14; g.fillStyle = `rgb(${base[0] + d | 0},${base[1] + d | 0},${base[2] + d | 0})`; g.fillRect(i, j, 1, 1); }
    return new THREE.MeshBasicMaterial({ map: nearest(new THREE.CanvasTexture(c)) });
  };
  const hide = mat([24, 18, 34]), wingM = mat([40, 26, 58]);
  wingM.side = THREE.DoubleSide;
  const box = (w, h, d, m, x, y, z) => { const b = new THREE.Mesh(new THREE.BoxGeometry(w, h, d), m); b.position.set(x, y, z); return b; };
  root.add(box(2.2, 1.6, 6, hide, 0, 0, 0), box(1, 1, 3, hide, 0, 0.3, 4.2), box(1.6, 1.2, 2.2, hide, 0, 0.4, 6.6));
  const eye = new THREE.MeshBasicMaterial({ color: 0xe07bff, fog: false });
  root.add(box(0.3, 0.2, 0.2, eye, 0.55, 0.65, 7.6), box(0.3, 0.2, 0.2, eye, -0.55, 0.65, 7.6));
  for (let i = 0; i < 8; i++) root.add(box(0.9 - i * 0.08, 0.8 - i * 0.07, 1.4, hide, 0, -0.1 * i, -3.6 - i * 1.3));
  for (let i = 0; i < 6; i++) root.add(box(0.2, 0.5, 0.5, hide, 0, 1, -2 + i * 0.9));
  const wings = [1, -1].map((side) => {
    const p = new THREE.Group();
    p.position.set(side * 1.1, 0.5, 0.8);
    p.add(box(7, 0.25, 0.5, hide, side * 3.5, 0, 1.3), box(7, 0.08, 4.5, wingM, side * 3.5, 0, -1));
    root.add(p);
    return { p, side };
  });
  root.userData.wings = wings;
  return root;
})();
dragon.scale.setScalar(1.3);
zones.end.scene.add(dragon);

// Ore blocks to mine, with crack overlays.
const crackTex = (() => {
  const lines = [];
  let s = 99;
  const r = () => ((s = (s * 16807) % 2147483647) / 2147483647);
  for (let i = 0; i < 14; i++) { let x = 8 + (r() - 0.5) * 6, y = 8 + (r() - 0.5) * 6; const p = []; for (let k = 0; k < 7; k++) { p.push([x | 0, y | 0]); x += (r() - 0.5) * 5; y += (r() - 0.5) * 5; } lines.push(p); }
  return [0, 1, 2, 3, 4, 5].map((k) => {
    const c = document.createElement("canvas"); c.width = c.height = 16;
    const g = c.getContext("2d"); g.fillStyle = "rgba(10,6,14,.8)";
    lines.slice(0, k * 3).forEach((p) => p.forEach(([x, y]) => g.fillRect(clamp(x, 0, 15), clamp(y, 0, 15), 1, 1)));
    return nearest(new THREE.CanvasTexture(c));
  });
})();
const ORE_KINDS = ["gold_ore", "amethyst_ore", "diamond_ore", "emerald_ore"];
const ORE_COL = { gold_ore: [0xf5d64a, 0xc99a1e], amethyst_ore: [0xb07cff, 0x6a32d8], diamond_ore: [0x5ee8e0, 0x1fa6a0], emerald_ore: [0x4fe07f, 0x178a45] };
const ores = ORE_KINDS.map((k) => {
  const g = new THREE.Group();
  const mats = [new THREE.MeshBasicMaterial({ map: blockTex(k) }), new THREE.MeshBasicMaterial({ map: blockTex("stone") })];
  const crackMat = new THREE.MeshBasicMaterial({ map: crackTex[0], transparent: true, depthWrite: false });
  const a = new THREE.Mesh(new THREE.BoxGeometry(1, 1, 1), mats[0]); a.position.y = 0.5;
  const b = new THREE.Mesh(new THREE.BoxGeometry(1, 1, 1), mats[1]); b.position.y = 1.5;
  const ca = new THREE.Mesh(new THREE.BoxGeometry(1.01, 1.01, 1.01), crackMat); ca.position.y = 0.5;
  const cb = new THREE.Mesh(new THREE.BoxGeometry(1.01, 1.01, 1.01), crackMat); cb.position.y = 1.5;
  g.add(a, b, ca, cb);
  zones.mineshaft.scene.add(g);
  return { g, mats, crackMat, kind: k, broken: false };
});

// Rails along the mineshaft floor.
const rails = new THREE.Group();
zones.mineshaft.scene.add(rails);

// Portals: an obsidian frame with a swirling surface. One is built in the caves; its twin
// in the Nether is where the player steps out.
function makePortal(scene, startLit) {
  const group = new THREE.Group(), blocks = [];
  const mat = new THREE.MeshBasicMaterial({ map: blockTex("obsidian") });
  const cells = [];
  for (let z = 0; z < 4; z++) cells.push([0, z]);
  for (let y = 1; y < 5; y++) cells.push([y, 0], [y, 3]);
  cells.push([4, 1], [4, 2]);
  for (const [y, z] of cells) {
    const m = new THREE.Mesh(new THREE.BoxGeometry(1, 1, 1), mat);
    m.position.set(0.5, y + 0.5, z + 0.5);
    m.visible = startLit;
    group.add(m);
    blocks.push(m);
  }
  const swirl = new THREE.Mesh(new THREE.PlaneGeometry(2, 3), new THREE.ShaderMaterial({
    transparent: true, depthWrite: false, side: THREE.DoubleSide, blending: THREE.AdditiveBlending,
    uniforms: { uTime: { value: 0 }, uLit: { value: startLit ? 1 : 0 } },
    vertexShader: "varying vec2 vUv; void main(){ vUv = uv; gl_Position = projectionMatrix * modelViewMatrix * vec4(position,1.0); }",
    fragmentShader: `uniform float uTime; uniform float uLit; varying vec2 vUv;
      float h(vec2 p){ return fract(sin(dot(p, vec2(127.1,311.7))) * 43758.5); }
      void main(){
        vec2 q = floor(vUv * vec2(16.0, 24.0)) / vec2(16.0, 24.0);
        vec2 c = q - 0.5; float a = atan(c.y, c.x) + uTime * 1.2; float r = length(c);
        float s = sin(a * 3.0 + r * 18.0 - uTime * 3.0) * 0.5 + 0.5;
        float n = h(q + floor(uTime * 6.0) * 0.01);
        vec3 col = mix(vec3(0.45,0.1,0.85), vec3(0.85,0.45,1.0), s) + n * 0.12;
        gl_FragColor = vec4(col, uLit * 0.85);
      }`,
  }));
  swirl.rotation.y = Math.PI / 2;
  swirl.position.set(0.5, 2.5, 2);
  group.add(swirl);
  scene.add(group);
  return { group, blocks, lit(v, t) { swirl.material.uniforms.uLit.value = v; swirl.material.uniforms.uTime.value = t; } };
}
const cavePortal = makePortal(zones.caves.scene, false);
const netherPortal = makePortal(zones.nether.scene, true);

// The End portal at the end of the fortress bridge: twelve frame blocks with eyes around a
// 3×3 pool of starry void, sunk into the deck.
const endPortal = (() => {
  const group = new THREE.Group();
  const side = blockTex("end_frame_side"), top = blockTex("end_frame_top");
  const mats = [side, side, top, side, side, side].map((m) => new THREE.MeshBasicMaterial({ map: m }));
  const geo = new THREE.BoxGeometry(1, 0.8125, 1);
  for (let z = -2; z <= 2; z++) for (let x = -2; x <= 2; x++) {
    const ring = Math.max(Math.abs(x), Math.abs(z)) === 2 && Math.abs(x) + Math.abs(z) < 4;
    if (!ring) continue;
    const m = new THREE.Mesh(geo, mats);
    m.position.set(x + 0.5, 0.40625, z + 0.5);
    // Square to the pool, three blocks per side, like a real frame.
    m.rotation.y = Math.abs(x) === 2 ? (x > 0 ? Math.PI / 2 : -Math.PI / 2) : z > 0 ? 0 : Math.PI;
    group.add(m);
  }
  const pool = new THREE.Mesh(new THREE.PlaneGeometry(3, 3), new THREE.ShaderMaterial({
    uniforms: { uTime: { value: 0 } }, fog: false,
    vertexShader: "varying vec2 vUv; void main(){ vUv = uv; gl_Position = projectionMatrix * modelViewMatrix * vec4(position,1.0); }",
    fragmentShader: `uniform float uTime; varying vec2 vUv;
      float star(vec2 p){ vec2 i = floor(p), f = fract(p); float h = fract(sin(dot(i, vec2(12.9898,78.233))) * 43758.5453);
        return step(0.9, h) * smoothstep(0.3, 0.0, length(f - 0.5)); }
      void main(){
        vec2 q = floor(vUv * 48.0) / 48.0;
        vec3 c = vec3(0.015, 0.03, 0.045);
        c += star(q * 9.0 + vec2(uTime * 0.05, 0.0)) * vec3(0.35, 0.8, 0.7);
        c += star(q * 15.0 + vec2(0.0, uTime * 0.08) + 3.1) * vec3(0.6, 0.95, 0.85) * 0.8;
        c += star(q * 23.0 - vec2(uTime * 0.11) + 7.7) * vec3(0.2, 0.5, 0.55);
        gl_FragColor = vec4(c, 1.0);
      }`,
  }));
  pool.rotation.x = -Math.PI / 2;
  pool.position.set(0.5, 0.75, 0.5);
  group.add(pool);
  zones.nether.scene.add(group);
  return { group, pool, tint(c) { mats.forEach((m) => m.color.copy(c)); } };
})();

// Blocks the player digs through to go down a level.
function breakable(scene, top, side, colors) {
  const tex = blockTex(side), topTex = blockTex(top);
  const mats = [tex, tex, topTex, tex, tex, tex].map((m) => new THREE.MeshBasicMaterial({ map: m }));
  const crackMat = new THREE.MeshBasicMaterial({ map: crackTex[0], transparent: true, depthWrite: false });
  const g = new THREE.Group();
  const cube = new THREE.Mesh(new THREE.BoxGeometry(1, 1, 1), mats);
  const crack = new THREE.Mesh(new THREE.BoxGeometry(1.01, 1.01, 1.01), crackMat);
  cube.position.set(0.5, 0.5, 0.5); crack.position.copy(cube.position);
  g.add(cube, crack);
  scene.add(g);
  const b = {
    g, x: 0, broken: false,
    place(x, y, z) { b.x = x; g.position.set(x, y, z); },
    tint(c) { mats.forEach((m) => m.color.copy(c)); },
    stage(st) {
      const gone = st >= 6;
      if (gone && !b.broken) debris.burst(g.position.clone().add(new THREE.Vector3(0.5, 0.6, 0.5)), colors, 26, 0.8);
      b.broken = gone;
      g.visible = !gone;
      crackMat.map = crackTex[Math.min(st, 5)];
      crack.visible = st > 0;
    },
  };
  return b;
}
const digBlocks = {};

// ---------- particles ----------
const debris = (() => {
  const n = 320;
  const mesh = new THREE.InstancedMesh(new THREE.BoxGeometry(1, 1, 1), new THREE.MeshBasicMaterial({ color: 0xffffff }), n);
  mesh.frustumCulled = false;
  const parts = [], m4 = new THREE.Matrix4(), col = new THREE.Color();
  for (let i = 0; i < n; i++) { parts.push({ life: 0 }); mesh.setColorAt(i, col.set(0xffffff)); m4.makeScale(0, 0, 0); mesh.setMatrixAt(i, m4); }
  let next = 0;
  return {
    mesh,
    burst(pos, colors, count = 18, spread = 0.5) {
      if (REDUCE || jumping) return;
      for (let k = 0; k < count; k++) {
        const p = parts[next];
        p.life = 0.8 + Math.random() * 0.5;
        p.x = pos.x + (Math.random() - 0.5) * spread; p.y = pos.y + (Math.random() - 0.5) * spread; p.z = pos.z + (Math.random() - 0.5) * spread;
        p.vx = (Math.random() - 0.5) * 3; p.vy = Math.random() * 4; p.vz = (Math.random() - 0.5) * 3;
        p.s = 0.08 + Math.random() * 0.07;
        mesh.setColorAt(next, col.set(colors[k % colors.length]));
        next = (next + 1) % n;
      }
      mesh.instanceColor.needsUpdate = true;
    },
    tick(dt) {
      parts.forEach((p, i) => {
        if (p.life <= 0) return;
        p.life -= dt; p.vy -= 12 * dt;
        p.x += p.vx * dt; p.y += p.vy * dt; p.z += p.vz * dt;
        const s = p.life > 0 ? p.s * Math.min(1, p.life * 3) : 0;
        m4.makeScale(s, s, s); m4.setPosition(p.x, p.y, p.z);
        mesh.setMatrixAt(i, m4);
      });
      mesh.instanceMatrix.needsUpdate = true;
    },
  };
})();
const ambient = (() => {
  const n = 380, pos = new Float32Array(n * 3), vel = new Float32Array(n * 3);
  const geo = new THREE.BufferGeometry();
  geo.setAttribute("position", new THREE.BufferAttribute(pos, 3));
  const mat = new THREE.PointsMaterial({ size: 0.09, color: 0xffffff, transparent: true, opacity: 0.8, depthWrite: false });
  const pts = new THREE.Points(geo, mat);
  pts.frustumCulled = false;
  const MODES = {
    summit: { color: 0xfff6d8, rise: 0.05, drift: 0.3, size: 0.06 },
    mineshaft: { color: 0xd9cdb4, rise: -0.08, drift: 0.1, size: 0.05 },
    caves: { color: 0xb07cff, rise: 0.06, drift: 0.12, size: 0.07 },
    nether: { color: 0xffa040, rise: 0.9, drift: 0.3, size: 0.1, add: true },
    fortress: { color: 0xffa040, rise: 0.9, drift: 0.3, size: 0.1, add: true },
    end: { color: 0xd6b8ff, rise: 0.25, drift: 0.2, size: 0.09, add: true },
  };
  let mode = MODES.summit;
  const respawn = (i, c, top) => {
    pos[i * 3] = c.x + (Math.random() - 0.5) * 40; pos[i * 3 + 1] = c.y + (top ? -8 : (Math.random() - 0.5) * 16); pos[i * 3 + 2] = c.z + (Math.random() - 0.3) * 30;
    vel[i * 3] = (Math.random() - 0.5) * mode.drift; vel[i * 3 + 1] = mode.rise * (0.5 + Math.random()); vel[i * 3 + 2] = (Math.random() - 0.5) * mode.drift;
  };
  return {
    pts,
    set(name, center) {
      mode = MODES[name];
      mat.color.set(mode.color); mat.size = mode.size;
      mat.blending = mode.add ? THREE.AdditiveBlending : THREE.NormalBlending;
      for (let i = 0; i < n; i++) respawn(i, center, false);
      geo.attributes.position.needsUpdate = true;
    },
    tick(dt, center) {
      if (REDUCE) return;
      for (let i = 0; i < n; i++) {
        pos[i * 3] += vel[i * 3] * dt; pos[i * 3 + 1] += vel[i * 3 + 1] * dt; pos[i * 3 + 2] += vel[i * 3 + 2] * dt;
        if (Math.abs(pos[i * 3 + 1] - center.y) > 9 || Math.abs(pos[i * 3] - center.x) > 22) respawn(i, center, mode.rise > 0);
      }
      geo.attributes.position.needsUpdate = true;
    },
  };
})();

// ---------- story ----------
// Each section of the page is a chapter: which world it plays in, where the player walks,
// and where the camera sits (keyframes over the chapter's scroll progress p).
const sections = $$(".scene").map((el) => ({ el, name: el.dataset.scene, beats: $$(".beat", el), p: 0, idx: -1, q: 0, top: 0, bottom: 0 }));
const S = Object.fromEntries(sections.map((s) => [s.name, s]));
const wide = () => innerWidth >= 760;

// Walking speed: each chapter covers about 30 blocks, so the player strolls rather than sprints.
// Every world change is something the player does (digs down, drops, steps through a portal),
// and chapters in the same world hand the camera over without a cut.
const CH = {
  surface: {
    zone: "summit", tool: "pick",
    cam: [{ p: 0, dist: 16, yaw: 165, pitch: 20, shift: 0.3 }, { p: 0.16, dist: 10, yaw: 198, pitch: 12, shift: 0.28 }, { p: 0.8, dist: 9, yaw: 214, pitch: 14, shift: 0.26 }, { p: 1, dist: 7, yaw: 222, pitch: 44, shift: 0 }],
    play(s) {
      skyUniforms.uDay.value = clamp(s.p * 1.25);
      zones.summit.scene.fog.color.copy(summitHorizon(skyUniforms.uDay.value));
      const x = 30 + ease(clamp((s.p - 0.12) / 0.7)) * 28.5;
      return dig(s.p, x, digBlocks.summit, "walk");
    },
  },
  mineshaft: {
    zone: "mineshaft", tool: "pick",
    cam: [{ p: 0, dist: 6, yaw: -112, pitch: 30, shift: 0.1 }, { p: 0.06, dist: 5.2, yaw: -112, pitch: 9, shift: 0.24 }, { p: 0.82, dist: 5.4, yaw: -118, pitch: 10, shift: 0.24 }, { p: 1, dist: 4.4, yaw: -125, pitch: 42, shift: 0 }],
    play(s) {
      const X = (i) => 40 + i * 9, { idx, q } = s;
      ores.forEach((o, i) => setOre(o, i < idx ? 6 : i > idx || idx < 0 ? 0 : Math.floor(clamp(q / 0.6) * 6)));
      let st;
      if (idx < 0) st = { x: X(0) - 1.35, pose: "idle" };
      else {
        const to = idx < ores.length - 1 ? X(idx + 1) - 1.35 : 76.5;
        st = { x: lerp(X(idx) - 1.35, to, ease(clamp((q - 0.62) / (idx < ores.length - 1 ? 0.38 : 0.2)))), pose: q < 0.62 ? "mine" : "walk" };
      }
      return dig(s.p, st.x, digBlocks.mineshaft, st.pose, drop(s.p));
    },
  },
  caves: {
    zone: "caves", tool: "sword",
    cam: [{ p: 0, dist: 8, yaw: 206, pitch: 26, shift: 0.1 }, { p: 0.07, dist: 8.5, yaw: 206, pitch: 12, shift: 0.22 }, { p: 1, dist: 8.5, yaw: 212, pitch: 12, shift: 0.22 }],
    play(s, t) {
      const X = (i) => 30 + i * 8, { idx, q } = s;
      mobs.forEach((m, i) => updateMob(m, i, idx, q, X, t));
      const d = drop(s.p);
      if (idx < 0) return { x: X(0) - 6, pose: d.pose || "idle", dy: d.dy };
      const x = idx === 0 && q < 0.2 ? lerp(X(0) - 6, X(0), ease(q / 0.2)) : X(idx);
      const nx = idx < mobs.length - 1 ? X(idx + 1) : 62;
      return { x: lerp(x, nx, ease(clamp((q - 0.8) / 0.2))), pose: q < 0.42 ? "idle" : q < 0.68 ? "fight" : q > 0.8 ? "walk" : "idle" };
    },
  },
  portal: {
    zone: "caves", tool: null,
    cam: [{ p: 0, dist: 8.5, yaw: 212, pitch: 12, shift: 0.22 }, { p: 0.32, dist: 9, yaw: 236, pitch: 9, shift: 0.18 }, { p: 0.66, dist: 6, yaw: 254, pitch: 6, shift: 0.08 }, { p: 0.92, dist: 2.2, yaw: 268, pitch: 3, shift: 0 }],
    play(s, t) {
      const p = s.p, built = Math.floor(clamp((p - 0.3) / 0.24) * cavePortal.blocks.length);
      cavePortal.blocks.forEach((b, i) => { const on = i < built; if (on && !b.visible) debris.burst(b.getWorldPosition(new THREE.Vector3()), ["#3b2466", "#1b1030"], 6); b.visible = on; });
      cavePortal.lit(ease(clamp((p - 0.55) / 0.1)), t);
      const x = p < 0.3 ? lerp(62, 70, ease(p / 0.3)) : lerp(70, 77.6, ease(clamp((p - 0.68) / 0.2)));
      return { x, pose: p < 0.3 || p > 0.68 ? "walk" : p < 0.55 ? "mine" : "idle" };
    },
  },
  nether: {
    zone: "nether", tool: null,
    cam: [{ p: 0, dist: 7, yaw: 115, pitch: 10, shift: 0.05 }, { p: 0.12, dist: 11, yaw: 205, pitch: 13, shift: 0.26 }, { p: 0.55, dist: 9, yaw: 222, pitch: 10, shift: 0.26 }, { p: 1, dist: 12, yaw: 200, pitch: 16, shift: 0.26 }],
    play(s, t) {
      netherPortal.lit(1, t);
      const out = ease(clamp(s.p / 0.1));
      return { x: s.p < 0.1 ? lerp(17.2, 20, out) : 20 + ease(clamp((s.p - 0.1) / 0.88)) * 30, pose: "walk" };
    },
  },
  fortress: {
    zone: "nether", tool: null,
    cam: [{ p: 0, dist: 12, yaw: 200, pitch: 16, shift: 0.26 }, { p: 0.55, dist: 10, yaw: 216, pitch: 12, shift: 0.24 }, { p: 0.78, dist: 8, yaw: 226, pitch: 20, shift: 0.05 }, { p: 1, dist: 6.5, yaw: 234, pitch: 46, shift: 0 }],
    play(s, t) {
      endPortal.pool.material.uniforms.uTime.value = t;
      if (s.p < 0.8) return { x: 50 + ease(s.p / 0.8) * 38, pose: "walk" };
      // Hop onto the frame and into the pool.
      const k = clamp((s.p - 0.8) / 0.1);
      const sink = ease(clamp((s.p - 0.9) / 0.1));
      return { x: lerp(88, 91.5, ease(k)), pose: sink > 0 ? "fall" : "walk", dy: Math.sin(k * Math.PI) * 1.2 - sink * 3.5 };
    },
  },
  end: {
    zone: "end", tool: null,
    cam: [{ p: 0, dist: 8, yaw: 215, pitch: 30, shift: 0.05 }, { p: 0.08, dist: 10, yaw: 206, pitch: 12, shift: 0.26 }, { p: 0.6, dist: 12, yaw: 214, pitch: 13, shift: 0.26 }, { p: 1, dist: 15, yaw: 228, pitch: 7, shift: 0.05, lift: 6 }],
    play(s) {
      const d = drop(s.p);
      return { x: 28.5 + ease(clamp((s.p - 0.06) / 0.8)) * 33, pose: d.pose || (s.p > 0.86 ? "idle" : "walk"), dy: d.dy };
    },
  },
};
// The wash between worlds, with a line of story while it covers the screen.
const WASH_IN = {
  mineshaft: { bg: "#140d08", text: "Dig straight down…" },
  caves: { bg: "#05050a", text: "…and deeper still." },
  nether: { cls: "portal", text: "Through the portal." },
  end: { cls: "void", text: "In the End." },
};

// Mine the block underfoot, then drop into the hole (the last 15% of a chapter).
function dig(p, x, block, pose, before = {}) {
  if (!block) return { x, pose, ...before };
  const k = clamp((p - 0.85) / 0.08);
  block.stage(p < 0.85 ? 0 : Math.floor(k * 6));
  if (p < 0.85) return { x, pose: before.pose || pose, dy: before.dy || 0 };
  const fall = ease(clamp((p - 0.93) / 0.07));
  return { x: block.x + 0.5, pose: fall > 0 ? "fall" : "dig", dy: -fall * 5 };
}
// Land from above at the start of a chapter.
function drop(p) {
  if (p >= 0.06) return {};
  return { dy: (1 - ease(p / 0.06)) * 6, pose: "fall" };
}

function camAt(keys, p) {
  let a = keys[0], b = keys[keys.length - 1];
  for (let i = 0; i < keys.length - 1; i++) if (p >= keys[i].p && p <= keys[i + 1].p) { a = keys[i]; b = keys[i + 1]; break; }
  const t = b.p === a.p ? 0 : ease(clamp((p - a.p) / (b.p - a.p)));
  const o = {};
  for (const k of ["dist", "yaw", "pitch", "shift", "lift"]) o[k] = lerp(a[k] || 0, b[k] || 0, t);
  return o;
}

function setOre(o, stage) {
  const gone = stage >= 6;
  if (gone && !o.broken) {
    const p = o.g.position.clone(); p.y += 1;
    debris.burst(p, [...ORE_COL[o.kind], "#7f7f88", "#6e6e77"], 26, 0.9);
  }
  o.broken = gone;
  o.g.visible = !gone;
  o.crackMat.map = crackTex[Math.min(stage, 5)];
  o.crackMat.visible = stage > 0 && !gone;
}

function updateMob(m, i, idx, q, X, t) {
  const root = m.obj.root, z = zones.caves;
  if (!z.data) return;
  const gy = z.data.ground[0], PZ = z.data.PZ;
  const dead = i < idx || (i === idx && q >= 0.68);
  if (dead) {
    if (!m.dead && root.visible) { m.dead = t; debris.burst(root.position.clone().add(new THREE.Vector3(0, 0.8, 0)), ["#e8e4f0", "#bdb6cc", "#8a8496"], 24, 0.8); }
    m.dead = m.dead || t;
    root.rotation.z = Math.min(Math.PI / 2, (t - m.dead) * 6);
    root.visible = t - m.dead < 0.45 && i === idx;
    return;
  }
  m.dead = 0;
  root.rotation.z = 0;
  root.visible = i === idx;
  const a = ease(clamp(q / 0.42));
  const x = lerp(X(i) + 10, X(i) + (i === 2 ? 2.1 : 1.6), a);
  root.position.set(x, gy, PZ);
  root.rotation.y = -Math.PI / 2;
  const hurt = i === idx && q > 0.48 && Math.floor(t * 11) % 2 === 0;
  m.mats.forEach((mat) => mat.color.copy(z.actor).multiply(hurt ? new THREE.Color(1, 0.35, 0.35) : new THREE.Color(1, 1, 1)));
  if (m.obj.animate) m.obj.animate(q < 0.42 ? "walk" : "idle", x * 1.2, t);
  else if (root.userData.legs) root.userData.legs.forEach((l, k) => (l.rotation.x = q < 0.42 ? Math.sin(t * 18 + k) * 0.25 : 0));
  else root.position.y = gy + (q < 0.42 ? Math.abs(Math.sin(t * 6)) * 0.5 : 0);
}

// ---------- smooth scrolling ----------
// Wheel and trackpad input glide towards a target instead of jumping, and in-page links glide
// too. Touch, keyboard and the scrollbar stay native; reduced-motion users get plain scrolling.
const smooth = (() => {
  let target = scrollY, cur = scrollY, gliding = false, ours = false;
  const max = () => document.documentElement.scrollHeight - innerHeight;
  const go = (y) => { target = clamp(y, 0, max()); if (!gliding) cur = scrollY; gliding = true; };
  if (!REDUCE) {
    addEventListener("wheel", (e) => {
      if (e.ctrlKey || e.defaultPrevented || e.target.closest("dialog, .term")) return;
      e.preventDefault();
      const d = e.deltaMode === 1 ? e.deltaY * 40 : e.deltaMode === 2 ? e.deltaY * innerHeight : e.deltaY;
      go((gliding ? target : scrollY) + d);
    }, { passive: false });
    document.addEventListener("click", (e) => {
      const a = e.target.closest('a[href^="#"]');
      const el = a && document.getElementById(a.getAttribute("href").slice(1));
      if (!el && a?.getAttribute("href") !== "#top") return;
      e.preventDefault();
      // Land on a chapter with its first card in view, past the fade between worlds.
      go(el ? el.getBoundingClientRect().top + scrollY + (el.classList.contains("scene") ? innerHeight * 0.5 : 0) : 0);
      history.replaceState(null, "", a.getAttribute("href"));
    });
  }
  // Anything else that scrolls the page (keys, scrollbar, touch) takes over from a glide.
  addEventListener("scroll", () => { if (!ours) { gliding = false; target = cur = scrollY; } ours = false; }, { passive: true });
  return {
    tick(dt) {
      if (!gliding) return;
      cur += (target - cur) * (1 - Math.exp(-dt * 8));
      if (Math.abs(target - cur) < 0.4) { cur = target; gliding = false; }
      ours = true;
      scrollTo(0, cur);
    },
  };
})();
const progressBar = $("#progress");

// ---------- frame loop ----------
let activeZone = null, current = sections[0], jumping = false, lastY = -1;
let heroX = 30, heroPrevX = 30, walked = 0, landed = true;
const camPos = new THREE.Vector3(), camLook = new THREE.Vector3(), tmp = new THREE.Vector3();
let snap = true;

function groundAt(z, x) {
  const g = z.data.ground, i = clamp(Math.floor(x), 0, g.length - 2), f = x - Math.floor(x);
  return lerp(g[i], g[i + 1], f);
}

function switchZone(z) {
  if (activeZone === z) return;
  activeZone = z;
  z.scene.add(hero.root, debris.mesh, ambient.pts, ...buddies.map((b) => b.root));
  $("#loading").hidden = z.ready;
  snap = true;
  if (z.ready) ambient.set(z.name, hero.root.position);
}
function onZoneReady(z) {
  if (z.name === "mineshaft") {
    const PZ = z.data.PZ, y = z.data.ground[0];
    ores.forEach((o, i) => o.g.position.set(40 + i * 9 + 0.5, y, PZ + 0.5));
    const railMat = new THREE.MeshBasicMaterial({ color: 0x8c8c96 }), tie = new THREE.MeshBasicMaterial({ map: blockTex("planks") });
    for (const dz of [-0.28, 0.28]) { const r = new THREE.Mesh(new THREE.BoxGeometry(z.data.size[0], 0.06, 0.07), railMat); r.position.set(z.data.size[0] / 2, y + 0.07, PZ + 0.5 + dz); rails.add(r); }
    for (let x = 0; x < z.data.size[0]; x += 0.5) { const t = new THREE.Mesh(new THREE.BoxGeometry(0.18, 0.04, 0.9), tie); t.position.set(x, y + 0.02, PZ + 0.5); rails.add(t); }
  }
  if (z.name === "summit") {
    digBlocks.summit = breakable(z.scene, "grass_top", "grass_side", ["#5fae3d", "#7a5236", "#86603f"]);
    digBlocks.summit.place(58, z.data.ground[58] - 1, z.data.PZ);
  }
  if (z.name === "mineshaft") {
    digBlocks.mineshaft = breakable(z.scene, "planks", "planks", ["#a7794a", "#6e4a2b"]);
    digBlocks.mineshaft.place(76, z.data.ground[76] - 1, z.data.PZ);
    digBlocks.mineshaft.tint(z.actor);
  }
  if (z.name === "caves") cavePortal.group.position.set(76, z.data.ground[0], z.data.PZ - 1.5);
  if (z.name === "nether") {
    netherPortal.group.position.set(16.5, z.data.ground[0], z.data.PZ - 1.5);
    endPortal.group.position.set(91, z.data.ground[0], z.data.PZ);
    endPortal.tint(z.actor);
  }
  if (z === activeZone) ambient.set(z.name, hero.root.position);
}

function readBeats(s) {
  const line = innerHeight * 0.55;
  let idx = -1, q = 0;
  s.beats.forEach((b, i) => { const r = b.getBoundingClientRect(); if (r.top <= line) { idx = i; q = clamp((line - r.top) / r.height); } });
  s.idx = idx; s.q = q;
  s.beats.forEach((b, i) => b.classList.toggle("on", s === current && i === idx));
}

const wash = $("#wash"), washText = $("#washText"), depthY = $("#depthY"), depthList = $("#depthList"), depthEl = $(".depth"), finale = $("#download");
depthList.innerHTML = sections.map((s) => `<li data-for="${s.name}">${s.el.dataset.label}</li>`).join("");
const depthItems = $$("li", depthList), navLinks = $$(".bar nav a");
const NAV = { surface: "surface", mineshaft: "mineshaft", caves: "mineshaft", portal: "mineshaft", nether: "nether", fortress: "fortress", end: "end" };

let last = performance.now();
function frame(now) {
  const dt = Math.min(0.05, (now - last) / 1000), t = now / 1000;
  last = now;
  smooth.tick(dt);
  const H = innerHeight, y = scrollY;
  progressBar.style.transform = `scaleX(${(y / Math.max(1, document.documentElement.scrollHeight - H)).toFixed(4)})`;
  jumping = Math.abs(y - lastY) > H * 0.6;
  lastY = y;

  // Which chapter owns the middle of the screen, and how far through it we are.
  let active = null;
  for (const s of sections) {
    const r = s.el.getBoundingClientRect();
    s.top = r.top; s.bottom = r.bottom;
    s.p = clamp(-r.top / (r.height - H));
    if (r.top <= H / 2 && r.bottom > H / 2) active = s;
  }
  if (active) current = active;
  readBeats(current);
  const ch = CH[current.name], zone = zones[ch.zone];
  switchZone(zone);

  // Fades between worlds, with a line of story while the screen is covered.
  let w = 0, wi = null;
  const i = sections.indexOf(current), nextS = sections[i + 1];
  if (!active) { w = 1; wi = nextS && WASH_IN[nextS.name]; }
  else {
    if (WASH_IN[current.name]) { w = Math.max(w, 1 - clamp(current.p / 0.07)); wi = WASH_IN[current.name]; }
    if (nextS && WASH_IN[nextS.name] && current.p > 0.86) { w = Math.max(w, clamp((current.p - 0.9) / 0.1)); wi = WASH_IN[nextS.name]; }
  }
  wi = wi || { bg: "#000", text: "" };
  wash.className = wi.cls || "";
  wash.style.background = wi.cls ? "" : wi.bg;
  wash.style.opacity = w.toFixed(3);
  washText.textContent = wi.text;
  washText.style.opacity = clamp((w - 0.6) / 0.4).toFixed(3);

  // Load the worlds nearest to here first.
  const order = [i, i + 1, i - 1, i + 2, i + 3, i - 2].filter((k) => sections[k]).map((k) => CH[sections[k].name].zone);
  pool.want([...new Set(order)]);

  if (zone.ready) {
    const st = ch.play(current, t);
    heroPrevX = heroX; heroX = st.x;
    walked += Math.abs(heroX - heroPrevX);
    const moving = Math.abs(heroX - heroPrevX) > 0.001;
    let pose = st.pose;
    if (pose === "walk" && !moving) pose = "idle";
    if (!active) pose = "fall";
    const gy = groundAt(zone, heroX), PZ = zone.data.PZ + 0.5;
    hero.root.position.set(heroX, gy + (st.dy || 0), PZ);
    if (current.name === "caves" && st.dy === undefined && !landed) { landed = true; debris.burst(hero.root.position.clone(), ["#4a4f66", "#2d3040"], 14, 0.9); }
    if (st.dy > 0) landed = false;
    hero.root.rotation.y = Math.PI / 2;
    hero.tool(ch.tool);
    hero.setTint(zone.actor);
    hero.animate(pose, walked, t);

    // Friends join in the Nether and stay for the rest of the journey.
    let n = 0;
    if (current.name === "nether") n = current.idx > 0 || (current.idx === 0 && current.q > 0.3) ? 1 : 0;
    if (current.name === "fortress") n = 1 + (current.idx > 1 || (current.idx === 1 && current.q > 0.35)) + (current.idx > 2 || (current.idx === 2 && current.q > 0.35));
    if (current.name === "end") n = 3;
    buddies.forEach((b, k) => {
      const show = k < n;
      if (show && !b.root.visible) debris.burst(new THREE.Vector3(heroX - 1.7 * (k + 1), gy + 1, PZ + (k % 2 ? 0.9 : -0.9)), ["#c8a2ff", "#ffffff", "#9b5cff"], 20, 0.8);
      b.root.visible = show;
      const bx = heroX - 1.7 * (k + 1);
      b.root.position.set(bx, groundAt(zone, Math.max(0, bx)), PZ + (k % 2 ? 0.9 : -0.9));
      b.root.rotation.y = Math.PI / 2;
      b.setTint(zone.actor);
      b.animate(pose === "fight" || pose === "mine" ? "idle" : pose, walked + k * 0.7, t + k);
    });

    // Camera: orbit the player per the chapter's keyframes, framed to leave room for the cards.
    const c = camAt(ch.cam, current.p);
    const target = tmp.set(heroX, gy + 1.15, PZ);
    const cp = Math.cos(c.pitch * DEG);
    const want = new THREE.Vector3(target.x + Math.sin(c.yaw * DEG) * cp * c.dist, target.y + Math.sin(c.pitch * DEG) * c.dist, target.z + Math.cos(c.yaw * DEG) * cp * c.dist);
    const fwd = target.clone().sub(want).normalize();
    const right = new THREE.Vector3().crossVectors(fwd, new THREE.Vector3(0, 1, 0)).normalize();
    const look = target.clone();
    look.y += c.lift;
    if (wide()) look.addScaledVector(right, c.shift * c.dist * 0.9);
    else look.y += c.dist * 0.14;
    const k = snap || REDUCE ? 1 : 1 - Math.exp(-dt * 7);
    camPos.lerp(want, k); camLook.lerp(look, k);
    if (snap) { camPos.copy(want); camLook.copy(look); snap = false; }
    camera.position.copy(camPos);
    camera.lookAt(camLook);

    debris.tick(dt);
    ambient.tick(dt, hero.root.position);
    if (zone.clouds) zone.clouds.userData.tick(dt);
    if (zone === zones.end && !REDUCE) {
      const a = t * 0.12;
      dragon.position.set(105 + Math.cos(a) * 62, 82 + Math.sin(a * 2) * 6, 110 + Math.sin(a) * 44);
      dragon.rotation.set(0, -a + Math.PI, Math.sin(a) * 0.25);
      dragon.userData.wings.forEach(({ p, side }) => (p.rotation.z = side * Math.sin(t * 3.2) * 0.6));
    }
  }
  hero.root.visible = zone.ready;
  if (renderer) renderer.render(zone.scene, camera);

  // Depth meter and nav.
  depthEl.style.opacity = finale.getBoundingClientRect().top < H * 0.7 ? 0 : 1;
  depthY.textContent = active ? current.el.dataset.y : "…";
  depthItems.forEach((li) => li.classList.toggle("on", li.dataset.for === current.name));
  navLinks.forEach((a) => a.classList.toggle("on", a.getAttribute("href") === "#" + NAV[current.name]));
  requestAnimationFrame(frame);
}

function resize() {
  const w = innerWidth, h = innerHeight;
  if (renderer) renderer.setSize(w, h, false);
  camera.aspect = w / h;
  camera.updateProjectionMatrix();
  document.documentElement.style.setProperty("--beat", Math.round(h * 0.92) + "px");
}
addEventListener("resize", resize);
sections.forEach((s) => s.el.style.setProperty("--beats", s.beats.length + parseFloat(s.el.dataset.extra || 0)));
resize();
requestAnimationFrame(frame);

// ---------- your skin ----------
// Drop (or pick) a 64×64 or 64×32 skin PNG and the explorer wears it, like in netherNode.
function useSkin(file) {
  if (!file || !/image\/png/.test(file.type)) return;
  const img = new Image();
  img.onload = () => {
    if (img.width !== 64 || (img.height !== 64 && img.height !== 32)) { skinMsg("That isn't a 64×64 skin."); return; }
    const c = document.createElement("canvas");
    c.width = 64; c.height = img.height;
    c.getContext("2d").drawImage(img, 0, 0);
    // Slim (3-pixel) arms leave this pixel of the right arm empty.
    const slim = img.height === 64 && c.getContext("2d").getImageData(54, 20, 1, 1).data[3] === 0;
    hero.setSkin(c, slim);
    skinMsg("Looking good. Scroll on.");
    URL.revokeObjectURL(img.src);
  };
  img.src = URL.createObjectURL(file);
}
const skinMsg = (m) => { const el = $("#skinMsg"); if (el) el.textContent = m; };
addEventListener("dragover", (e) => { e.preventDefault(); document.body.classList.add("dropping"); });
addEventListener("dragleave", (e) => { if (!e.relatedTarget) document.body.classList.remove("dropping"); });
addEventListener("drop", (e) => { e.preventDefault(); document.body.classList.remove("dropping"); useSkin(e.dataTransfer.files[0]); });
$("#skinInput")?.addEventListener("change", (e) => useSkin(e.target.files[0]));

// ---------- downloads ----------
function detectOS() {
  const ua = navigator.userAgent;
  if (/iPhone|iPad|Android/i.test(ua)) return null;
  if (/Windows/i.test(ua)) return "windows";
  if (/Mac/i.test(ua)) return "mac";
  if (/Linux|X11|CrOS/i.test(ua)) return "linux";
  return null;
}
const OS_NAME = { windows: "Windows", mac: "macOS", linux: "Linux" };
const OS_ASSET = { windows: /windows-x64\.zip$/, mac: /macos-universal\.dmg$/, linux: /linux-x64\.tar\.gz$/ };
(() => {
  const os = detectOS(), heroBtn = $("#heroDownload");
  if (os) { heroBtn.textContent = `Download for ${OS_NAME[os]}`; $(`.platform[data-os="${os}"]`).classList.add("mine"); }
  else heroBtn.href = "#download";
  fetch(`https://api.github.com/repos/${REPO}/releases/latest`, { headers: { Accept: "application/vnd.github+json" } })
    .then((r) => (r.ok ? r.json() : null))
    .then((rel) => {
      if (!rel || !rel.assets) return;
      for (const key of Object.keys(OS_ASSET)) {
        const a = rel.assets.find((x) => OS_ASSET[key].test(x.name));
        if (!a) continue;
        const card = $(`.platform[data-os="${key}"]`);
        card.href = a.browser_download_url;
        $("em", card).textContent = `${a.name} · ${(a.size / 1048576).toFixed(0)} MB`;
        if (key === os) heroBtn.href = a.browser_download_url;
      }
      $("#heroVersion").textContent = `${rel.tag_name} · Free and open source (GPL-3.0). No accounts, no ads, no telemetry.`;
      $("#finaleVersion").textContent = `netherNode ${rel.tag_name}, released ${new Date(rel.published_at).toLocaleDateString(undefined, { year: "numeric", month: "long", day: "numeric" })}.`;
    })
    .catch(() => {});
})();

// ---------- screenshots ----------
{
  const dlg = $("#lightbox"), img = $("img", dlg);
  $$(".shot").forEach((b) => {
    b.addEventListener("click", () => { img.src = b.dataset.src; img.alt = $("img", b).alt; dlg.showModal(); });
    // A screenshot that can't load is left out rather than shown broken.
    const im = $("img", b);
    const hide = () => { b.hidden = true; };
    im.addEventListener("error", hide);
    if (im.complete && im.naturalWidth === 0 && im.src) im.decode().catch(hide);
  });
  dlg.addEventListener("click", (e) => { if (e.target === dlg) dlg.close(); });
}
