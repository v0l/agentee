use crate::canvas::{View, cursor_readout};
use crate::paint::{self, Layers, SymbolStyle, Xf};
use crate::{pcb, sheet};
use agentee_core::board::{Board, LayerKind, Outline};
use agentee_core::calc::TraceGeometry;
use agentee_core::footprint::{Drill, Footprint, natural_cmp};
use agentee_core::layout::Layout;
use agentee_core::project::{ItemRef, Project};
use agentee_core::schematic::Schematic;
use agentee_core::symbol::{PinType, Side, Symbol};
use agentee_core::units::{Length, trim};
use agentee_core::{Diagnostic, Severity};
use egui::{Color32, Pos2, Rect, Sense, Stroke, Ui, Vec2};
use egui_bench::prelude::*;

pub struct PageState {
    pub item: Option<ItemRef>,
    pub view: View,
    pub unit: u32,
    pub layers: Layers,
    pub show_hidden: bool,
    pub interactive: bool,
    pub panels: bool,
    pub pcb_layers: Layers,
    pub ratsnest: bool,
    pub zone_key: Option<(u64, usize)>,
    pub zone_tex: Vec<egui::TextureHandle>,
    pub region: Option<agentee_core::graphic::Bounds>,
    pub hidden_curves: Vec<(usize, usize)>,
    pub sim_progress: Option<agentee_core::sim::SimProgress>,
    pub map_index: usize,
    pub map_key: Option<(u64, usize, usize)>,
    pub map_tex: Option<egui::TextureHandle>,
    pub map_values: Vec<f32>,
    pub show_fields: bool,
    pub show_tdr: bool,
    pub view_3d: bool,
    pub camera: crate::board3d::Camera,
    pub scene: Option<((u64, usize, u64), std::sync::Arc<crate::board3d::Scene>)>,
    pub show_parts: bool,
    pub soft_3d: crate::board3d::SoftCache,
    pub tdr_cache: Option<((u64, usize), Vec<crate::plot::Series>)>,
    pub runs: crate::simrun::Runs,
}

impl Default for PageState {
    fn default() -> Self {
        PageState {
            item: None,
            view: View::default(),
            unit: 1,
            layers: Layers::default(),
            show_hidden: false,
            interactive: true,
            panels: true,
            pcb_layers: crate::pcb::default_layers(),
            ratsnest: true,
            zone_key: None,
            zone_tex: Vec::new(),
            region: None,
            hidden_curves: Vec::new(),
            sim_progress: None,
            map_index: 0,
            map_key: None,
            map_tex: None,
            map_values: Vec::new(),
            show_fields: false,
            show_tdr: false,
            view_3d: false,
            camera: Default::default(),
            scene: None,
            show_parts: true,
            soft_3d: None,
            tdr_cache: None,
            runs: Default::default(),
        }
    }
}

impl PageState {
    pub fn select(&mut self, item: ItemRef) {
        if self.item != Some(item) {
            self.item = Some(item);
            self.view = View::default();
            self.unit = 1;
        }
    }
}

pub const SIDE_W: f32 = 400.0;

pub fn page(ui: &mut Ui, project: &Project, item: ItemRef, st: &mut PageState) {
    st.select(item);
    let diags = project.diags_of(item);
    if !st.panels {
        egui::CentralPanel::no_frame().show(ui, |ui| match item {
            ItemRef::Symbol(i) => symbol_canvas(ui, &project.symbols[i].item, st),
            ItemRef::Footprint(i) => footprint_canvas(ui, &project.footprints[i].item, st),
            ItemRef::Schematic(i) => schematic_canvas(ui, &project.schematics[i].item, st),
            ItemRef::Layout(i) => layout_canvas(ui, project, i, st),
            ItemRef::Sim(i) => sim_canvas(ui, project, i, st),
            ItemRef::Board(i) => {
                egui::Frame::NONE.inner_margin(egui::Margin::symmetric(12, 10)).show(ui, |ui| {
                    board_sheet(ui, &project.boards[i].item, st);
                });
            }
        });
        return;
    }
    egui::Panel::bottom("diagnostics")
        .resizable(st.interactive)
        .default_size(150.0)
        .frame(egui::Frame::NONE.fill(CHASSIS).inner_margin(egui::Margin::symmetric(10, 6)))
        .show(ui, |ui| diagnostics(ui, diags, st.interactive));
    egui::Panel::right("properties")
        .resizable(st.interactive)
        .default_size(SIDE_W)
        .min_size(340.0)
        .frame(egui::Frame::NONE.fill(CHASSIS).inner_margin(egui::Margin::symmetric(10, 8)))
        .show(ui, |ui| {
            scroll(ui, st.interactive, "props", |ui| match item {
                ItemRef::Symbol(i) => symbol_props(ui, project, &project.symbols[i].item, st),
                ItemRef::Footprint(i) => footprint_props(ui, &project.footprints[i].item, st),
                ItemRef::Board(i) => board_props(ui, &project.boards[i].item),
                ItemRef::Schematic(i) => schematic_props(ui, &project.schematics[i].item),
                ItemRef::Layout(i) => layout_props(ui, &project.layouts[i].item, st),
                ItemRef::Sim(i) => sim_props(ui, project, &project.sims[i].item, st),
            });
        });
    egui::CentralPanel::no_frame().show(ui, |ui| match item {
        ItemRef::Symbol(i) => symbol_canvas(ui, &project.symbols[i].item, st),
        ItemRef::Footprint(i) => footprint_canvas(ui, &project.footprints[i].item, st),
        ItemRef::Schematic(i) => schematic_canvas(ui, &project.schematics[i].item, st),
        ItemRef::Layout(i) => layout_canvas(ui, project, i, st),
        ItemRef::Sim(i) => sim_canvas(ui, project, i, st),
        ItemRef::Board(i) => {
            egui::Frame::NONE.inner_margin(egui::Margin::symmetric(12, 10)).show(ui, |ui| {
                scroll(ui, st.interactive, "board", |ui| {
                    board_sheet(ui, &project.boards[i].item, st)
                })
            });
        }
    });
}

fn scroll(ui: &mut Ui, interactive: bool, id: &str, body: impl FnOnce(&mut Ui)) {
    if interactive && id == "board" {
        egui::ScrollArea::both().id_salt(id).auto_shrink([false, false]).show(ui, |ui| {
            ui.set_min_width(760.0);
            body(ui)
        });
    } else if interactive {
        egui::ScrollArea::vertical().id_salt(id).auto_shrink([false, false]).show(ui, body);
    } else {
        body(ui);
    }
}

fn severity_color(s: Severity) -> Color32 {
    match s {
        Severity::Error => FAULT,
        Severity::Warning => WARN,
        Severity::Info => LEGEND,
    }
}

