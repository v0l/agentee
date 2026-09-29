struct Params {
    nx: u32,
    ny: u32,
    nz: u32,
    n: u32,
    groups: u32,
    links: u32,
    pad1: u32,
    pad2: u32,
}

@group(0) @binding(0) var<uniform> P: Params;
@group(0) @binding(1) var<storage, read> gx: array<f32>;
@group(0) @binding(2) var<storage, read> gy: array<f32>;
@group(0) @binding(3) var<storage, read> gz: array<f32>;
@group(0) @binding(4) var<storage, read> diag: array<f32>;
@group(0) @binding(5) var<storage, read_write> x: array<f32>;
@group(0) @binding(6) var<storage, read_write> r: array<f32>;
@group(0) @binding(7) var<storage, read_write> p: array<f32>;
@group(0) @binding(8) var<storage, read_write> ap: array<f32>;
@group(0) @binding(9) var<storage, read_write> partial: array<f32>;
@group(0) @binding(10) var<storage, read_write> scalars: array<f32>;
@group(0) @binding(11) var<storage, read> free: array<u32>;
@group(0) @binding(12) var<storage, read> links: array<f32>;

var<workgroup> sa: array<f32, 256>;
var<workgroup> sb: array<f32, 256>;

@compute @workgroup_size(256, 1, 1)
fn apply(@builtin(global_invocation_id) g: vec3<u32>) {
    let id = g.x + g.y * 65535u * 256u;
    if id >= P.n { return; }
    if free[id] == 0u {
        ap[id] = p[id];
        return;
    }
    let sx = P.ny * P.nz;
    let sy = P.nz;
    let k = id % P.nz;
    let j = (id / P.nz) % P.ny;
    let i = id / sx;
    var s = diag[id] * p[id];
    if i + 1u < P.nx { s -= gx[id] * p[id + sx] * f32(free[id + sx]); }
    if i > 0u { s -= gx[id - sx] * p[id - sx] * f32(free[id - sx]); }
    if j + 1u < P.ny { s -= gy[id] * p[id + sy] * f32(free[id + sy]); }
    if j > 0u { s -= gy[id - sy] * p[id - sy] * f32(free[id - sy]); }
    if k + 1u < P.nz { s -= gz[id] * p[id + 1u] * f32(free[id + 1u]); }
    if k > 0u { s -= gz[id - 1u] * p[id - 1u] * f32(free[id - 1u]); }
    ap[id] = s;
}

@compute @workgroup_size(256, 1, 1)
fn dot_pap(@builtin(local_invocation_id) l: vec3<u32>, @builtin(workgroup_id) w: vec3<u32>) {
    var s = 0.0;
    var i = w.x * 256u + l.x;
    while i < P.n {
        s += p[i] * ap[i];
        i += P.groups * 256u;
    }
    sa[l.x] = s;
    workgroupBarrier();
    for (var k = 128u; k > 0u; k /= 2u) {
        if l.x < k { sa[l.x] += sa[l.x + k]; }
        workgroupBarrier();
    }
    if l.x == 0u { partial[w.x] = sa[0]; }
}

@compute @workgroup_size(256, 1, 1)
fn reduce_alpha(@builtin(local_invocation_id) l: vec3<u32>) {
    var s = 0.0;
    var i = l.x;
    while i < P.groups {
        s += partial[i];
        i += 256u;
    }
    sa[l.x] = s;
    workgroupBarrier();
    for (var k = 128u; k > 0u; k /= 2u) {
        if l.x < k { sa[l.x] += sa[l.x + k]; }
        workgroupBarrier();
    }
    if l.x == 0u {
        scalars[1] = sa[0];
        scalars[2] = select(0.0, scalars[0] / sa[0], sa[0] != 0.0);
    }
}

@compute @workgroup_size(256, 1, 1)
fn update_xr(@builtin(local_invocation_id) l: vec3<u32>, @builtin(workgroup_id) w: vec3<u32>) {
    let alpha = scalars[2];
    var s_rz = 0.0;
    var s_rr = 0.0;
    var i = w.x * 256u + l.x;
    while i < P.n {
        x[i] += alpha * p[i];
        let ri = r[i] - alpha * ap[i];
        r[i] = ri;
        let d = diag[i];
        let z = select(0.0, ri / d, d > 0.0);
        s_rz += ri * z;
        s_rr += ri * ri;
        i += P.groups * 256u;
    }
    sa[l.x] = s_rz;
    sb[l.x] = s_rr;
    workgroupBarrier();
    for (var k = 128u; k > 0u; k /= 2u) {
        if l.x < k {
            sa[l.x] += sa[l.x + k];
            sb[l.x] += sb[l.x + k];
        }
        workgroupBarrier();
    }
    if l.x == 0u {
        partial[w.x] = sa[0];
        partial[P.groups + w.x] = sb[0];
    }
}

@compute @workgroup_size(256, 1, 1)
fn reduce_beta(@builtin(local_invocation_id) l: vec3<u32>) {
    var a = 0.0;
    var b = 0.0;
    var i = l.x;
    while i < P.groups {
        a += partial[i];
        b += partial[P.groups + i];
        i += 256u;
    }
    sa[l.x] = a;
    sb[l.x] = b;
    workgroupBarrier();
    for (var k = 128u; k > 0u; k /= 2u) {
        if l.x < k {
            sa[l.x] += sa[l.x + k];
            sb[l.x] += sb[l.x + k];
        }
        workgroupBarrier();
    }
    if l.x == 0u {
        scalars[3] = select(0.0, sa[0] / scalars[0], scalars[0] != 0.0);
        scalars[0] = sa[0];
        scalars[4] = sb[0];
    }
}

@compute @workgroup_size(256, 1, 1)
fn update_p(@builtin(global_invocation_id) g: vec3<u32>) {
    let id = g.x + g.y * 65535u * 256u;
    if id >= P.n { return; }
    let d = diag[id];
    let z = select(0.0, r[id] / d, d > 0.0);
    p[id] = z + scalars[3] * p[id];
}

@compute @workgroup_size(1, 1, 1)
fn apply_links() {
    for (var l = 0u; l < P.links; l++) {
        let a = u32(links[l * 3u]);
        let b = u32(links[l * 3u + 1u]);
        let g = links[l * 3u + 2u];
        ap[a] -= g * p[b];
        ap[b] -= g * p[a];
    }
}
