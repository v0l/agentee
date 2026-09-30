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
    sheets: u32,
    base: vec4<u32>,
    omega: vec4<f32>,
    nf: u32,
    plane_k: u32,
    plane_n: u32,
    patches: u32,
    debye: u32,
    pad1: u32,
    pad2: u32,
    pad3: u32,
}

@group(0) @binding(0) var<storage, read_write> e: array<f32>;
@group(0) @binding(8) var<storage, read> e_ro: array<f32>;
@group(0) @binding(9) var<storage, read_write> h: array<f32>;
@group(0) @binding(10) var<storage, read> h_ro: array<f32>;
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
@group(1) @binding(5) var<storage, read_write> plane_acc: array<f32>;
@group(1) @binding(6) var<storage, read> patch_idx: array<u32>;
@group(1) @binding(7) var<storage, read_write> ntff_acc: array<f32>;
@group(1) @binding(8) var<storage, read> branch: array<f32>;
@group(1) @binding(9) var<storage, read_write> sheet: array<f32>;
@group(1) @binding(10) var<storage, read_write> current: array<f32>;
@group(1) @binding(11) var<storage, read_write> debye_edge: array<f32>;
@group(1) @binding(12) var<storage, read> debye_table: array<f32>;
@group(1) @binding(13) var<storage, read_write> debye_state: array<f32>;

const STRIDE: u32 = 10u;

fn psi_base(e: u32, c: u32, a: u32) -> u32 {
    let k = e * 9u + c * 3u + a;
    return P.psi[k / 4u][k % 4u];
}

fn axv(a: u32, i: u32, f: u32) -> f32 {
    return axes[(P.base[a] + i) * STRIDE + f];
}

fn slotv(a: u32, i: u32, f: u32) -> i32 {
    return i32(axes[(P.base[a] + i) * STRIDE + f]);
}

fn psi_at(e: u32, c: u32, a: u32, i: u32, j: u32, k: u32, s: u32) -> u32 {
    let b = psi_base(e, c, a);
    let w = 2u * P.pml;
    if a == 0u { return b + (s * P.n1 + j) * P.n2 + k; }
    if a == 1u { return b + (i * w + s) * P.n2 + k; }
    return b + (i * P.n1 + j) * w + s;
}

fn edge_zone(a: u32, p: u32) -> bool {
    return p < P.pml + 1u || p + P.pml + 2u > dim_of(a);
}

fn dim_of(a: u32) -> u32 {
    return select(select(P.n2, P.n1, a == 1u), P.n0, a == 0u);
}

fn curl_h(c: u32, u: u32, v: u32, id: u32, du: u32, dv: u32, pu: u32, pv: u32, i: u32, j: u32, k: u32) {
    let ev = e_ro[v * P.nn + id + du] - e_ro[v * P.nn + id];
    let eu = e_ro[u * P.nn + id + dv] - e_ro[u * P.nn + id];
    var t1 = ev * axv(u, pu, 0u);
    var t2 = eu * axv(v, pv, 0u);
    if P.pml > 0u && (edge_zone(u, pu) || edge_zone(v, pv)) {
        let su = slotv(u, pu, 9u);
        if su >= 0 {
            let q = psi_at(1u, c, u, i, j, k, u32(su));
            psi[q] = axv(u, pu, 6u) * psi[q] + axv(u, pu, 7u) * ev * axv(u, pu, 2u);
            t1 += psi[q];
        }
        let sv = slotv(v, pv, 9u);
        if sv >= 0 {
            let q = psi_at(1u, c, v, i, j, k, u32(sv));
            psi[q] = axv(v, pv, 6u) * psi[q] + axv(v, pv, 7u) * eu * axv(v, pv, 2u);
            t2 += psi[q];
        }
    }
    h[c * P.nn + id] -= P.k_mu * (t1 - t2);
}