pub fn diagnostics(ui: &mut Ui, diags: &[Diagnostic], interactive: bool) {
    let e = diags.iter().filter(|d| d.severity == Severity::Error).count();
    let w = diags.iter().filter(|d| d.severity == Severity::Warning).count();
    ui.horizontal(|ui| {
        Line::new().legend("check").show(ui);
        lamp(ui, &format!("{e} errors"), e == 0, e > 0);
        lamp(ui, &format!("{w} warnings"), w == 0, false);
    });
    ui.add_space(4.0);
    scroll(ui, interactive, "diags", |ui| {
        if diags.is_empty() {
            status(ui, true, "nothing to report");
        }
        let mut sorted: Vec<&Diagnostic> = diags.iter().collect();
        sorted.sort_by(|a, b| b.severity.cmp(&a.severity));
        for d in sorted {
            ui.horizontal(|ui| {
                let (rect, _) = ui.allocate_exact_size(Vec2::new(10.0, 16.0), Sense::hover());
                ui.painter().circle_filled(rect.center(), 3.0, severity_color(d.severity));
                let mut line = Line::new();
                if !d.at.is_empty() {
                    line = line.legend(&d.at);
                }
                line.value(&d.message)
                    .tint(severity_color(d.severity).lerp_to_gamma(VALUE, 0.45))
                    .size(11.5)
                    .wrapped(ui);
            });
        }
    });
}

fn symbol_canvas(ui: &mut Ui, s: &Symbol, st: &mut PageState) {
    let b = s.bounds(st.unit);
    let mut padded = b;
    if !b.is_empty() {
        padded.add([b.min[0], b.min[1] - 2.5]);
        padded.add([b.max[0], b.max[1] + 2.5]);
    }
    st.view.max_fit = if st.region.is_some() { 4000.0 } else { 45.0 };
    let (resp, xf) = st.view.show(ui, &st.region.unwrap_or(padded), 40.0);
    let p = ui.painter_at(xf.rect);
    paint::grid(&p, &xf, 1.27);
    let hover = if st.interactive { resp.hover_pos() } else { None };
    let reference = format!("{}?{}", s.reference, s.unit_label(st.unit));
    let style = SymbolStyle {
        show_hidden: st.show_hidden,
        reference,
        value: s.value.clone(),
        dim: false,
        tips: true,
    };
    let hit = paint::symbol(&p, &xf, s, st.unit, &style, hover);
    cursor_readout(ui, &xf, hover);
    if let Some(i) = hit {
        let pin = &s.pins[i];
        resp.on_hover_ui_at_pointer(|ui| {
            Line::new().legend("pin").set(&pin.number).value(&pin.name).show(ui);
            reading(ui, "type", pin_type(pin.kind).to_string());
            reading(ui, "at", pin.at.to_string());
            reading(ui, "side", side(pin.side));
        });
    }
}

fn footprint_canvas(ui: &mut Ui, fp: &Footprint, st: &mut PageState) {
    st.view.max_fit = 2000.0;
    let (resp, xf) = st.view.show(ui, &st.region.unwrap_or(fp.bounds()), 40.0);
    let p = ui.painter_at(xf.rect);
    paint::grid(&p, &xf, 0.5);
    let hover = if st.interactive { resp.hover_pos() } else { None };
    let hit = paint::footprint(&p, &xf, fp, &st.layers, hover);
    scale_bar(&p, &xf);
    cursor_readout(ui, &xf, hover);
    if let Some(i) = hit {
        let pad = &fp.pads[i];
        resp.on_hover_ui_at_pointer(|ui| {
            Line::new()
                .legend("pad")
                .set(if pad.number.is_empty() { "-" } else { &pad.number })
                .show(ui);
            reading(ui, "kind", format!("{:?} {:?}", pad.kind, pad.shape).to_lowercase());
            reading(ui, "at", pad.at.to_string());
            reading(ui, "size", pad.size.to_string());
            if let Some(d) = pad.drill {
                reading(ui, "drill", drill(d));
            }
            reading(ui, "layers", pad.layers.join(" "));
        });
    }
}

fn scale_bar(p: &egui::Painter, xf: &Xf) {
    let target = 90.0 / xf.scale as f64;
    let steps = [0.1, 0.2, 0.5, 1.0, 2.0, 5.0, 10.0, 20.0, 50.0, 100.0];
    let mm = steps.iter().copied().find(|s| *s >= target).unwrap_or(100.0);
    let w = xf.len(mm);
    let r = xf.rect;
    let y = r.bottom() - 10.0;
    let x1 = r.right() - 14.0;
    let x0 = x1 - w;
    let s = Stroke::new(1.0, LEGEND);
    p.line_segment([Pos2::new(x0, y), Pos2::new(x1, y)], s);
    p.line_segment([Pos2::new(x0, y - 4.0), Pos2::new(x0, y + 1.0)], s);
    p.line_segment([Pos2::new(x1, y - 4.0), Pos2::new(x1, y + 1.0)], s);
    p.text(
        Pos2::new((x0 + x1) / 2.0, y - 5.0),
        egui::Align2::CENTER_BOTTOM,
        format!("{} mm", trim(mm, 2)),
        figure(10.5),
        LEGEND,
    );
}

fn pin_type(t: PinType) -> &'static str {
    match t {
        PinType::Input => "input",
        PinType::Output => "output",
        PinType::Bidirectional => "bidirectional",
        PinType::TriState => "tri-state",
        PinType::Passive => "passive",
        PinType::Free => "free",
        PinType::Unspecified => "unspecified",
        PinType::PowerIn => "power in",
        PinType::PowerOut => "power out",
        PinType::OpenCollector => "open collector",
        PinType::OpenEmitter => "open emitter",
        PinType::NoConnect => "no connect",
    }
}

fn side(s: Side) -> &'static str {
    match s {
        Side::Left => "left",
        Side::Right => "right",
        Side::Top => "top",
        Side::Bottom => "bottom",
    }
}

fn drill(d: Drill) -> String {
    match d {
        Drill::Round(l) => format!("{l}"),
        Drill::Slot(p) => format!("{} x {} slot", p.0, p.1),
    }
}

fn mm(l: Length) -> String {
    trim(l.to_mm(), 3)
}

