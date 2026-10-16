use agentee_core::footprint::Footprint;
use agentee_core::geom::P;
use serde_json::Value;
use std::path::PathBuf;

const UNIT: f64 = 0.254;

fn trim(v: f64) -> String {
    agentee_core::units::trim(v, 4)
}
const MODELS: &str = "https://modules.easyeda.com/qAxj6KHrDKw4blvCG8QJPs7Y/";

pub struct Found {
    pub lcsc: String,
    pub model: PathBuf,
    pub footprint: Footprint,
}

fn normal(s: &str) -> String {
    s.chars().filter(|c| c.is_ascii_alphanumeric()).collect::<String>().to_ascii_uppercase()
}

pub fn lcsc_code(mpn: &str) -> Result<String, String> {
    let url =
        "https://jlcpcb.com/api/overseas-pcb-order/v1/shoppingCart/smtGood/selectSmtComponentList";
    let body = serde_json::json!({ "keyword": mpn, "currentPage": 1, "pageSize": 25 }).to_string();
    let text = ureq::post(url)
        .header("Content-Type", "application/json")
        .send(body.as_str())
        .map_err(|e| format!("JLCPCB parts search: {e}"))?
        .into_body()
        .read_to_string()
        .map_err(|e| format!("JLCPCB parts search: {e}"))?;
    let v: Value = serde_json::from_str(&text).map_err(|e| format!("JLCPCB parts search: {e}"))?;
    let want = normal(mpn);
    v["data"]["componentPageInfo"]["list"]
        .as_array()
        .into_iter()
        .flatten()
        .find(|c| {
            ["componentModelEn", "erpComponentName"]
                .iter()
                .any(|k| c[*k].as_str().is_some_and(|m| normal(m) == want))
        })
        .and_then(|c| c["componentCode"].as_str())
        .map(str::to_string)
        .ok_or(format!("{mpn} is not in the JLCPCB/LCSC parts list"))
}

fn component(lcsc: &str) -> Result<Value, String> {
    let url = format!("https://easyeda.com/api/products/{lcsc}/components");
    let text = ureq::get(&url)
        .header("User-Agent", "curl/8.0")
        .call()
        .map_err(|e| format!("{url}: {e}"))?
        .into_body()
        .with_config()
        .limit(32 << 20)
        .read_to_string()
        .map_err(|e| format!("{url}: {e}"))?;
    let v: Value = serde_json::from_str(&text).map_err(|e| format!("{url}: {e}"))?;
    if v["success"].as_bool() == Some(false) {
        return Err(format!("EasyEDA has no component for {lcsc}"));
    }
    Ok(v["result"]["packageDetail"]["dataStr"].clone())
}

struct Package {
    pads: Vec<(String, P, [f64; 2])>,
    outline: Vec<P>,
    uuid: String,
    rotate: [f64; 3],
    z: f64,
}

fn num(v: &str) -> Option<f64> {
    v.trim().parse().ok()
}

fn package(data: &Value) -> Result<Package, String> {
    let origin = [
        data["head"]["x"].as_f64().ok_or("the EasyEDA footprint has no origin")?,
        data["head"]["y"].as_f64().ok_or("the EasyEDA footprint has no origin")?,
    ];
    let at = |x: f64, y: f64| [(x - origin[0]) * UNIT, (y - origin[1]) * UNIT];
    let mut pads = Vec::new();
    let mut node = None;
    for s in data["shape"].as_array().into_iter().flatten().filter_map(Value::as_str) {
        if let Some(rest) = s.strip_prefix("PAD~") {
            let f: Vec<&str> = rest.split('~').collect();
            let (Some(x), Some(y), Some(w), Some(h)) = (num(f[1]), num(f[2]), num(f[3]), num(f[4]))
            else {
                continue;
            };
            let number = f.get(7).copied().unwrap_or("").trim().to_string();
            if !number.is_empty() {
                pads.push((number, at(x, y), [w * UNIT, h * UNIT]));
            }
        } else if let Some(rest) = s.strip_prefix("SVGNODE~") {
            let v: Value = serde_json::from_str(rest).map_err(|e| e.to_string())?;
            if v["attrs"]["c_etype"].as_str() == Some("outline3D") {
                node = Some(v);
            }
        }
    }
    let node = node.ok_or("the EasyEDA footprint has no 3D model")?;
    let attrs = &node["attrs"];
    let uuid = attrs["uuid"].as_str().ok_or("the EasyEDA 3D model has no id")?.to_string();
    let rot: Vec<f64> =
        attrs["c_rotation"].as_str().unwrap_or("0,0,0").split(',').filter_map(num).collect();
    let mut outline = Vec::new();
    for c in node["childNodes"].as_array().into_iter().flatten() {
        let v: Vec<f64> = c["attrs"]["points"]
            .as_str()
            .unwrap_or("")
            .split_whitespace()
            .filter_map(num)
            .collect();
        outline.extend(v.chunks_exact(2).map(|p| at(p[0], p[1])));
    }
    Ok(Package {
        pads,
        outline,
        uuid,
        rotate: [
            rot.first().copied().unwrap_or(0.0),
            rot.get(1).copied().unwrap_or(0.0),
            rot.get(2).copied().unwrap_or(0.0),
        ],
        z: attrs["z"].as_str().and_then(num).unwrap_or(0.0) * UNIT,
    })
}

