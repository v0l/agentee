use super::LimitsFile;
use crate::footprint::Mount;
use crate::geom;
use crate::height;
use crate::layout::Layout;
use crate::schematic::Schematic;
use serde::Serialize;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Side {
    Top,
    Bottom,
}

impl Side {
    pub fn other(self) -> Side {
        match self {
            Side::Top => Side::Bottom,
            Side::Bottom => Side::Top,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Side::Top => "top",
            Side::Bottom => "bottom",
        }
    }
}

#[derive(Clone, Debug)]
pub struct PartInput {
    pub reference: String,
    pub value: String,
    pub mpn: Option<String>,
    pub footprint: String,
    pub mount: Mount,
    pub side: Side,
    pub body: Option<[f64; 3]>,
    pub footprint_mm: [f64; 2],
    pub pad_area_mm2: f64,
    pub library: Option<LimitsFile>,
}

pub fn last_word(mpn: &str) -> &str {
    mpn.trim().rsplit(char::is_whitespace).next().unwrap_or("")
}

pub fn gather(
    layout: &Layout,
    schematic: Option<&Schematic>,
    lookup: &dyn Fn(&str, &str) -> Option<LimitsFile>,
) -> Vec<PartInput> {
    let mut out = Vec::new();
    for p in &layout.parts {
        let sch = schematic.and_then(|s| s.parts.iter().find(|q| q.reference == p.reference));
        if sch.is_some_and(|q| q.dnp) {
            continue;
        }
        if p.pads.iter().all(|x| x.copper.is_empty()) {
            continue;
        }
        let mpn = sch
            .and_then(|q| q.fields.get("mpn"))
            .map(|m| m.trim().to_string())
            .filter(|m| !m.is_empty());
        let fp = &p.footprint;
        let fab = height::fab_bounds(fp);
        let pads = height::copper_pads(fp);
        let mut extent = crate::graphic::Bounds::EMPTY;
        for pad in &pads {
            let (lo, hi) = height::pad_box(pad);
            extent.add(lo);
            extent.add(hi);
        }
        let size = fab.map(|b| b.size()).or_else(|| (!extent.is_empty()).then(|| extent.size()));
        let footprint_mm = size.unwrap_or([1.0, 1.0]);
        let body = match (fab, height::body_height(fp)) {
            (Some(b), Some(h)) => {
                let [l, w] = b.size();
                Some([l, w, h])
            }
            _ => None,
        };
        let pad_area_mm2 = p
            .pads
            .iter()
            .filter(|x| !x.paste.is_empty() || x.drill.is_some())
            .flat_map(|x| x.outlines.iter())
            .map(|ring| geom::signed_area(ring).abs())
            .sum();
        let key = mpn.as_deref().unwrap_or("");
        let library = lookup(key, &p.reference);
        out.push(PartInput {
            reference: p.reference.clone(),
            value: p.value.clone(),
            mpn,
            footprint: p.footprint_name.clone(),
            mount: fp.mount,
            side: if p.bottom { Side::Bottom } else { Side::Top },
            body,
            footprint_mm,
            pad_area_mm2,
            library,
        });
    }
    out.sort_by(|a, b| crate::footprint::natural_cmp(&a.reference, &b.reference));
    out
}