fn curl_e(c: u32, u: u32, v: u32, id: u32, du: u32, dv: u32, pu: u32, pv: u32, i: u32, j: u32, k: u32) {
    let f = c * P.nn + id;
    let cb = coef[(3u + c) * P.nn + id];
    if cb == 0.0 { return; }
    let hv = h_ro[v * P.nn + id] - h_ro[v * P.nn + id - du];
    let hu = h_ro[u * P.nn + id] - h_ro[u * P.nn + id - dv];
    var t1 = hv * axv(u, pu, 1u);
    var t2 = hu * axv(v, pv, 1u);
    if P.pml > 0u && (edge_zone(u, pu) || edge_zone(v, pv)) {
        let su = slotv(u, pu, 8u);
        if su >= 0 {
            let q = psi_at(0u, c, u, i, j, k, u32(su));
            psi[q] = axv(u, pu, 4u) * psi[q] + axv(u, pu, 5u) * hv * axv(u, pu, 3u);
            t1 += psi[q];
        }
        let sv = slotv(v, pv, 8u);
        if sv >= 0 {
            let q = psi_at(0u, c, v, i, j, k, u32(sv));
            psi[q] = axv(v, pv, 4u) * psi[q] + axv(v, pv, 5u) * hu * axv(v, pv, 3u);
            t2 += psi[q];
        }
    }
    e[f] = coef[f] * e[f] + cb * (t1 - t2);
}

@compute @workgroup_size(64, 4, 1)
fn update_h(@builtin(global_invocation_id) g: vec3<u32>) {
    let i = g.z;
    let j = g.y;
    let k = g.x;
    if i >= P.n0 || j >= P.n1 || k >= P.n2 { return; }
    let sx = P.n1 * P.n2;
    let sy = P.n2;
    let id = i * sx + j * sy + k;
    let xi = i + 1u < P.n0;
    let yj = j + 1u < P.n1;
    let zk = k + 1u < P.n2;
    if yj && zk { curl_h(0u, 1u, 2u, id, sy, 1u, j, k, i, j, k); }
    if zk && xi { curl_h(1u, 2u, 0u, id, 1u, sx, k, i, i, j, k); }
    if xi && yj { curl_h(2u, 0u, 1u, id, sx, sy, i, j, i, j, k); }
}

@compute @workgroup_size(64, 4, 1)
fn update_e(@builtin(global_invocation_id) g: vec3<u32>) {
    let i = g.z;
    let j = g.y;
    let k = g.x;
    if i >= P.n0 || j >= P.n1 || k >= P.n2 { return; }
    let sx = P.n1 * P.n2;
    let sy = P.n2;
    let id = i * sx + j * sy + k;
    curl_e(0u, 1u, 2u, id, sy, 1u, j, k, i, j, k);
    curl_e(1u, 2u, 0u, id, 1u, sx, k, i, i, j, k);
    curl_e(2u, 0u, 1u, id, sx, sy, i, j, i, j, k);
}

const SHEET: u32 = 16u;

@compute @workgroup_size(64, 1, 1)
fn sheet_pre(@builtin(global_invocation_id) g: vec3<u32>) {
    let s = g.x;
    if s >= P.sheets { return; }
    let b = s * SHEET;
    sheet[b + 12u] = e[bitcast<u32>(sheet[b])];
    let ha = h_ro[bitcast<u32>(sheet[b + 1u])];
    let hb = h_ro[bitcast<u32>(sheet[b + 2u])];
    let aa = sheet[b + 7u] + ha * ha;
    let bb = sheet[b + 8u] + hb * hb;
    let ab = sheet[b + 9u] + ha * hb;
    sheet[b + 7u] = aa;
    sheet[b + 8u] = bb;
    sheet[b + 9u] = ab;
    let sum = aa + bb;
    if sum <= 1e-30 { return; }
    var scale = 1.0;
    if sheet[b + 6u] != 0.0 {
        scale = sheet[b + 10u] + sheet[b + 11u];
        let total = aa + bb - 2.0 * ab;
        if total > 1e-30 {
            scale = clamp(sum / total, 0.5, 4.0);
        }
    }
    sheet[b + 10u] = scale * aa / sum;
    sheet[b + 11u] = scale * bb / sum;
}