fn symbol_props(ui: &mut Ui, project: &Project, s: &Symbol, st: &mut PageState) {
    card(
        ui,
        Some(READOUT),
        |ui| {
            Line::new().legend("symbol").value(&s.name).elided(ui);
        },
        |ui| {
            readouts(
                ui,
                &[
                    ("reference", s.reference.clone(), READOUT),
                    ("pins", s.pins.len().to_string(), VALUE),
                    ("units", s.units.to_string(), VALUE),
                ],
            );
            if !s.description.is_empty() {
                note(ui, &s.description, VALUE);
            }
            if let Some(fp) = &s.footprint {
                let known = project.footprint(fp).is_some();
                reading(
                    ui,
                    "footprint",
                    if known { fp.clone() } else { format!("{fp} (not in project)") },
                );
            }
            if !s.footprint_filters.is_empty() {
                reading(ui, "fp filters", s.footprint_filters.join(" "));
            }
            if !s.keywords.is_empty() {
                reading(ui, "keywords", s.keywords.join(" "));
            }
            if !s.datasheet.is_empty() {
                reading(ui, "datasheet", s.datasheet.clone());
            }
        },
    );
    ui.add_space(8.0);
    if s.units > 1 {
        let opts: Vec<(u32, String)> =
            (1..=s.units).map(|u| (u, format!("unit {}", s.unit_label(u)))).collect();
        let refs: Vec<(u32, &str)> = opts.iter().map(|(u, l)| (*u, l.as_str())).collect();
        tabs(ui, &mut st.unit, &refs);
    }
    if s.pins.iter().any(|p| p.hidden) && toggle(ui, "hidden pins", st.show_hidden).clicked() {
        st.show_hidden = !st.show_hidden;
    }
    ui.add_space(4.0);
    let mut pins: Vec<_> = s.pins.iter().filter(|p| p.in_unit(st.unit)).collect();
    pins.sort_by(|a, b| natural_cmp(&a.number, &b.number));
    let cols = [("#", 40.0), ("name", 104.0), ("type", 96.0), ("side", 50.0), ("at", 84.0)];
    Table::new(&cols, pins.len()).show(ui, |i, p, r, at| {
        let pin = pins[i];
        let dim = if pin.hidden { 0.5 } else { 1.0 };
        cell(p, r, at(0), cols[0].1, &pin.number, READOUT.gamma_multiply(dim));
        cell(
            p,
            r,
            at(1),
            cols[1].1,
            &pin.name.replace("~{", "/").replace('}', ""),
            VALUE.gamma_multiply(dim),
        );
        cell(p, r, at(2), cols[2].1, pin_type(pin.kind), LEGEND);
        cell(p, r, at(3), cols[3].1, side(pin.side), LEGEND);
        cell(p, r, at(4), cols[4].1, &format!("{} {}", mm(pin.at.0), mm(pin.at.1)), LEGEND);
    });
}

fn footprint_props(ui: &mut Ui, fp: &Footprint, st: &mut PageState) {
    let body = fp.bounds();
    let cy = fp.courtyard("F");
    let size = if cy.is_empty() { body.size() } else { cy.size() };
    card(
        ui,
        Some(READOUT),
        |ui| {
            Line::new().legend("footprint").value(&fp.name).elided(ui);
        },
        |ui| {
            readouts(
                ui,
                &[
                    ("pads", fp.pad_numbers().len().to_string(), VALUE),
                    ("mount", format!("{:?}", fp.mount).to_uppercase(), READOUT),
                    ("courtyard", format!("{} x {}", trim(size[0], 2), trim(size[1], 2)), TRACE),
                ],
            );
            if !fp.description.is_empty() {
                note(ui, &fp.description, VALUE);
            }
            if !fp.tags.is_empty() {
                reading(ui, "tags", fp.tags.join(" "));
            }
            if let Some(m) = &fp.model {
                reading(ui, "3d model", m.clone());
            }
        },
    );
    ui.add_space(8.0);
    Line::new().legend("layers").show(ui);
    let mut present: Vec<&str> = paint::LAYER_ORDER
        .iter()
        .copied()
        .filter(|l| {
            fp.graphics.iter().any(|g| g.layer == *l) || fp.pads.iter().any(|p| p.on_layer(l))
        })
        .collect();
    present.reverse();
    ui.horizontal_wrapped(|ui| {
        for l in present {
            if toggle(ui, l, st.layers.shows(l)).clicked() {
                st.layers.toggle(l);
            }
        }
    });
    ui.add_space(6.0);
    let mut pads: Vec<_> = fp.pads.iter().collect();
    pads.sort_by(|a, b| natural_cmp(&a.number, &b.number));
    let cols = [("#", 34.0), ("kind", 104.0), ("at", 100.0), ("size", 80.0), ("drill", 50.0)];
    Table::new(&cols, pads.len()).show(ui, |i, p, r, at| {
        let pad = pads[i];
        cell(
            p,
            r,
            at(0),
            cols[0].1,
            if pad.number.is_empty() { "-" } else { &pad.number },
            READOUT,
        );
        cell(
            p,
            r,
            at(1),
            cols[1].1,
            &format!("{:?} {:?}", pad.kind, pad.shape).to_lowercase(),
            LEGEND,
        );
        cell(p, r, at(2), cols[2].1, &format!("{} {}", mm(pad.at.0), mm(pad.at.1)), VALUE);
        cell(p, r, at(3), cols[3].1, &format!("{}x{}", mm(pad.size.0), mm(pad.size.1)), VALUE);
        cell(p, r, at(4), cols[4].1, &pad.drill.map(|d| mm(d.min())).unwrap_or_default(), LEGEND);
    });
}

fn board_props(ui: &mut Ui, b: &Board) {
    let cu = b.stackup.copper().count();
    card(
        ui,
        Some(READOUT),
        |ui| {
            Line::new().legend("board").value(&b.name).elided(ui);
        },
        |ui| {
            readouts(
                ui,
                &[
                    ("copper", format!("{cu} L"), READOUT),
                    ("thickness", mm(b.stackup.thickness()), TRACE),
                    ("fab", b.fab.to_uppercase(), READOUT),
                ],
            );
            if !b.description.is_empty() {
                note(ui, &b.description, VALUE);
            }
            if let Some(p) = &b.stackup.preset {
                reading(ui, "preset", p.clone());
            }
            reading(ui, "finish", b.stackup.finish.clone());
            reading(ui, "mask", b.stackup.mask_color.clone());
            reading(ui, "silk", b.stackup.silk_color.clone());
            if let Some(o) = &b.outline {
                let s = paint::outline_bounds(o).size();
                reading(ui, "outline", format!("{} x {} mm", trim(s[0], 2), trim(s[1], 2)));
            }
        },
    );
    ui.add_space(8.0);
    section(ui, "design rules", "from the fab preset and [rules]", |ui| {
        for (name, v, doc) in b.rules.table() {
            ui.horizontal(|ui| {
                Line::new()
                    .legend(&name.replace("min_", "").replace('_', " "))
                    .column(ui, 150.0)
                    .set(mm(v))
                    .show(ui)
                    .on_hover_text(doc);
            });
        }
    });
    ui.add_space(8.0);
    if let Some(o) = &b.outline {
        section(ui, "outline", "Edge.Cuts", |ui| {
            let (rect, _) =
                ui.allocate_exact_size(Vec2::new(ui.available_width(), 180.0), Sense::hover());
            outline_preview(ui, rect, o, &b.cutouts);
        });
    }
}

