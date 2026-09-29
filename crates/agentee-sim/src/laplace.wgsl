struct Params {
    nx: u32,
    ny: u32,
    color: u32,
    omega: f32,
}

@group(0) @binding(0) var<storage, read_write> phi: array<f32>;
@group(0) @binding(1) var<storage, read> ex: array<f32>;
@group(0) @binding(2) var<storage, read> ey: array<f32>;
@group(0) @binding(3) var<storage, read> fixed: array<u32>;
@group(0) @binding(4) var<uniform> p: Params;

@compute @workgroup_size(16, 16)
fn sor(@builtin(global_invocation_id) id: vec3<u32>) {
    let i = id.x;
    let j = id.y;
    if (i >= p.nx || j >= p.ny) {
        return;
    }
    if (((i + j) & 1u) != p.color) {
        return;
    }
    let k = j * p.nx + i;
    if (fixed[k] != 0u) {
        return;
    }
    var num = 0.0;
    var den = 0.0;
    if (i > 0u) {
        let e = ex[k - 1u];
        num += e * phi[k - 1u];
        den += e;
    }
    if (i + 1u < p.nx) {
        let e = ex[k];
        num += e * phi[k + 1u];
        den += e;
    }
    if (j > 0u) {
        let e = ey[k - p.nx];
        num += e * phi[k - p.nx];
        den += e;
    }
    if (j + 1u < p.ny) {
        let e = ey[k];
        num += e * phi[k + p.nx];
        den += e;
    }
    if (den > 0.0) {
        phi[k] = phi[k] + p.omega * (num / den - phi[k]);
    }
}