fn download(uuid: &str) -> Result<PathBuf, String> {
    let path = crate::cache_dir().join("easyeda").join(format!("{uuid}.step"));
    if path.is_file() {
        return Ok(path);
    }
    let url = format!("{MODELS}{uuid}");
    let bytes = ureq::get(&url)
        .header("User-Agent", "curl/8.0")
        .call()
        .map_err(|e| format!("{url}: {e}"))?
        .into_body()
        .with_config()
        .limit(128 << 20)
        .read_to_vec()
        .map_err(|e| format!("{url}: {e}"))?;
    if !bytes.starts_with(b"ISO-10303-21") {
        return Err(format!("{url}: not a STEP file"));
    }
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    std::fs::write(&path, bytes).map_err(|e| e.to_string())?;
    Ok(path)
}

pub fn place(
    mesh: &crate::Mesh,
    outline: &[P],
    tilt: [f64; 2],
    turns: &[f64],
) -> Option<([f64; 3], [f64; 2], f64)> {
    let all: Vec<[f64; 3]> =
        mesh.parts.iter().flat_map(|p| p.positions.iter()).map(|v| v.map(f64::from)).collect();
    if all.is_empty() || outline.is_empty() {
        return None;
    }
    let step = (all.len() / 20000).max(1);
    let verts: Vec<[f64; 3]> = all.into_iter().step_by(step).collect();
    let centre = |pts: &mut dyn Iterator<Item = P>| {
        let (mut lo, mut hi) = ([f64::MAX; 2], [f64::MIN; 2]);
        for p in pts {
            lo = [lo[0].min(p[0]), lo[1].min(p[1])];
            hi = [hi[0].max(p[0]), hi[1].max(p[1])];
        }
        [(lo[0] + hi[0]) / 2.0, (lo[1] + hi[1]) / 2.0]
    };
    let target = centre(&mut outline.iter().copied());
    turns
        .iter()
        .map(|&turn| {
            let r = crate::export::rotation([tilt[0], tilt[1], turn]);
            let flat: Vec<P> = verts
                .iter()
                .map(|m| {
                    let q = [0, 1].map(|i| (0..3).map(|k| r[i][k] * m[k]).sum::<f64>());
                    [q[0], -q[1]]
                })
                .collect();
            let c = centre(&mut flat.iter().copied());
            let shift = [target[0] - c[0], target[1] - c[1]];
            let miss = outline
                .iter()
                .map(|o| {
                    flat.iter()
                        .map(|p| {
                            (p[0] + shift[0] - o[0]).powi(2) + (p[1] + shift[1] - o[1]).powi(2)
                        })
                        .fold(f64::MAX, f64::min)
                        .sqrt()
                })
                .sum::<f64>()
                / outline.len() as f64;
            ([tilt[0], tilt[1], turn], shift, miss)
        })
        .min_by(|a, b| a.2.total_cmp(&b.2))
}