@compute @workgroup_size(64, 1, 1)
fn sheet_post(@builtin(global_invocation_id) g: vec3<u32>) {
    let s = g.x;
    if s >= P.sheets { return; }
    let b = s * SHEET;
    let f = bitcast<u32>(sheet[b]);
    let len = sheet[b + 3u];
    let c = sheet[b + 4u];
    let r = sheet[b + 5u];
    let ma = sheet[b + 10u];
    let mb = sheet[b + 11u];
    let old = sheet[b + 12u];
    let i_old = sheet[b + 13u];
    let nr = u32(branch[0]);
    let nc = u32(branch[1]);
    let stride = 2u + nr + 2u * nc;
    let faces = 4u + 2u * nr + 4u * nc;
    let fa = faces + bitcast<u32>(sheet[b + 14u]) * stride;
    let fb = faces + bitcast<u32>(sheet[b + 15u]) * stride;
    let base = s * (nr + 2u * nc);
    var hist = (ma * branch[fa + 1u] + mb * branch[fb + 1u]) * i_old;
    for (var k = 0u; k < nr; k++) {
        hist -= (ma * branch[fa + 2u + k] + mb * branch[fb + 2u + k]) * current[base + k];
    }
    for (var k = 0u; k < nc; k++) {
        let o = 2u + nr + 2u * k;
        let cr = ma * branch[fa + o] + mb * branch[fb + o];
        let ci = ma * branch[fa + o + 1u] + mb * branch[fb + o + 1u];
        let u = base + nr + 2u * k;
        hist -= cr * current[u] - ci * current[u + 1u];
    }
    let a = r * (ma * branch[fa] + mb * branch[fb]);
    let ibar = (0.5 * len * (e[f] + old) + r * hist) / (a + 0.5 * len * c);
    e[f] = e[f] - c * ibar;
    sheet[b + 13u] = 2.0 * ibar - i_old;
    for (var k = 0u; k < nr; k++) {
        let p = 4u + 2u * k;
        current[base + k] = branch[p] * current[base + k] + branch[p + 1u] * ibar;
    }
    for (var k = 0u; k < nc; k++) {
        let p = 4u + 2u * nr + 4u * k;
        let u = base + nr + 2u * k;
        let ur = current[u];
        let ui = current[u + 1u];
        current[u] = branch[p] * ur - branch[p + 1u] * ui + branch[p + 2u] * ibar;
        current[u + 1u] = branch[p] * ui + branch[p + 1u] * ur + branch[p + 3u] * ibar;
    }
}

fn debye_index(g: vec3<u32>) -> u32 {
    return g.y * 65535u * 64u + g.x;
}

@compute @workgroup_size(64, 1, 1)
fn debye_pre(@builtin(global_invocation_id) g: vec3<u32>) {
    let s = debye_index(g);
    if s >= P.debye { return; }
    debye_edge[s * 3u + 2u] = e[bitcast<u32>(debye_edge[s * 3u])];
}

