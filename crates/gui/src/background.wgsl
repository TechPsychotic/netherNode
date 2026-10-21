// netherNode background: an obsidian frame around a living nether portal.
//
// The portal mixes a smooth, high-resolution swirl (domain-warped noise) with the game's own
// pixel-art portal animation, plus particles drifting up. The obsidian uses the game's
// texture when available, otherwise a procedural stand-in.

struct Uniforms {
    origin: vec2<f32>,      // widget origin, physical px
    resolution: vec2<f32>,  // widget size, physical px
    time: f32,
    block: f32,             // size of one block, physical px
    portal_frames: f32,     // frames in the portal strip (0 = no game textures)
    has_obsidian: f32,
    srgb_target: f32,       // 1 when the surface converts linear -> sRGB itself
    _pad0: f32,
    _pad1: f32,
    _pad2: f32,
};

@group(0) @binding(0) var<uniform> u: Uniforms;
@group(0) @binding(1) var obsidian_tex: texture_2d<f32>;
@group(0) @binding(2) var portal_tex: texture_2d<f32>;
@group(0) @binding(3) var pixel_sampler: sampler;

@vertex
fn vs_main(@builtin(vertex_index) i: u32) -> @builtin(position) vec4<f32> {
    // One triangle that covers the whole viewport.
    let p = vec2<f32>(f32((i << 1u) & 2u), f32(i & 2u));
    return vec4<f32>(p * 2.0 - 1.0, 0.0, 1.0);
}

fn hash21(p: vec2<f32>) -> f32 {
    var q = fract(p * vec2<f32>(123.34, 456.21));
    q += dot(q, q + 45.32);
    return fract(q.x * q.y);
}

fn noise(p: vec2<f32>) -> f32 {
    let i = floor(p);
    let f = fract(p);
    let s = f * f * (3.0 - 2.0 * f);
    let a = hash21(i);
    let b = hash21(i + vec2<f32>(1.0, 0.0));
    let c = hash21(i + vec2<f32>(0.0, 1.0));
    let d = hash21(i + vec2<f32>(1.0, 1.0));
    return mix(mix(a, b, s.x), mix(c, d, s.x), s.y);
}

fn fbm(p0: vec2<f32>) -> f32 {
    var p = p0;
    var v = 0.0;
    var a = 0.5;
    for (var k = 0; k < 5; k++) {
        v += a * noise(p);
        p = p * 2.03 + vec2<f32>(17.1, 9.2);
        a *= 0.5;
    }
    return v;
}

// Deep violet -> purple -> magenta -> pale lilac.
fn portal_palette(t: f32) -> vec3<f32> {
    let c0 = vec3<f32>(0.05, 0.01, 0.12);
    let c1 = vec3<f32>(0.30, 0.06, 0.62);
    let c2 = vec3<f32>(0.62, 0.22, 0.95);
    let c3 = vec3<f32>(0.93, 0.78, 1.00);
    if (t < 0.45) { return mix(c0, c1, t / 0.45); }
    if (t < 0.8) { return mix(c1, c2, (t - 0.45) / 0.35); }
    return mix(c2, c3, (t - 0.8) / 0.2);
}

fn obsidian(local: vec2<f32>) -> vec3<f32> {
    let uv = local / u.block;
    if (u.has_obsidian > 0.5) {
        return textureSampleLevel(obsidian_tex, pixel_sampler, uv, 0.0).rgb;
    }
    // Procedural 16x16 stand-in: dark purples with a few lighter flecks.
    let texel = floor(uv * 16.0);
    let h = hash21(texel + 3.7);
    var c = mix(vec3<f32>(0.04, 0.03, 0.07), vec3<f32>(0.11, 0.07, 0.19), h);
    if (h > 0.9) { c = vec3<f32>(0.23, 0.15, 0.36); }
    return c;
}