fn outline_preview(ui: &Ui, rect: Rect, o: &Outline, cutouts: &[Outline]) {
    let p = ui.painter_at(rect);
    p.rect_filled(rect, 0.0, WELL);
    let mut v = View::default();
    let b = paint::outline_bounds(o);
    v.fit(rect, &b, 22.0);
    let xf = v.xf(rect);
    let pts: Vec<Pos2> = paint::outline_points(o).into_iter().map(|q| xf.pos(q)).collect();
    p.add(egui::epaint::PathShape::closed_line(pts, Stroke::new(1.5, paint::EDGE)));
    for c in cutouts {
        let pts: Vec<Pos2> = paint::outline_points(c).into_iter().map(|q| xf.pos(q)).collect();
        p.add(egui::epaint::PathShape::closed_line(pts, Stroke::new(1.5, paint::EDGE)));
    }
    let s = b.size();
    p.text(
        Pos2::new(xf.pos([(b.min[0] + b.max[0]) / 2.0, b.max[1]]).x, rect.bottom() - 4.0),
        egui::Align2::CENTER_BOTTOM,
        format!("{} mm", trim(s[0], 2)),
        figure(10.5),
        LEGEND,
    );
    p.text(
        Pos2::new(rect.left() + 4.0, rect.center().y),
        egui::Align2::LEFT_CENTER,
        format!("{} mm", trim(s[1], 2)),
        figure(10.5),
        LEGEND,
    );
}

fn mask_color(name: &str) -> Color32 {
    match name.to_ascii_lowercase().as_str() {
        "red" => Color32::from_rgb(0xA8, 0x30, 0x30),
        "blue" => Color32::from_rgb(0x2A, 0x4E, 0xA0),
        "black" | "matte black" => Color32::from_rgb(0x20, 0x20, 0x22),
        "white" => Color32::from_rgb(0xDD, 0xDD, 0xDD),
        "yellow" => Color32::from_rgb(0xC8, 0xB0, 0x30),
        "purple" => Color32::from_rgb(0x6A, 0x3A, 0x9A),
        _ => Color32::from_rgb(0x2E, 0x7D, 0x46),
    }
}

const COPPER: Color32 = Color32::from_rgb(0xC8, 0x8A, 0x4A);

fn board_sheet(ui: &mut Ui, b: &Board, _st: &mut PageState) {
    section(ui, "stackup", "top of the board first, to scale for dielectrics", |ui| stackup(ui, b));
    ui.add_space(10.0);
    section(ui, "net classes", "amber is what you set, cyan is what the stackup gives you", |ui| {
        netclasses(ui, b)
    });
    ui.add_space(10.0);
    section(ui, "vias", "drill, pad and the ring left between them", |ui| {
        let cols =
            [("name", 110.0), ("drill", 70.0), ("diameter", 80.0), ("ring", 70.0), ("span", 140.0)];
        Table::new(&cols, b.vias.len()).show(ui, |i, p, r, at| {
            let v = &b.vias[i];
            let ok = v.annular_ring() >= b.rules.min_annular_ring;
            cell(p, r, at(0), cols[0].1, &v.name, VALUE);
            cell(p, r, at(1), cols[1].1, &mm(v.drill), READOUT);
            cell(p, r, at(2), cols[2].1, &mm(v.diameter), READOUT);
            cell(p, r, at(3), cols[3].1, &mm(v.annular_ring()), if ok { TRACE } else { FAULT });
            cell(p, r, at(4), cols[4].1, &format!("{} - {}", v.from, v.to), LEGEND);
        });
    });
}

fn stackup(ui: &mut Ui, b: &Board) {
    let layers: Vec<_> = b.stackup.layers.iter().filter(|l| l.kind != LayerKind::Paste).collect();
    let diel: f64 =
        layers.iter().filter(|l| l.kind.is_dielectric()).map(|l| l.thickness.to_mm()).sum();
    let px_per_mm = if diel > 0.0 { (300.0 / diel).min(260.0) } else { 100.0 };
    let heights: Vec<f32> = layers
        .iter()
        .map(|l| match l.kind {
            LayerKind::Copper | LayerKind::Mask | LayerKind::Silk => 17.0,
            _ => ((l.thickness.to_mm() * px_per_mm) as f32).clamp(18.0, 120.0),
        })
        .collect();
    let total: f32 = heights.iter().sum::<f32>() + 8.0;
    let width = ui.available_width();
    let (rect, _) = ui.allocate_exact_size(Vec2::new(width, total), Sense::hover());
    let p = ui.painter_at(rect);
    let band_w = (width * 0.34).clamp(120.0, 260.0);
    let x0 = rect.left() + 4.0;
    let label_x = x0 + band_w + 14.0;
    let cols = [label_x, label_x + 90.0, label_x + 180.0, label_x + 260.0, label_x + 320.0];
    let mut y = rect.top() + 4.0;
    let mask = mask_color(&b.stackup.mask_color);
    let geometry: Vec<(String, Option<TraceGeometry>)> =
        b.stackup.copper().map(|(_, l)| (l.name.clone(), b.stackup.geometry(&l.name))).collect();
    for (l, h) in layers.iter().zip(&heights) {
        let row = Rect::from_min_size(Pos2::new(x0, y), Vec2::new(band_w, *h));
        let visual = match l.kind {
            LayerKind::Copper => 13.0,
            LayerKind::Mask => 8.0,
            LayerKind::Silk => 4.0,
            _ => *h - 1.0,
        };
        let band = Rect::from_center_size(row.center(), Vec2::new(band_w, visual));
        match l.kind {
            LayerKind::Copper => {
                p.rect_filled(band, 1.0, COPPER);
            }
            LayerKind::Mask => {
                p.rect_filled(band, 1.0, mask);
            }
            LayerKind::Silk => {
                p.rect_filled(band.shrink2(Vec2::new(band_w * 0.3, 0.0)), 0.0, VALUE);
            }
            LayerKind::Core => {
                p.rect_filled(band, 0.0, Color32::from_rgb(0x4B, 0x55, 0x36));
            }
            LayerKind::Prepreg => {
                p.rect_filled(band, 0.0, Color32::from_rgb(0x5E, 0x68, 0x40));
                let hatch = p.with_clip_rect(band);
                let mut x = band.left() - band.height();
                while x < band.right() {
                    hatch.line_segment(
                        [Pos2::new(x, band.bottom()), Pos2::new(x + band.height(), band.top())],
                        Stroke::new(1.0, Color32::from_rgb(0x70, 0x7C, 0x4C)),
                    );
                    x += 9.0;
                }
            }
            LayerKind::Paste => {}
        }
        let cy = band.center().y;
        let at = |i: usize| Pos2::new(cols[i], cy);
        let name_col = if l.kind == LayerKind::Copper { READOUT } else { LEGEND };
        p.text(
            at(0),
            egui::Align2::LEFT_CENTER,
            l.name.to_uppercase(),
            legend_font(11.0),
            name_col,
        );
        let kind = format!("{:?}", l.kind).to_lowercase();
        let material = if l.material.is_empty() || l.material == kind {
            kind
        } else {
            format!("{kind} {}", l.material)
        };
        p.text(
            at(1),
            egui::Align2::LEFT_CENTER,
            material,
            egui::FontId::proportional(11.5),
            LEGEND,
        );
        if l.thickness.is_positive() {
            p.text(
                at(2),
                egui::Align2::LEFT_CENTER,
                format!("{} mm", trim(l.thickness.to_mm(), 4)),
                figure(11.0),
                VALUE,
            );
        }
        if l.kind.is_dielectric() || l.kind == LayerKind::Mask {
            p.text(
                at(3),
                egui::Align2::LEFT_CENTER,
                format!("er {}", trim(l.er, 2)),
                figure(11.0),
                VALUE,
            );
        }
        if l.kind == LayerKind::Copper
            && let Some((_, Some(g))) = geometry.iter().find(|(n, _)| *n == l.name)
        {
            let s = match g {
                TraceGeometry::Microstrip { .. } => "microstrip",
                TraceGeometry::Stripline { .. } => "stripline",
            };
            p.text(at(4), egui::Align2::LEFT_CENTER, s, egui::FontId::proportional(11.5), TRACE);
        }
        y += h;
    }
    ui.add_space(4.0);
    Line::new()
        .legend("finished")
        .measured(format!("{} mm", trim(b.stackup.thickness().to_mm(), 3)))
        .legend("copper")
        .set(
            b.stackup
                .copper()
                .map(|(_, l)| trim(l.thickness.to_mm() * 1000.0, 1))
                .collect::<Vec<_>>()
                .join(" / ")
                + " um",
        )
        .show(ui);
}

