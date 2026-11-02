// Converts planar Y'CbCr to display R'G'B', tone mapping HDR to SDR.

struct Params {
    // Y'CbCr -> R'G'B' matrix rows.
    rows: array<vec4<f32>, 3>,
    // x: luma offset, y: luma scale, z: chroma offset, w: chroma scale.
    range: vec4<f32>,
    // Linear-light source primaries -> BT.709 rows.
    gamut: array<vec4<f32>, 3>,
    // x: sample scale, y: transfer (0 SDR, 1 PQ, 2 HLG), z: peak / SDR white, w: 1 if the
    // target expects linear values (sRGB format).
    misc: vec4<f32>,
    // x: 1 if colors need linear-light processing (HDR or wide gamut).
    flags: vec4<f32>,
};

@group(0) @binding(0) var<uniform> params: Params;
@group(0) @binding(1) var plane_y: texture_2d<f32>;
@group(0) @binding(2) var plane_u: texture_2d<f32>;
@group(0) @binding(3) var plane_v: texture_2d<f32>;
@group(0) @binding(4) var plane_sampler: sampler;

struct VertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

// One triangle covering the viewport.
@vertex
fn vs_main(@builtin(vertex_index) index: u32) -> VertexOutput {
    let uv = vec2<f32>(f32((index << 1u) & 2u), f32(index & 2u));
    var out: VertexOutput;
    out.position = vec4<f32>(uv.x * 2.0 - 1.0, 1.0 - uv.y * 2.0, 0.0, 1.0);
    out.uv = uv;
    return out;
}

const SDR_WHITE_NITS: f32 = 203.0;

fn pq_to_nits(encoded: vec3<f32>) -> vec3<f32> {
    let m1 = 0.1593017578125;
    let m2 = 78.84375;
    let c1 = 0.8359375;
    let c2 = 18.8515625;
    let c3 = 18.6875;
    let p = pow(max(encoded, vec3<f32>(0.0)), vec3<f32>(1.0 / m2));
    let linear = pow(max(p - c1, vec3<f32>(0.0)) / (c2 - c3 * p), vec3<f32>(1.0 / m1));
    return linear * 10000.0;
}

fn hlg_to_nits(encoded: vec3<f32>) -> vec3<f32> {
    let a = 0.17883277;
    let b = 0.28466892;
    let c = 0.55991073;
    let e = max(encoded, vec3<f32>(0.0));
    let low = e * e / 3.0;
    let high = (exp((e - c) / a) + b) / 12.0;
    let scene = select(high, low, e <= vec3<f32>(0.5));
    // Display OOTF for a 1000-nit reference display, system gamma 1.2.
    let luma = dot(scene, vec3<f32>(0.2627, 0.6780, 0.0593));
    return 1000.0 * pow(max(luma, 1e-6), 0.2) * scene;
}

fn luminance(rgb: vec3<f32>) -> f32 {
    return dot(rgb, vec3<f32>(0.2126, 0.7152, 0.0722));
}

// Extended Reinhard on luminance: keeps midtones, rolls highlights off to `peak`.
fn tone_map(rgb: vec3<f32>, peak: f32) -> vec3<f32> {
    let l = luminance(rgb);
    if (l <= 0.0) {
        return vec3<f32>(0.0);
    }
    let mapped = l * (1.0 + l / (peak * peak)) / (1.0 + l);
    return rgb * (mapped / l);
}

fn srgb_to_linear(c: vec3<f32>) -> vec3<f32> {
    let low = c / 12.92;
    let high = pow((c + 0.055) / 1.055, vec3<f32>(2.4));
    return select(high, low, c <= vec3<f32>(0.04045));
}

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    let scale = params.misc.x;
    let y = (textureSample(plane_y, plane_sampler, in.uv).r * scale - params.range.x) * params.range.y;
    let cb = (textureSample(plane_u, plane_sampler, in.uv).r * scale - params.range.z) * params.range.w;
    let cr = (textureSample(plane_v, plane_sampler, in.uv).r * scale - params.range.z) * params.range.w;
    let ycc = vec3<f32>(y, cb, cr);
    var rgb = clamp(
        vec3<f32>(dot(params.rows[0].xyz, ycc), dot(params.rows[1].xyz, ycc), dot(params.rows[2].xyz, ycc)),
        vec3<f32>(0.0),
        vec3<f32>(1.0),
    );

    if (params.flags.x > 0.5) {
        let transfer = params.misc.y;
        var linear: vec3<f32>;
        if (transfer > 1.5) {
            linear = hlg_to_nits(rgb) / SDR_WHITE_NITS;
        } else if (transfer > 0.5) {
            linear = pq_to_nits(rgb) / SDR_WHITE_NITS;
        } else {
            linear = pow(rgb, vec3<f32>(2.4));
        }
        linear = max(vec3<f32>(
            dot(params.gamut[0].xyz, linear),
            dot(params.gamut[1].xyz, linear),
            dot(params.gamut[2].xyz, linear),
        ), vec3<f32>(0.0));
        if (transfer > 0.5) {
            linear = tone_map(linear, params.misc.z);
        }
        rgb = pow(clamp(linear, vec3<f32>(0.0), vec3<f32>(1.0)), vec3<f32>(1.0 / 2.4));
    }

    if (params.misc.w > 0.5) {
        rgb = srgb_to_linear(rgb);
    }
    return vec4<f32>(rgb, 1.0);
}
