use std::path::{Path, PathBuf};

fn ep_split(stem: &str) -> Option<(&str, [f64; 2], &str)> {
    let i = stem.rfind("_EP")?;
    let rest = &stem[i + 3..];
    let end = rest.find("mm")?;
    let (a, b) = rest[..end].split_once('x')?;
    Some((&stem[..i], [a.parse().ok()?, b.parse().ok()?], &rest[end + 2..]))
}

fn stem(name: &str) -> &str {
    let lower = name.to_ascii_lowercase();
    match [".step", ".stp", ".wrl"].iter().find(|e| lower.ends_with(*e)) {
        Some(e) => &name[..name.len() - e.len()],
        None => name,
    }
}

pub fn pick(wanted: &str, names: &[String]) -> Option<String> {
    let want = stem(wanted).replace("_ThermalVias", "");
    let wrl = |n: &str| n.ends_with(".wrl");
    let models: Vec<&String> =
        names.iter().filter(|n| n.ends_with(".step") || n.ends_with(".stp") || wrl(n)).collect();
    if let Some(n) = models.iter().filter(|n| stem(n) == want).min_by_key(|n| wrl(n)) {
        return Some(n.to_string());
    }
    let (body, ep, tail) = ep_split(&want)?;
    models
        .into_iter()
        .filter_map(|n| {
            let (b, e, t) = ep_split(stem(n))?;
            let rank = if t != tail {
                return None;
            } else if b == body {
                0
            } else if b.ends_with(&format!("_{body}")) {
                1
            } else {
                return None;
            };
            let off = ((e[0] - ep[0]).abs() + (e[1] - ep[1]).abs()) * 1000.0;
            Some(((rank, off.round() as i64, wrl(n)), n))
        })
        .min_by_key(|(k, _)| *k)
        .map(|(_, n)| n.clone())
}

fn listing_file(dir: &str) -> PathBuf {
    crate::cache_dir().join("listing").join(format!("{dir}.txt"))
}

fn local_names(dir: &str, roots: &[PathBuf]) -> Vec<String> {
    roots
        .iter()
        .filter_map(|r| std::fs::read_dir(r.join(dir)).ok())
        .flatten()
        .filter_map(|e| e.ok()?.file_name().into_string().ok())
        .collect()
}

fn names_in(text: &str, key: &str) -> Vec<String> {
    let mut out = Vec::new();
    let needle = format!("\"{key}\":\"");
    let mut rest = text;
    while let Some(i) = rest.find(&needle) {
        rest = &rest[i + needle.len()..];
        if let Some(j) = rest.find('"') {
            out.push(rest[..j].to_string());
            rest = &rest[j..];
        }
    }
    out
}

fn get_text(url: &str) -> Option<String> {
    let resp = ureq::get(url).header("User-Agent", "agentee").call().ok()?;
    if resp.status() != 200 {
        return None;
    }
    resp.into_body().with_config().limit(16 << 20).read_to_string().ok()
}

fn remote_names(dir: &str) -> Vec<String> {
    let mut out = Vec::new();
    for page in 1..=20 {
        let url = format!(
            "https://gitlab.com/api/v4/projects/kicad%2Flibraries%2Fkicad-packages3D/repository/tree?path={dir}&per_page=100&page={page}"
        );
        let Some(text) = get_text(&url) else { break };
        let names = names_in(&text, "name");
        let last = names.len() < 100;
        out.extend(names);
        if last {
            break;
        }
    }
    let url = format!("https://api.github.com/repos/KiCad/kicad-packages3D/contents/{dir}");
    if let Some(text) = get_text(&url) {
        out.extend(names_in(&text, "name"));
    }
    out
}

fn known_names(dir: &str, roots: &[PathBuf], online: bool) -> Vec<String> {
    let mut names = local_names(dir, roots);
    let cached = listing_file(dir);
    match std::fs::read_to_string(&cached) {
        Ok(text) => names.extend(text.lines().map(String::from)),
        Err(_) if online => {
            let remote = remote_names(dir);
            if !remote.is_empty() {
                if let Some(parent) = cached.parent() {
                    let _ = std::fs::create_dir_all(parent);
                }
                let _ = std::fs::write(&cached, remote.join("\n"));
            }
            names.extend(remote);
        }
        Err(_) => {}
    }
    names.sort();
    names.dedup();
    names
}

pub fn nearest(model: &str, project: &Path, online: bool) -> Option<String> {
    let rel = crate::relative(model);
    let (dir, file) = rel.rsplit_once('/')?;
    if !dir.ends_with(".3dshapes") {
        return None;
    }
    let names = known_names(dir, &crate::roots(project), online);
    let found = pick(file, &names)?;
    (stem(&found) != stem(file)).then(|| format!("{dir}/{found}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn a_missing_pad_size_takes_the_nearest_body_of_the_same_package() {
        let have = names(&[
            "VQFN-24-1EP_4x4mm_P0.5mm_EP2.45x2.45mm.wrl",
            "VQFN-24-1EP_4x4mm_P0.5mm_EP2.45x2.45mm.step",
            "Texas_RGE0024H_VQFN-24-1EP_4x4mm_P0.5mm_EP2.7x2.7mm.step",
            "HVQFN-24-1EP_4x4mm_P0.5mm_EP2.1x2.1mm.step",
            "VQFN-24-1EP_4x5mm_P0.5mm_EP2.5x2.5mm.step",
        ]);
        let got = |w: &str| pick(w, &have);
        assert_eq!(
            got("VQFN-24-1EP_4x4mm_P0.5mm_EP2.5x2.5mm.wrl").as_deref(),
            Some("VQFN-24-1EP_4x4mm_P0.5mm_EP2.45x2.45mm.step")
        );
        let ti = names(&["Texas_RGE0024H_VQFN-24-1EP_4x4mm_P0.5mm_EP2.7x2.7mm.step"]);
        assert_eq!(
            pick("VQFN-24-1EP_4x4mm_P0.5mm_EP2.6x2.6mm.step", &ti).as_deref(),
            Some("Texas_RGE0024H_VQFN-24-1EP_4x4mm_P0.5mm_EP2.7x2.7mm.step")
        );
        assert_eq!(got("VQFN-28-1EP_5x5mm_P0.5mm_EP3.7x3.7mm.step"), None);
        assert_eq!(
            got("VQFN-24-1EP_4x4mm_P0.5mm_EP2.45x2.45mm_ThermalVias.step").as_deref(),
            Some("VQFN-24-1EP_4x4mm_P0.5mm_EP2.45x2.45mm.step")
        );
    }
}