fn netclasses(ui: &mut Ui, b: &Board) {
    let analysis = b.analyze();
    let cols = [
        ("class", 96.0),
        ("layer", 62.0),
        ("width", 62.0),
        ("clear", 56.0),
        ("target", 70.0),
        ("z", 66.0),
        ("fit w", 62.0),
        ("I max", 60.0),
        ("I need", 60.0),
    ];
    Table::new(&cols, analysis.len()).show(ui, |i, p, r, at| {
        let a = &analysis[i];
        let n = b.netclasses.iter().find(|n| n.name == a.netclass).unwrap();
        let first = i == 0 || analysis[i - 1].netclass != a.netclass;
        if first {
            cell(p, r, at(0), cols[0].1, &n.name, VALUE);
            cell(p, r, at(2), cols[2].1, &mm(n.track_width), READOUT);
            cell(p, r, at(3), cols[3].1, &mm(n.clearance), READOUT);
        }
        cell(p, r, at(1), cols[1].1, &a.layer, LEGEND);
        let kind = match (n.diff_gap, n.coplanar_gap) {
            (Some(_), _) => "d",
            (None, Some(_)) => "c",
            _ => "",
        };
        if let Some(t) = n.impedance {
            cell(p, r, at(4), cols[4].1, &format!("{}{kind}", trim(t.0, 1)), READOUT);
        }
        let zc = match a.impedance_ok {
            Some(true) => TRACE,
            Some(false) => FAULT,
            None => TRACE.gamma_multiply(0.6),
        };
        cell(p, r, at(5), cols[5].1, &format!("{:.1}{kind}", a.impedance), zc);
        if let Some(w) = a.width_for_impedance {
            cell(p, r, at(6), cols[6].1, &mm(w), TRACE);
        }
        cell(
            p,
            r,
            at(7),
            cols[7].1,
            &format!("{:.2}A", a.current_capacity.0),
            TRACE.gamma_multiply(0.8),
        );
        if let (Some(need), Some(i)) = (a.width_for_current, n.current) {
            let ok = need <= n.track_width;
            cell(p, r, at(8), cols[8].1, &format!("{}", i), if ok { OK } else { FAULT });
        }
    });
    if analysis.is_empty() {
        status(ui, false, "no net classes yet, add [[netclasses]] with at least a Default");
    }
    ui.add_space(2.0);
    hint(
        ui,
        "z in ohm (d = differential pair, c = grounded coplanar), uncoated. I max is IPC-2221 at the class temperature rise.",
    );
}

fn schematic_canvas(ui: &mut Ui, s: &Schematic, st: &mut PageState) {
    st.view.max_fit = if st.region.is_some() { 4000.0 } else { 45.0 };
    let (resp, xf) = st.view.show(ui, &st.region.unwrap_or(s.bounds()), 50.0);
    let p = ui.painter_at(xf.rect);
    paint::grid(&p, &xf, 2.54);
    let hover = if st.interactive { resp.hover_pos() } else { None };
    for f in &s.sheets {
        let r = egui::Rect::from_two_pos(xf.world(f.min), xf.world(f.max));
        p.rect_stroke(r, 0.0, egui::Stroke::new(1.0, LEGEND), egui::StrokeKind::Middle);
        p.text(
            r.left_top() + egui::vec2(0.0, -4.0),
            egui::Align2::LEFT_BOTTOM,
            &f.name,
            egui::FontId::monospace(xf.len(5.0).clamp(9.0, 40.0)),
            LEGEND,
        );
    }
    let hit = sheet::schematic(&p, &xf, s, hover, st.show_hidden);
    cursor_readout(ui, &xf, hover);
    if let Some((what, detail)) = sheet::legend_for(s, &hit) {
        resp.on_hover_ui_at_pointer(|ui| {
            Line::new().legend("net").set(&what).value(&detail).show(ui);
        });
    }
}

fn schematic_props(ui: &mut Ui, s: &Schematic) {
    let pins: usize = s.nets.iter().map(|n| n.pins.len()).sum();
    card(
        ui,
        Some(READOUT),
        |ui| {
            Line::new().legend("schematic").value(&s.name).elided(ui);
        },
        |ui| {
            readouts(
                ui,
                &[
                    ("parts", s.references().len().to_string(), VALUE),
                    ("nets", s.nets.len().to_string(), VALUE),
                    ("pins wired", pins.to_string(), TRACE),
                ],
            );
            if !s.description.is_empty() {
                note(ui, &s.description, VALUE);
            }
            if let Some(b) = &s.board {
                reading(ui, "board", b.clone());
            }
            if !s.sheets.is_empty() {
                let names: Vec<&str> = s.sheets.iter().map(|f| f.name.as_str()).collect();
                reading(ui, "sheets", names.join(", "));
            }
            if let Some(parent) = &s.parent {
                reading(ui, "sheet of", parent.clone());
            }
        },
    );
    ui.add_space(8.0);
    Line::new().legend("parts").show(ui);
    let mut parts: Vec<_> = s.parts.iter().collect();
    parts.sort_by(|a, b| natural_cmp(&a.reference, &b.reference));
    let cols = [("ref", 44.0), ("value", 110.0), ("footprint", 226.0)];
    Table::new(&cols, parts.len()).show(ui, |i, p, r, at| {
        let part = parts[i];
        cell(p, r, at(0), cols[0].1, &part.reference, READOUT);
        cell(p, r, at(1), cols[1].1, &part.value, VALUE);
        cell(p, r, at(2), cols[2].1, part.footprint.as_deref().unwrap_or("-"), LEGEND);
    });
    ui.add_space(8.0);
    Line::new().legend("nets").show(ui);
    let cols = [("net", 96.0), ("class", 60.0), ("pins", 224.0)];
    Table::new(&cols, s.nets.len()).show(ui, |i, p, r, at| {
        let n = &s.nets[i];
        let pins: Vec<String> = n.pins.iter().map(|x| s.pin_label(*x)).collect();
        cell(p, r, at(0), cols[0].1, &n.name, VALUE);
        cell(p, r, at(1), cols[1].1, &n.class, LEGEND);
        cell(p, r, at(2), cols[2].1, &pins.join(" "), TRACE);
    });
}