@compute @workgroup_size(64, 1, 1)
fn debye_post(@builtin(global_invocation_id) g: vec3<u32>) {
    let s = debye_index(g);
    if s >= P.debye { return; }
    let f = bitcast<u32>(debye_edge[s * 3u]);
    let dd = debye_edge[s * 3u + 1u];
    let old = debye_edge[s * 3u + 2u];
    let np = u32(debye_table[0]);
    var s0 = 0.0;
    for (var k = 0u; k < np; k++) {
        s0 += 0.5 * (1.0 + debye_table[1u + 2u * k]) * debye_state[k * P.debye + s];
    }
    let comp = f / P.nn;
    let cb = coef[(3u + comp) * P.nn + (f % P.nn)];
    let next = e[f] - cb * s0;
    e[f] = next;
    let de = next - old;
    for (var k = 0u; k < np; k++) {
        let q = k * P.debye + s;
        debye_state[q] = debye_table[1u + 2u * k] * debye_state[q] + dd * debye_table[2u + 2u * k] * de;
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
        e[comp * P.nn + id] -= source[i * 3u + 1u] * pulse(t);
    }
    if i < P.inductors {
        let b = i * 5u;
        let f = u32(inductor[b]) * P.nn + u32(inductor[b + 1u]);
        let x = e[f] - inductor[b + 2u] * inductor[b + 4u];
        e[f] = x;
        inductor[b + 4u] += inductor[b + 3u] * x;
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
        v -= e_ro[comp * P.nn + u32(port_edges[q])] * port_edges[q + 1u];
    }
    let lf = u32(probe_def[b + 3u]);
    let lc = u32(probe_def[b + 4u]);
    var cur = 0.0;
    for (var c = 0u; c < lc; c++) {
        let q = (lf + c) * 3u;
        cur += h_ro[(u32(loops[q + 1u]) - 3u) * P.nn + u32(loops[q])] * loops[q + 2u];
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
        let x = e_ro[i];
        let y = h_ro[i] * 376.73;
        s += x * x + y * y;
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

@compute @workgroup_size(64, 1, 1)
fn field_dft(@builtin(global_invocation_id) g: vec3<u32>) {
    let q = g.x;
    if q >= P.plane_n { return; }
    let i = q / P.n1;
    let j = q % P.n1;
    let id = (i * P.n1 + j) * P.n2 + P.plane_k;
    let n = state[0];
    let te = (f32(n) + 1.0) * P.dt;
    let th = (f32(n) + 0.5) * P.dt;
    let f = array<f32, 5>(e_ro[id], e_ro[P.nn + id], e_ro[2u * P.nn + id], h_ro[id], h_ro[P.nn + id]);
    for (var k = 0u; k < P.nf; k++) {
        let w = P.omega[k];
        let ce = vec2<f32>(cos(w * te), -sin(w * te)) * P.dt;
        let ch = vec2<f32>(cos(w * th), -sin(w * th)) * P.dt;
        let b = (q * P.nf + k) * 10u;
        for (var c = 0u; c < 5u; c++) {
            let ph = select(ce, ch, c >= 3u);
            plane_acc[b + 2u * c] += f[c] * ph.x;
            plane_acc[b + 2u * c + 1u] += f[c] * ph.y;
        }
    }
}

@compute @workgroup_size(64, 1, 1)
fn ntff(@builtin(global_invocation_id) g: vec3<u32>) {
    if g.x >= P.patches { return; }
    let n = state[0];
    let b = g.x * 13u;
    let meta_ = patch_idx[b];
    let axis = meta_ & 3u;
    let side = select(-1.0, 1.0, (meta_ & 4u) != 0u);
    let u = (axis + 1u) % 3u;
    let v = (axis + 2u) % 3u;
    var ef = array<f32, 3>(0.0, 0.0, 0.0);
    var hf = array<f32, 3>(0.0, 0.0, 0.0);
    ef[u] = 0.5 * (e_ro[u * P.nn + patch_idx[b + 1u]] + e_ro[u * P.nn + patch_idx[b + 2u]]);
    ef[v] = 0.5 * (e_ro[v * P.nn + patch_idx[b + 3u]] + e_ro[v * P.nn + patch_idx[b + 4u]]);
    hf[u] = 0.25 * (h_ro[u * P.nn + patch_idx[b + 5u]] + h_ro[u * P.nn + patch_idx[b + 6u]]
        + h_ro[u * P.nn + patch_idx[b + 7u]] + h_ro[u * P.nn + patch_idx[b + 8u]]);
    hf[v] = 0.25 * (h_ro[v * P.nn + patch_idx[b + 9u]] + h_ro[v * P.nn + patch_idx[b + 10u]]
        + h_ro[v * P.nn + patch_idx[b + 11u]] + h_ro[v * P.nn + patch_idx[b + 12u]]);
    var nrm = array<f32, 3>(0.0, 0.0, 0.0);
    nrm[axis] = side;
    let jv = vec3<f32>(nrm[1] * hf[2] - nrm[2] * hf[1], nrm[2] * hf[0] - nrm[0] * hf[2], nrm[0] * hf[1] - nrm[1] * hf[0]);
    let mv = vec3<f32>(ef[1] * nrm[2] - ef[2] * nrm[1], ef[2] * nrm[0] - ef[0] * nrm[2], ef[0] * nrm[1] - ef[1] * nrm[0]);
    let te = (f32(n) + 1.0) * P.dt;
    let th = (f32(n) + 0.5) * P.dt;
    for (var k = 0u; k < P.nf; k++) {
        let wf = P.omega[k];
        let pe = vec2<f32>(cos(-wf * te), sin(-wf * te)) * P.dt;
        let ph = vec2<f32>(cos(-wf * th), sin(-wf * th)) * P.dt;
        let o = (g.x * P.nf + k) * 12u;
        for (var c = 0u; c < 3u; c++) {
            ntff_acc[o + 2u * c] += ph.x * jv[c];
            ntff_acc[o + 2u * c + 1u] += ph.y * jv[c];
            ntff_acc[o + 6u + 2u * c] += pe.x * mv[c];
            ntff_acc[o + 6u + 2u * c + 1u] += pe.y * mv[c];
        }
    }
}
