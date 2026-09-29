struct Params {
    n0: u32,
    n1: u32,
    n2: u32,
    nn: u32,
    pml: u32,
    sources: u32,
    inductors: u32,
    ports: u32,
    psi: array<vec4<u32>, 5>,
    dt: f32,
    k_mu: f32,
    t0: f32,
    tau: f32,
    w0: f32,
    cap: u32,
    probes: u32,
    pad: u32,
}

@group(0) @binding(0) var<storage, read_write> fields: array<f32>;
@group(0) @binding(1) var<storage, read> coef: array<f32>;
@group(0) @binding(2) var<storage, read_write> psi: array<f32>;
@group(0) @binding(3) var<storage, read> axes: array<f32>;
@group(0) @binding(4) var<uniform> P: Params;
@group(0) @binding(5) var<storage, read_write> state: array<u32>;
@group(0) @binding(6) var<storage, read_write> series: array<f32>;
@group(0) @binding(7) var<storage, read> source: array<f32>;
@group(1) @binding(0) var<storage, read_write> inductor: array<f32>;
@group(1) @binding(1) var<storage, read> probe_def: array<f32>;
@group(1) @binding(2) var<storage, read_write> partial: array<f32>;
@group(1) @binding(3) var<storage, read> port_edges: array<f32>;
@group(1) @binding(4) var<storage, read> loops: array<f32>;

const STRIDE: u32 = 10u;

fn dim(a: u32) -> u32 {
    if a == 0u { return P.n0; }
    if a == 1u { return P.n1; }
    return P.n2;
}

fn base(a: u32) -> u32 {
    if a == 0u { return 0u; }
    if a == 1u { return P.n0; }
    return P.n0 + P.n1;
}

fn ax(a: u32, i: u32, f: u32) -> f32 {
    return axes[(base(a) + i) * STRIDE + f];
}

fn slot(a: u32, i: u32, f: u32) -> i32 {
    return i32(ax(a, i, f));
}

fn idx(p: vec3<u32>) -> u32 {
    return (p.x * P.n1 + p.y) * P.n2 + p.z;
}

fn psi_base(e: u32, c: u32, a: u32) -> u32 {
    let k = e * 9u + c * 3u + a;
    return P.psi[k / 4u][k % 4u];
}

fn psi_idx(a: u32, p: vec3<u32>, s: u32) -> u32 {
    let w = 2u * P.pml;
    if a == 0u { return (s * P.n1 + p.y) * P.n2 + p.z; }
    if a == 1u { return (p.x * w + s) * P.n2 + p.z; }
    return (p.x * P.n1 + p.y) * w + s;
}

fn comp(p: vec3<u32>, a: u32) -> u32 {
    if a == 0u { return p.x; }
    if a == 1u { return p.y; }
    return p.z;
}

fn step_along(p: vec3<u32>, a: u32, d: i32) -> vec3<u32> {
    var q = vec3<i32>(p);
    if a == 0u { q.x += d; }
    if a == 1u { q.y += d; }
    if a == 2u { q.z += d; }
    return vec3<u32>(q);
}

@compute @workgroup_size(64, 4, 1)
fn update_h(@builtin(global_invocation_id) g: vec3<u32>) {
    let p = vec3<u32>(g.z, g.y, g.x);
    if p.x >= P.n0 || p.y >= P.n1 || p.z >= P.n2 { return; }
    let id = idx(p);
    for (var c = 0u; c < 3u; c++) {
        let u = (c + 1u) % 3u;
        let v = (c + 2u) % 3u;
        let pu = comp(p, u);
        let pv = comp(p, v);
        if pu + 1u >= dim(u) || pv + 1u >= dim(v) { continue; }
        let ev = fields[v * P.nn + idx(step_along(p, u, 1))] - fields[v * P.nn + id];
        let eu = fields[u * P.nn + idx(step_along(p, v, 1))] - fields[u * P.nn + id];
        var t1 = ev * ax(u, pu, 0u);
        var t2 = eu * ax(v, pv, 0u);
        if P.pml > 0u {
            let su = slot(u, pu, 9u);
            if su >= 0 {
                let q = psi_base(1u, c, u) + psi_idx(u, p, u32(su));
                psi[q] = ax(u, pu, 6u) * psi[q] + ax(u, pu, 7u) * ev * ax(u, pu, 2u);
                t1 += psi[q];
            }
            let sv = slot(v, pv, 9u);
            if sv >= 0 {
                let q = psi_base(1u, c, v) + psi_idx(v, p, u32(sv));
                psi[q] = ax(v, pv, 6u) * psi[q] + ax(v, pv, 7u) * eu * ax(v, pv, 2u);
                t2 += psi[q];
            }
        }
        fields[(3u + c) * P.nn + id] -= P.k_mu * (t1 - t2);
    }
}