fn layout_canvas(ui: &mut Ui, project: &Project, i: usize, st: &mut PageState) {
    let l = &project.layouts[i].item;
    if st.panels {
        ui.horizontal(|ui| {
            ui.add_space(8.0);
            if toggle(ui, "2d", !st.view_3d).clicked() {
                st.view_3d = false;
            }
            if toggle(ui, "3d", st.view_3d).clicked() {
                st.view_3d = true;
            }
            if st.view_3d {
                ui.add_space(12.0);
                if toggle(ui, "parts", st.show_parts).clicked() {
                    st.show_parts = !st.show_parts;
                }
            }
        });
    }
    if st.view_3d {
        let key = (project.generation, i, agentee_3d::generation());
        if st.scene.as_ref().map(|s| s.0) != Some(key)
            && let Some(board) = project.boards.iter().find(|b| b.name == l.board)
        {
            let fetch = if st.interactive {
                agentee_3d::Fetch::Background
            } else {
                agentee_3d::Fetch::Blocking
            };
            let scene = crate::board3d::build(l, &board.item, &project.root, fetch);
            st.scene = Some((key, std::sync::Arc::new(scene)));
        }
        if let Some((_, scene)) = &st.scene {
            crate::board3d::show(
                ui,
                scene,
                &mut st.camera,
                st.interactive,
                st.show_parts,
                &mut st.soft_3d,
            );
        }
        return;
    }
    let key = (project.generation, i);
    if st.zone_key != Some(key) {
        st.zone_tex = pcb::zone_textures(ui.ctx(), l);
        st.zone_key = Some(key);
    }
    st.view.max_fit = 2000.0;
    let (resp, xf) = st.view.show(ui, &st.region.unwrap_or(l.bounds()), 30.0);
    let p = ui.painter_at(xf.rect);
    paint::grid(&p, &xf, 1.0);
    let hover = if st.interactive { resp.hover_pos() } else { None };
    let hit = pcb::layout(&p, &xf, l, &st.pcb_layers, &st.zone_tex, hover, st.ratsnest);
    scale_bar(&p, &xf);
    cursor_readout(ui, &xf, hover);
    if hit.net.is_some() || hit.pad.is_some() {
        resp.on_hover_ui_at_pointer(|ui| {
            if let Some((pi, k)) = hit.pad {
                let part = &l.parts[pi];
                Line::new()
                    .legend("pad")
                    .set(format!("{}.{}", part.reference, part.pads[k].number))
                    .value(&part.value)
                    .show(ui);
            }
            if let Some(n) = hit.net {
                let net = &l.nets[n];
                Line::new().legend("net").set(&net.name).value(&net.class).show(ui);
            }
        });
    }
}

fn layout_props(ui: &mut Ui, l: &Layout, st: &mut PageState) {
    let unrouted = l.unrouted();
    card(
        ui,
        Some(if unrouted == 0 { OK } else { FAULT }),
        |ui| {
            Line::new().legend("layout").value(&l.name).elided(ui);
        },
        |ui| {
            let routed = l.nets.iter().filter(|n| n.unrouted == 0).count();
            readouts(
                ui,
                &[
                    ("parts", l.parts.len().to_string(), VALUE),
                    (
                        "nets routed",
                        format!("{routed}/{}", l.nets.len()),
                        if unrouted == 0 { OK } else { FAULT },
                    ),
                    ("vias", l.vias.len().to_string(), VALUE),
                ],
            );
            reading(ui, "board", l.board.clone());
            reading(ui, "schematic", l.schematic.clone());
        },
    );
    ui.add_space(8.0);
    Line::new().legend("layers").show(ui);
    let side = |a: &str, b: &str| vec![a.to_string(), b.to_string()];
    let groups: [(&str, Vec<String>); 6] = [
        ("copper", l.copper.clone()),
        ("silk", side("F.SilkS", "B.SilkS")),
        ("mask", side("F.Mask", "B.Mask")),
        ("fab", side("F.Fab", "B.Fab")),
        ("courtyard", side("F.CrtYd", "B.CrtYd")),
        ("board", side("Edge.Cuts", "Cutouts")),
    ];
    for (group, names) in &groups {
        ui.horizontal_top(|ui| {
            ui.allocate_ui_with_layout(
                Vec2::new(80.0, 20.0),
                egui::Layout::left_to_right(egui::Align::Center),
                |ui| {
                    ui.set_min_width(80.0);
                    ui.label(legend(*group))
                },
            );
            ui.horizontal_wrapped(|ui| {
                for n in names {
                    if toggle(ui, n, st.pcb_layers.shows(n)).clicked() {
                        st.pcb_layers.toggle(n);
                    }
                }
                if *group == "board" && toggle(ui, "ratsnest", st.ratsnest).clicked() {
                    st.ratsnest = !st.ratsnest;
                }
            });
        });
    }
    ui.add_space(6.0);
    let cols =
        [("net", 110.0), ("class", 64.0), ("width", 60.0), ("length", 66.0), ("state", 70.0)];
    Table::new(&cols, l.nets.len()).show(ui, |i, p, r, at| {
        let n = &l.nets[i];
        cell(p, r, at(0), cols[0].1, &n.name, VALUE);
        cell(p, r, at(1), cols[1].1, &n.class, LEGEND);
        cell(p, r, at(2), cols[2].1, &trim(n.width, 3), READOUT);
        cell(p, r, at(3), cols[3].1, &format!("{} mm", trim(n.length_mm, 1)), TRACE);
        let (s, c) = if n.unrouted == 0 {
            ("routed".to_string(), OK)
        } else {
            (format!("{} open", n.unrouted), FAULT)
        };
        cell(p, r, at(4), cols[4].1, &s, c);
    });
}