fn swirl(p: vec2<f32>, t: f32) -> f32 {
    // Domain warping: noise displaced by noise, drifting and slowly rotating.
    let q = vec2<f32>(fbm(p + vec2<f32>(0.0, t * 0.15)), fbm(p + vec2<f32>(5.2, 1.3) - t * 0.1));
    let r = vec2<f32>(fbm(p + 3.0 * q + vec2<f32>(1.7, 9.2) + t * 0.12), fbm(p + 3.0 * q + vec2<f32>(8.3, 2.8)));
    return fbm(p + 3.5 * r);
}

fn particles(px: vec2<f32>, t: f32) -> f32 {
    // Grid cells each own one square particle rising at its own speed.
    let cell = u.block * 1.5;
    var glow = 0.0;
    for (var layer = 0; layer < 2; layer++) {
        let speed = cell * (0.35 + 0.25 * f32(layer));
        let shifted = px + vec2<f32>(0.0, t * speed);
        let id = floor(shifted / cell) + f32(layer) * 31.0;
        let h = hash21(id);
        if (h < 0.55) { continue; }
        let center = (floor(shifted / cell) + vec2<f32>(hash21(id + 1.3), hash21(id + 7.1))) * cell;
        let size = u.block / 8.0 * (1.0 + h);
        let d = abs(shifted - center);
        let flicker = 0.5 + 0.5 * sin(t * 3.0 + h * 40.0);
        if (max(d.x, d.y) < size) { glow += 0.8 * flicker; }
    }
    return glow;
}

// Colours here are sRGB values; an sRGB surface expects linear ones.
fn output(c: vec3<f32>) -> vec4<f32> {
    if (u.srgb_target > 0.5) {
        return vec4<f32>(pow(max(c, vec3<f32>(0.0)), vec3<f32>(2.2)), 1.0);
    }
    return vec4<f32>(c, 1.0);
}

@fragment
fn fs_main(@builtin(position) frag: vec4<f32>) -> @location(0) vec4<f32> {
    let px = frag.xy - u.origin;
    let res = u.resolution;
    let b = u.block;

    // Obsidian frame one block thick. Tiles are anchored to the nearest edge so all four
    // corners line up exactly whatever the window size.
    let in_frame = px.x < b || px.y < b || px.x > res.x - b || px.y > res.y - b;
    if (in_frame) {
        var local = px;
        if (px.x > res.x - b) { local.x = px.x - (res.x - b); }
        if (px.y > res.y - b) { local.y = px.y - (res.y - b); }
        local = local - floor(local / b) * b;
        var c = obsidian(local);
        // Subtle violet glow spilling onto the frame from the portal.
        let inner = min(min(px.x, res.x - px.x), min(px.y, res.y - px.y));
        c += vec3<f32>(0.25, 0.08, 0.45) * smoothstep(0.0, b, inner) * 0.25;
        return output(c);
    }

    let t = u.time;
    let p = px / min(res.x, res.y) * 3.0;
    let s = swirl(p, t);
    var col = portal_palette(clamp(s * 1.25 - 0.1, 0.0, 1.0));

    if (u.portal_frames > 0.5) {
        // The game's portal animation, upscaled with crisp pixels and gently warped.
        let frame = floor(t * 10.0) % u.portal_frames;
        let warp = vec2<f32>(s - 0.5, fbm(p * 1.3 + t * 0.2) - 0.5) * 0.35;
        var uv = fract(px / (b * 2.0) + warp);
        uv.y = (uv.y + frame) / u.portal_frames;
        let pixel = textureSampleLevel(portal_tex, pixel_sampler, uv, 0.0).rgb;
        col = mix(col, pixel * 1.1, 0.42);
    }

    col += vec3<f32>(0.85, 0.6, 1.0) * particles(px, t) * 0.35;

    // Depth: darker towards the frame, a soft bright core.
    let inner = min(min(px.x - b, res.x - b - px.x), min(px.y - b, res.y - b - px.y));
    col *= mix(0.45, 1.0, smoothstep(0.0, b * 2.5, inner));
    let centre = length((px - res * 0.5) / res);
    col += vec3<f32>(0.18, 0.05, 0.3) * (1.0 - smoothstep(0.0, 0.6, centre));

    return output(col);
}