@compute @workgroup_size(64, 4, 1)
fn update_e(@builtin(global_invocation_id) g: vec3<u32>) {
    let p = vec3<u32>(g.z, g.y, g.x);
    if p.x >= P.n0 || p.y >= P.n1 || p.z >= P.n2 { return; }
    let id = idx(p);
    for (var c = 0u; c < 3u; c++) {
        let cb = coef[(3u + c) * P.nn + id];
        if cb == 0.0 { continue; }
        let u = (c + 1u) % 3u;
        let v = (c + 2u) % 3u;
        let pu = comp(p, u);
        let pv = comp(p, v);
        let hv = fields[(3u + v) * P.nn + id] - fields[(3u + v) * P.nn + idx(step_along(p, u, -1))];
        let hu = fields[(3u + u) * P.nn + id] - fields[(3u + u) * P.nn + idx(step_along(p, v, -1))];
        var t1 = hv * ax(u, pu, 1u);
        var t2 = hu * ax(v, pv, 1u);
        if P.pml > 0u {
            let su = slot(u, pu, 8u);
            if su >= 0 {
                let q = psi_base(0u, c, u) + psi_idx(u, p, u32(su));
                psi[q] = ax(u, pu, 4u) * psi[q] + ax(u, pu, 5u) * hv * ax(u, pu, 3u);
                t1 += psi[q];
            }
            let sv = slot(v, pv, 8u);
            if sv >= 0 {
                let q = psi_base(0u, c, v) + psi_idx(v, p, u32(sv));
                psi[q] = ax(v, pv, 4u) * psi[q] + ax(v, pv, 5u) * hu * ax(v, pv, 3u);
                t2 += psi[q];
            }
        }
        let f = c * P.nn + id;
        fields[f] = coef[f] * fields[f] + cb * (t1 - t2);
    }
}

fn pulse(t: f32) -> f32 {
    let x = (t - P.t0) / P.tau;
    return exp(-x * x) * cos(P.w0 * (t - P.t0));
}

@compute @workgroup_size(64, 1, 1)
fn lumped(@builtin(global_invocation_id) g: vec3<u32>) {
    let i = g.x;
    if i < P.sources {
        let n = state[0];
        let t = (f32(n) + 0.5) * P.dt;
        let id = u32(source[i * 3u]);
        let comp = u32(source[i * 3u + 2u]);
        fields[comp * P.nn + id] -= source[i * 3u + 1u] * pulse(t);
    }
    if i < P.inductors {
        let b = i * 5u;
        let f = u32(inductor[b]) * P.nn + u32(inductor[b + 1u]);
        let e = fields[f] - inductor[b + 2u] * inductor[b + 4u];
        fields[f] = e;
        inductor[b + 4u] += inductor[b + 3u] * e;
    }
}

@compute @workgroup_size(64, 1, 1)
fn probe(@builtin(global_invocation_id) g: vec3<u32>) {
    let k = g.x;
    if k >= P.ports { return; }
    let n = state[0];
    let b = k * 8u;
    let comp = u32(probe_def[b]);
    let first = u32(probe_def[b + 1u]);
    let count = u32(probe_def[b + 2u]);
    var v = 0.0;
    for (var e = 0u; e < count; e++) {
        let q = (first + e) * 2u;
        v -= fields[comp * P.nn + u32(port_edges[q])] * port_edges[q + 1u];
    }
    let lf = u32(probe_def[b + 3u]);
    let lc = u32(probe_def[b + 4u]);
    let hu = 3u + u32(probe_def[b + 5u]);
    let hv = 3u + u32(probe_def[b + 6u]);
    var cur = 0.0;
    for (var c = 0u; c < lc; c++) {
        let q = (lf + c) * 5u;
        let id = u32(loops[q]);
        let iu = u32(loops[q + 1u]);
        let iv = u32(loops[q + 2u]);
        cur += (fields[hv * P.nn + id] - fields[hv * P.nn + iu]) * loops[q + 3u]
            - (fields[hu * P.nn + id] - fields[hu * P.nn + iv]) * loops[q + 4u];
    }
    if n < P.cap {
        series[(n * P.ports + k) * 2u] = v;
        series[(n * P.ports + k) * 2u + 1u] = cur;
    }
}

@compute @workgroup_size(1, 1, 1)
fn tick() {
    state[0] = state[0] + 1u;
}

var<workgroup> scratch: array<f32, 256>;

@compute @workgroup_size(256, 1, 1)
fn energy(@builtin(global_invocation_id) g: vec3<u32>, @builtin(local_invocation_id) l: vec3<u32>, @builtin(workgroup_id) w: vec3<u32>) {
    var s = 0.0;
    let total = 3u * P.nn;
    let stride = 256u * 1024u;
    var i = w.x * 256u + l.x;
    while i < total {
        let x = fields[i];
        s += x * x;
        i += stride;
    }
    scratch[l.x] = s;
    workgroupBarrier();
    for (var k = 128u; k > 0u; k /= 2u) {
        if l.x < k { scratch[l.x] += scratch[l.x + k]; }
        workgroupBarrier();
    }
    if l.x == 0u { partial[w.x] = scratch[0]; }
}