fn sim_canvas(ui: &mut Ui, project: &Project, index: usize, st: &mut PageState) {
    let s = &project.sims[index].item;
    egui::Frame::NONE.fill(CHASSIS).inner_margin(egui::Margin::symmetric(12, 10)).show(ui, |ui| {
        ui.set_min_size(ui.available_size());
        sim_controls(ui, project, index, st);
        if let Some(pr) = &st.sim_progress {
            let secs = agentee_core::sim::now().saturating_sub(pr.started);
            let elapsed = format!("{}:{:02}", secs / 60, secs % 60);
            let phase = if pr.phase.is_empty() { "running" } else { pr.phase.as_str() };
            section(ui, phase, &format!("pid {}", pr.pid), |ui| {
                if phase == "running" {
                    progress(ui, "", pr.fraction(), Some(1.0), "");
                    ui.add_space(4.0);
                    Line::new()
                        .legend("port")
                        .set(format!("{} ({}/{})", pr.port, pr.run + 1, pr.runs))
                        .legend("steps")
                        .measured(format!("{} / {}", pr.steps, pr.max_steps))
                        .legend("fields down")
                        .measured(format!("{:.1} dB", pr.decay_db))
                        .legend("elapsed")
                        .value(elapsed)
                        .show(ui);
                } else {
                    let t = ui.input(|i| i.time) as f32;
                    progress(ui, "", (t * 0.5).fract(), None, "");
                    ui.ctx().request_repaint_after(std::time::Duration::from_millis(50));
                    ui.add_space(4.0);
                    Line::new().legend("elapsed").value(elapsed).show(ui);
                }
            });
            ui.add_space(8.0);
        }
        if let Some(m) = &s.maps {
            crate::heat::canvas(ui, project, s, index, &m.maps, st);
            return;
        }
        if let Some(c) = &s.channel {
            crate::eye::canvas(ui, c, index, st, project.generation);
            return;
        }
        if let Some(r) = &s.result
            && s.kind == agentee_core::sim::SimKind::Pdn
        {
            let h = ui.available_height();
            section(ui, "impedance", "self impedance at each sink, the other sinks open", |ui| {
                crate::plot::loglog(ui, r, egui::vec2(ui.available_width(), (h - 40.0).max(240.0)));
            });
            return;
        }
        if let Some(r) = &s.result {
            ui.horizontal(|ui| {
                if toggle(ui, "s-parameters", !st.show_fields && !st.show_tdr).clicked() {
                    st.show_fields = false;
                    st.show_tdr = false;
                }
                if toggle(ui, "tdr", st.show_tdr).clicked() {
                    st.show_tdr = true;
                    st.show_fields = false;
                }
                if !r.maps.is_empty() && toggle(ui, "fields", st.show_fields).clicked() {
                    st.show_fields = true;
                    st.show_tdr = false;
                }
            });
            ui.add_space(6.0);
            if st.show_fields && !r.maps.is_empty() {
                crate::heat::canvas(ui, project, s, index, &r.maps, st);
                return;
            }
            if st.show_tdr {
                let key = (project.generation, index);
                if st.tdr_cache.as_ref().map(|c| c.0) != Some(key) {
                    st.tdr_cache = Some((key, crate::plot::tdr_series(r, s)));
                }
                let series = &st.tdr_cache.as_ref().unwrap().1;
                let h = ui.available_height();
                section(ui, "tdr", "impedance seen from each driven port, Gaussian edge at 1.3 / the top frequency", |ui| {
                    crate::plot::xy_plot(ui, series, "ps", "ohm", egui::vec2(ui.available_width(), (h - 40.0).max(200.0)), Some(200.0));
                });
                return;
            }
        }
        let Some(r) = &s.result else {
            if st.sim_progress.is_some() {
                return;
            }
            section(ui, "not run yet", "", |ui| {
                note(
                    ui,
                    format!(
                        "Press run, or run `agentee sim {}`; the plot appears here when it finishes.",
                        s.name
                    ),
                    VALUE,
                );
            });
            return;
        };
        let h = ui.available_height();
        section(ui, "s-parameters", "magnitude, dB", |ui| {
            crate::plot::db_plot(ui, r, &st.hidden_curves, (h * 0.55).max(220.0), st.interactive);
        });
        ui.add_space(8.0);
        let side = (ui.available_height() - 40.0).min(ui.available_width() * 0.5).max(160.0);
        section(
            ui,
            "reflection",
            "S11, S22 ... from the start (dot) to the stop (ring) frequency",
            |ui| {
                ui.horizontal_top(|ui| {
                    crate::plot::smith(ui, r, &st.hidden_curves, side);
                    if !r.curves.is_empty() {
                        ui.add_space(8.0);
                        ui.vertical(|ui| {
                            let n = r.curves.len() as f32;
                            let size =
                                egui::vec2(ui.available_width(), (side - 6.0 * (n - 1.0)) / n);
                            for (k, c) in r.curves.iter().enumerate() {
                                crate::plot::curve_plot(
                                    ui,
                                    r,
                                    c,
                                    crate::plot::color(k + 4),
                                    size,
                                    st.interactive,
                                );
                                ui.add_space(6.0);
                            }
                        });
                    }
                });
            },
        );
    });
}

fn sim_controls(ui: &mut Ui, project: &Project, index: usize, st: &mut PageState) {
    let s = &project.sims[index].item;
    let has_result = s.result.is_some() || s.maps.is_some() || s.channel.is_some();
    let starting = st.runs.starting(&s.name) && st.sim_progress.is_none();
    let running = st.sim_progress.is_some() || st.runs.starting(&s.name);
    ui.horizontal(|ui| {
        if running {
            if toggle(ui, "stop", true).clicked() {
                let pid = st.sim_progress.as_ref().map(|p| p.pid);
                st.runs.stop(&s.name, pid);
            }
            if starting {
                let secs = st.runs.elapsed(&s.name).map(|d| d.as_secs()).unwrap_or(0);
                note(ui, format!("starting, {secs} s"), LEGEND);
                ui.ctx().request_repaint_after(std::time::Duration::from_millis(500));
            }
        } else {
            let label = if has_result { "re-run" } else { "run" };
            if toggle(ui, label, false).clicked() {
                st.runs.start(project, &s.name);
            }
            if s.stale {
                note(ui, "the copper or the spec changed since the last run", WARN);
            }
        }
        if let Some(e) = st.runs.failure(&s.name) {
            note(ui, format!("last run failed: {e}"), FAULT);
        }
    });
    ui.add_space(6.0);
}

fn sim_props(ui: &mut Ui, project: &Project, s: &agentee_core::sim::Sim, st: &mut PageState) {
    let layout = project.layouts.iter().find(|l| l.name == s.layout).map(|l| &l.item);
    let rail = match (
        &st.sim_progress,
        s.result.is_some() || s.maps.is_some() || s.channel.is_some(),
        s.stale,
    ) {
        (Some(_), _, _) => READOUT,
        (None, true, false) => OK,
        _ => WARN,
    };
    if let Some(m) = &s.maps {
        crate::heat::props(ui, s, m, rail);
        return;
    }
    if let Some(c) = &s.channel {
        crate::eye::props(ui, s, c, rail);
        return;
    }
    let cascade = s.kind == agentee_core::sim::SimKind::Cascade;
    if let Some(pdn) = &s.pdn {
        pdn_card(ui, s, pdn, rail);
        ui.add_space(8.0);
        if let Some(r) = &s.result {
            crate::heat::readings(ui, &r.readings);
        }
        return;
    }
    if cascade {
        cascade_card(ui, s, rail);
    } else {
        fdtd_card(ui, s, rail);
    }
    ui.add_space(8.0);
    sim_curves_and_readings(ui, s, st);
    if cascade {
        Line::new().legend("devices").show(ui);
        let cols = [("ref", 50.0), ("file", 150.0), ("ports", 160.0)];
        Table::new(&cols, s.devices.len()).show(ui, |i, p, r, at| {
            let d = &s.devices[i];
            cell(p, r, at(0), cols[0].1, &d.reference, READOUT);
            cell(p, r, at(1), cols[1].1, &d.file, VALUE);
            cell(p, r, at(2), cols[2].1, &d.ports.join(", "), LEGEND);
        });
        return;
    }
    sim_tables(ui, layout, s);
}