pub fn find(mpn: &str) -> Result<Found, String> {
    let lcsc = lcsc_code(mpn)?;
    let data = component(&lcsc)?;
    let pkg = package(&data)?;
    let model = download(&pkg.uuid)?;
    let mesh = crate::load(&model)?;
    let [rx, ry, rz] = pkg.rotate;
    let (rotate, shift, miss) =
        place(&mesh, &pkg.outline, [-rx, -ry], &[-rz, rz]).ok_or("the EasyEDA model is empty")?;
    if miss > 0.3 {
        return Err(format!(
            "the EasyEDA model of {lcsc} does not match its footprint outline ({miss:.2} mm off)"
        ));
    }
    let mut text = format!(
        "name = \"{lcsc}\"\nmodel = \"easyeda.step\"\nmodel_offset = [\"{}mm\", \"{}mm\", \"{}mm\"]\nmodel_rotate = [{}, {}, {}]\n",
        trim(shift[0]),
        trim(-shift[1]),
        trim(pkg.z),
        rotate[0],
        rotate[1],
        rotate[2]
    );
    for (n, at, size) in &pkg.pads {
        text += &format!(
            "\n[[pads]]\nnumber = {n:?}\nkind = \"smd\"\nshape = \"rect\"\nat = [{}, {}]\nsize = [{}, {}]\n",
            at[0],
            at[1],
            size[0].max(0.01),
            size[1].max(0.01)
        );
    }
    let file: agentee_core::footprint::FootprintFile =
        toml::from_str(&text).map_err(|e| format!("the EasyEDA footprint of {lcsc}: {e}"))?;
    let footprint = file.resolve(&mut agentee_core::diag::Diags::new(&lcsc));
    Ok(Found { lcsc, model, footprint })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_footprint_and_model_frame_come_from_the_shape_list() {
        let data: Value = serde_json::json!({
            "head": { "x": 4000, "y": 3000 },
            "shape": [
                "PAD~RECT~3992.126~2988.445~3.7402~6.4961~1~~4~0~~0~gge20",
                "PAD~RECT~4007.874~3011.555~3.7402~6.4961~1~~2~0~~0~gge10",
                "SVGNODE~{\"attrs\":{\"c_etype\":\"outline3D\",\"uuid\":\"abc\",\"c_rotation\":\"0,0,90\",\"z\":\"0\"},\"childNodes\":[{\"attrs\":{\"points\":\"3990 2990 4010 2990 4010 3010\"}}]}"
            ]
        });
        let p = package(&data).unwrap();
        assert_eq!(p.uuid, "abc");
        assert_eq!(p.rotate, [0.0, 0.0, 90.0]);
        assert_eq!(p.pads.len(), 2);
        assert_eq!(p.pads[0].0, "4");
        assert!((p.pads[0].1[0] + 2.0).abs() < 1e-3 && (p.pads[0].1[1] + 2.935).abs() < 1e-3);
        assert!((p.outline[1][0] - 2.54).abs() < 1e-9 && (p.outline[1][1] + 2.54).abs() < 1e-9);
    }

    #[test]
    fn a_turned_model_is_placed_by_its_outline() {
        let mut b = crate::MeshBuilder::default();
        let tri = |p: [[f32; 3]; 3]| p;
        b.push_flat([1.0; 3], tri([[0.0, 0.0, 0.0], [4.0, 0.0, 0.0], [0.0, 1.0, 0.0]]));
        let mesh = b.finish();
        let outline: Vec<P> = vec![[5.0, 1.0], [5.0, 5.0], [6.0, 1.0]];
        let (rot, shift, miss) = place(&mesh, &outline, [0.0, 0.0], &[-90.0, 90.0]).unwrap();
        assert!(miss < 1e-6 && rot[2] == 90.0, "{rot:?} {shift:?} {miss}");
        let r = crate::export::rotation(rot);
        let m = [4.0, 0.0, 0.0];
        let q = [0, 1].map(|i| (0..3).map(|k| r[i][k] * m[k]).sum::<f64>());
        let at = [q[0] + shift[0], -q[1] + shift[1]];
        assert!(outline.iter().any(|o| agentee_core::geom::dist(*o, at) < 1e-6), "{at:?}");
    }
}