fn pdn_card(
    ui: &mut Ui,
    s: &agentee_core::sim::Sim,
    pdn: &agentee_core::sim::PdnSpec,
    rail: Color32,
) {
    card(
        ui,
        Some(rail),
        |ui| {
            Line::new().legend("pdn").value(&s.name).elided(ui);
        },
        |ui| {
            let target =
                pdn.target.map(|t| format!("{:.1} mohm", t * 1e3)).unwrap_or_else(|| "none".into());
            readouts(
                ui,
                &[
                    ("sinks", pdn.sinks.len().to_string(), VALUE),
                    ("decaps", pdn.decaps.len().to_string(), READOUT),
                    ("target", target, TRACE),
                ],
            );
            if !s.description.is_empty() {
                note(ui, &s.description, VALUE);
            }
            reading(ui, "board", s.board.clone());
            reading(ui, "sinks", pdn.sinks.join(", "));
            if let Some((p, r, l)) = &pdn.vrm {
                reading(ui, "vrm", format!("{p}: {:.1} mohm + {:.1} nH", r * 1e3, l * 1e9));
            }
            for c in &pdn.decaps {
                let what = match &c.model {
                    agentee_core::sim::DecapModel::File { path, .. } => path.clone(),
                    agentee_core::sim::DecapModel::Rlc { c, esl, esr } => {
                        let cap = if *c >= 1e-6 {
                            format!("{} uF", trim(c * 1e6, 2))
                        } else {
                            format!("{} nF", trim(c * 1e9, 2))
                        };
                        format!("{cap}, {:.2} nH, {:.1} mohm", esl * 1e9, esr * 1e3)
                    }
                };
                reading(ui, &c.port, format!("{} {what}", c.reference));
            }
        },
    );
}

fn cascade_card(ui: &mut Ui, s: &agentee_core::sim::Sim, rail: Color32) {
    card(
        ui,
        Some(rail),
        |ui| {
            Line::new().legend("cascade").value(&s.name).elided(ui);
        },
        |ui| {
            if let Some(r) = &s.result {
                let (a, b) = (r.freqs[0], *r.freqs.last().unwrap());
                readouts(
                    ui,
                    &[
                        ("ports", r.ports.len().to_string(), VALUE),
                        ("band", format!("{}-{} GHz", trim(a / 1e9, 2), trim(b / 1e9, 2)), READOUT),
                    ],
                );
            }
            if !s.description.is_empty() {
                note(ui, &s.description, VALUE);
            }
            reading(ui, "board", s.board.clone());
            if let Some(r) = &s.result {
                reading(ui, "ports", r.ports.join(", "));
            }
        },
    );
}

fn fdtd_card(ui: &mut Ui, s: &agentee_core::sim::Sim, rail: Color32) {
    card(
        ui,
        Some(rail),
        |ui| {
            Line::new().legend("fdtd").value(&s.name).elided(ui);
        },
        |ui| {
            let band = format!("{}-{} GHz", trim(s.start / 1e9, 2), trim(s.stop / 1e9, 2));
            readouts(
                ui,
                &[
                    ("ports", s.ports.len().to_string(), VALUE),
                    ("band", band, READOUT),
                    ("cell", format!("{} mm", trim(s.cell, 3)), READOUT),
                ],
            );
            if !s.description.is_empty() {
                note(ui, &s.description, VALUE);
            }
            reading(ui, "layout", s.layout.clone());
            if let Some(r) = &s.result {
                reading(
                    ui,
                    "grid",
                    format!(
                        "{} x {} x {}, {:.1} M cells",
                        r.grid[0],
                        r.grid[1],
                        r.grid[2],
                        r.cells as f64 / 1e6
                    ),
                );
                reading(
                    ui,
                    "steps",
                    r.steps.iter().map(|v| v.to_string()).collect::<Vec<_>>().join(" / "),
                );
                reading(ui, "run", format!("{:.1} s on {}", r.seconds, r.device));
            }
        },
    );
}

fn sim_curves_and_readings(ui: &mut Ui, s: &agentee_core::sim::Sim, st: &mut PageState) {
    if let Some(r) = &s.result {
        Line::new().legend("curves").show(ui);
        ui.horizontal_wrapped(|ui| {
            for (i, j) in crate::plot::curves(r) {
                let on = !st.hidden_curves.contains(&(i, j));
                if toggle(ui, &crate::plot::label(i, j), on).clicked() {
                    if on {
                        st.hidden_curves.push((i, j));
                    } else {
                        st.hidden_curves.retain(|c| *c != (i, j));
                    }
                }
            }
        });
        ui.add_space(6.0);
    }
    if let Some(r) = &s.result
        && !r.readings.is_empty()
    {
        crate::heat::readings(ui, &r.readings);
        ui.add_space(8.0);
    }
}

fn sim_tables(ui: &mut Ui, layout: Option<&Layout>, s: &agentee_core::sim::Sim) {
    Line::new().legend("ports").show(ui);
    let cols = [("#", 26.0), ("name", 90.0), ("pad", 70.0), ("layers", 124.0), ("z", 50.0)];
    Table::new(&cols, s.ports.len()).show(ui, |i, p, r, at| {
        let port = &s.ports[i];
        let pad = layout
            .map(|l| {
                format!(
                    "{}.{}",
                    l.parts[port.part].reference, l.parts[port.part].pads[port.pad].number
                )
            })
            .unwrap_or_default();
        cell(p, r, at(0), cols[0].1, &(i + 1).to_string(), READOUT);
        cell(p, r, at(1), cols[1].1, &port.name, VALUE);
        cell(p, r, at(2), cols[2].1, &pad, TRACE);
        cell(p, r, at(3), cols[3].1, &format!("{} / {}", port.layer, port.reference), LEGEND);
        cell(p, r, at(4), cols[4].1, &format!("{}", port.impedance), LEGEND);
    });
    ui.add_space(8.0);
    Line::new().legend("lumped models").show(ui);
    let cols = [("ref", 50.0), ("model", 110.0), ("layer", 80.0)];
    Table::new(&cols, s.elements.len()).show(ui, |i, p, r, at| {
        let e = &s.elements[i];
        let text = match e.model {
            agentee_core::sim::Model::Capacitor(c) if c >= 1e-6 => {
                format!("{} uF", trim(c * 1e6, 3))
            }
            agentee_core::sim::Model::Capacitor(c) if c >= 1e-9 => {
                format!("{} nF", trim(c * 1e9, 3))
            }
            agentee_core::sim::Model::Capacitor(c) => format!("{} pF", trim(c * 1e12, 3)),
            agentee_core::sim::Model::Inductor(l) => format!("{} nH", trim(l * 1e9, 3)),
            agentee_core::sim::Model::Resistor(v) => format!("{} ohm", trim(v, 3)),
            agentee_core::sim::Model::Open => "open".into(),
        };
        cell(p, r, at(0), cols[0].1, &e.reference, READOUT);
        cell(p, r, at(1), cols[1].1, &text, VALUE);
        cell(p, r, at(2), cols[2].1, &e.layer, LEGEND);
    });
}
