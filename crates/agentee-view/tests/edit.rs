use agentee_core::project::{ItemRef, Project};
use agentee_view::edit::{Sel, Tool};
use agentee_view::pages::{PageState, page};
use egui::{Event, Key, Modifiers, PointerButton, Pos2, RawInput, Rect, Vec2};
use std::path::{Path, PathBuf};

const SIZE: Vec2 = Vec2::new(1200.0, 800.0);

fn copy(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).unwrap();
    for e in std::fs::read_dir(from).unwrap().flatten() {
        let p = e.path();
        let name = p.file_name().unwrap().to_string_lossy().to_string();
        if p.is_dir() && (name == "footprints" || name == "symbols") {
            copy(&p, &to.join(&name));
        } else if p.is_file() && !name.contains(".sim.") && name.ends_with(".toml") {
            std::fs::copy(&p, to.join(&name)).unwrap();
        }
    }
}

fn lna() -> PathBuf {
    example("lna")
}

fn example(name: &str) -> PathBuf {
    static N: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let k = N.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    let dir = std::env::temp_dir().join(format!("agentee-edit-{}-{k}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples").join(name);
    copy(&root, &dir);
    dir
}

struct Harness {
    ctx: egui::Context,
    st: PageState,
    project: Project,
    item: ItemRef,
    time: f64,
    path: PathBuf,
}

impl Harness {
    fn new(dir: &Path) -> Harness {
        let project = Project::load(dir).unwrap();
        let item = ItemRef::Layout(0);
        let path = project.layouts[0].path.clone();
        let st = PageState { panels: false, ..Default::default() };
        let ctx = egui::Context::default();
        egui_bench::install(&ctx);
        let mut h = Harness { ctx, st, project, item, time: 0.0, path };
        h.frame(vec![]);
        h.frame(vec![]);
        h
    }

    fn frame(&mut self, events: Vec<Event>) {
        self.time += 0.05;
        let raw = RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, SIZE)),
            time: Some(self.time),
            events,
            ..Default::default()
        };
        let (project, item, st) = (&self.project, self.item, &mut self.st);
        let _ = self.ctx.run_ui(raw, |ui| page(ui, project, item, st));
    }

    fn screen(&self, mm: [f64; 2]) -> Pos2 {
        self.st.view.xf(Rect::from_min_size(Pos2::ZERO, SIZE)).world(mm)
    }

    fn button(&mut self, at: Pos2, pressed: bool) {
        self.frame(vec![Event::PointerButton {
            pos: at,
            button: PointerButton::Primary,
            pressed,
            modifiers: Modifiers::NONE,
        }]);
    }

    fn click(&mut self, mm: [f64; 2]) {
        let at = self.screen(mm);
        self.frame(vec![Event::PointerMoved(at)]);
        self.button(at, true);
        self.button(at, false);
    }

    fn drag(&mut self, from: [f64; 2], to: [f64; 2]) {
        let (a, b) = (self.screen(from), self.screen(to));
        self.frame(vec![Event::PointerMoved(a)]);
        self.button(a, true);
        for k in 1..=8 {
            self.frame(vec![Event::PointerMoved(a + (b - a) * (k as f32 / 8.0))]);
        }
        self.button(b, false);
    }

    fn key(&mut self, key: Key, modifiers: Modifiers) {
        let at = self.screen([18.0, 12.0]);
        self.frame(vec![
            Event::PointerMoved(at),
            Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers },
        ]);
    }

    fn settle(&mut self) {
        for _ in 0..400 {
            self.frame(vec![]);
            if !self.ed().checking() && !self.ed().routing() {
                return;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        panic!("the editor never settled");
    }

    fn ed(&self) -> &agentee_view::edit::Editor {
        self.st.editors.get(&self.path).expect("an editor for the layout")
    }

    fn ed_mut(&mut self) -> &mut agentee_view::edit::Editor {
        self.st.editors.get_mut(&self.path).unwrap()
    }

    fn file(&self) -> String {
        std::fs::read_to_string(&self.path).unwrap()
    }
}

fn part_at(p: &Project, r: &str) -> [f64; 2] {
    p.layouts[0].item.parts.iter().find(|x| x.reference == r).unwrap().at.to_mm()
}

#[test]
fn dragging_a_part_moves_it_and_saves_on_ctrl_s() {
    let dir = lna();
    let mut h = Harness::new(&dir);
    let before = h.file();
    let pad = {
        let l = &h.project.layouts[0].item;
        let c1 = l.parts.iter().find(|p| p.reference == "C1").unwrap();
        let mut b = agentee_core::graphic::Bounds::EMPTY;
        c1.pads[0].outlines.iter().flatten().for_each(|q| b.add(*q));
        b.center()
    };
    h.drag(pad, [pad[0] + 1.0, pad[1] - 2.0]);
    assert_eq!(h.ed().sel, Some(Sel::Part("C1".into())));
    h.settle();
    assert!(h.ed().dirty);
    assert_eq!(h.file(), before, "nothing is written before save");
    let shown = h.ed().layout(&h.project, 0).parts.iter().find(|p| p.reference == "C1").unwrap();
    let at = shown.at.to_mm();
    assert!((at[0] - 9.5).abs() < 0.03 && (at[1] - 10.0).abs() < 0.03, "{at:?}");
    h.key(Key::S, Modifiers::COMMAND);
    assert!(!h.ed().dirty);
    let saved = Project::load(&dir).unwrap();
    let moved = part_at(&saved, "C1");
    assert_eq!(moved, at);
    assert!(h.file().contains("label = { at = [9.5"), "the label follows the part");
}

#[test]
fn undo_and_redo_walk_the_edits() {
    let dir = lna();
    let mut h = Harness::new(&dir);
    let start = part_at(&h.project, "C1");
    h.ed_mut().sel = Some(Sel::Part("C1".into()));
    h.key(Key::R, Modifiers::NONE);
    h.settle();
    let turned = |h: &Harness| {
        h.ed().layout(&h.project, 0).parts.iter().find(|p| p.reference == "C1").unwrap().rotation
    };
    assert_eq!(turned(&h), 90.0);
    h.key(Key::Z, Modifiers::COMMAND);
    h.settle();
    assert_eq!(turned(&h), 0.0);
    assert!(!h.ed().dirty);
    h.key(Key::Z, Modifiers::COMMAND | Modifiers::SHIFT);
    h.settle();
    assert_eq!(turned(&h), 90.0);
    assert_eq!(part_at(&h.project, "C1"), start);
}

#[test]
fn route_tool_draws_a_track_between_pads() {
    let dir = lna();
    let mut h = Harness::new(&dir);
    let tracks = h.project.layouts[0].item.tracks.len();
    let (a, b, net) = h.project.layouts[0].item.ratsnest.first().copied().unwrap_or_default();
    let routed = !h.project.layouts[0].item.ratsnest.is_empty();
    let (from, to) = if routed {
        (a, b)
    } else {
        let l = &h.project.layouts[0].item;
        let t = &l.tracks[0];
        (t.points[0], *t.points.last().unwrap())
    };
    h.key(Key::X, Modifiers::NONE);
    assert_eq!(h.ed().tool, Tool::Route);
    h.click(from);
    assert!(h.ed().draft.is_some(), "the route starts on copper");
    let mid = [(from[0] + to[0]) / 2.0, from[1]];
    h.click(mid);
    h.click(to);
    if h.ed().draft.is_some() {
        h.key(Key::Enter, Modifiers::NONE);
    }
    h.settle();
    let l = h.ed().layout(&h.project, 0);
    assert_eq!(l.tracks.len(), tracks + 1);
    let added = l.tracks.iter().max_by_key(|t| t.source).unwrap();
    if routed {
        assert_eq!(added.net, net);
    }
    assert!(added.points.len() >= 2);
}

#[test]
fn deleting_a_track_and_undoing_restores_the_file_text() {
    let dir = lna();
    let mut h = Harness::new(&dir);
    let n = h.project.layouts[0].item.tracks.len();
    let t = h.project.layouts[0].item.tracks[0].clone();
    let mid = [(t.points[0][0] + t.points[1][0]) / 2.0, (t.points[0][1] + t.points[1][1]) / 2.0];
    h.click(mid);
    assert!(matches!(h.ed().sel, Some(Sel::Track(_))), "{:?}", h.ed().sel);
    h.key(Key::Delete, Modifiers::NONE);
    h.settle();
    assert_eq!(h.ed().layout(&h.project, 0).tracks.len(), n - 1);
    h.key(Key::Z, Modifiers::COMMAND);
    h.settle();
    assert_eq!(h.ed().layout(&h.project, 0).tracks.len(), n);
    assert!(!h.ed().dirty);
}

#[test]
fn a_file_change_under_unsaved_edits_is_a_conflict() {
    let dir = lna();
    let mut h = Harness::new(&dir);
    h.ed_mut().sel = Some(Sel::Part("C1".into()));
    h.key(Key::R, Modifiers::NONE);
    h.settle();
    let text =
        h.file().replace("ref = \"C2\"\nat = [18.8, 12.0]", "ref = \"C2\"\nat = [18.9, 12.0]");
    std::fs::write(&h.path, text).unwrap();
    h.project = Project::load(&dir).unwrap();
    let ctx = h.ctx.clone();
    h.st.sync(&ctx, &h.project);
    assert!(h.ed().conflict.is_some());
    h.ed_mut().take_theirs(&ctx);
    assert!(!h.ed().dirty && h.ed().conflict.is_none());
}

#[test]
fn dragging_a_fanout_via_takes_it_out_of_the_rule() {
    use agentee_core::geom::dist;
    use agentee_core::layout::ViaSource;
    let dir = example("hdi");
    let mut h = Harness::new(&dir);
    for l in ["In1.Cu", "In2.Cu", "B.Cu"] {
        h.st.pcb_layers.hidden.retain(|x| x != l);
    }
    let l = &h.project.layouts[0].item;
    let v = l.vias.iter().find(|v| matches!(v.source, ViaSource::Fanout(_))).unwrap().clone();
    let to = [v.at[0] + 0.25, v.at[1] + 0.25];
    h.drag(v.at, to);
    h.settle();
    let l = h.ed().layout(&h.project, 0);
    let moved = l.vias.iter().find(|x| dist(x.at, to) < 1e-6).unwrap();
    assert!(matches!(moved.source, ViaSource::File { .. }), "{:?}", moved.source);
    assert!(!l.vias.iter().any(|x| dist(x.at, v.at) < 1e-6));
    h.ed_mut().save().unwrap();
    assert!(h.file().contains("skip_at = [["), "{}", h.file());
    let saved = Project::load(&dir).unwrap();
    let vias = &saved.layouts[0].item.vias;
    assert!(vias.iter().any(|x| dist(x.at, to) < 1e-6 && x.name == v.name));
    assert!(!vias.iter().any(|x| dist(x.at, v.at) < 1e-6));
}

#[test]
fn routing_one_connection_closes_it() {
    let dir = lna();
    let mut h = Harness::new(&dir);
    let l = &h.project.layouts[0].item;
    let t = l.tracks.iter().find(|t| l.nets[t.net].name == "LED_A").unwrap().source;
    let ctx = h.ctx.clone();
    let project = Project::load(&dir).unwrap();
    h.ed_mut().delete_track(&ctx, &project, 0, t);
    h.settle();
    let open = h.ed().layout(&h.project, 0).ratsnest.clone();
    assert_eq!(open.len(), 1, "{open:?}");
    let (a, b, n) = open[0];
    h.ed_mut().route_connection(&ctx, &project, 0, n, Some((a, b)));
    h.settle();
    h.settle();
    assert!(h.ed().note.is_none(), "{:?}", h.ed().note);
    assert!(h.ed().layout(&h.project, 0).ratsnest.is_empty());
}

#[test]
fn a_selected_track_over_a_pad_drags_before_the_pad() {
    let dir = lna();
    let mut h = Harness::new(&dir);
    let l = &h.project.layouts[0].item;
    let (t, part) = l
        .tracks
        .iter()
        .find_map(|t| {
            let p0 = t.points[0];
            let part = l.parts.iter().find(|p| {
                p.pads
                    .iter()
                    .any(|q| q.outlines.iter().any(|o| agentee_core::geom::point_in_polygon(p0, o)))
            })?;
            Some((t.clone(), part.reference.clone()))
        })
        .expect("a track that ends in a pad");
    let before = part_at(&h.project, &part);
    h.click(t.points[0]);
    assert_eq!(h.ed().sel, Some(Sel::Part(part.clone())), "the pad wins the first click");
    h.click(t.points[0]);
    assert_eq!(h.ed().sel, Some(Sel::Track(t.source)), "a second click cycles to the track");
    let to = [t.points[0][0], t.points[0][1] + 0.5];
    h.drag(t.points[0], to);
    h.settle();
    let l = h.ed().layout(&h.project, 0);
    let moved = l.tracks.iter().find(|x| x.source == t.source).unwrap();
    assert!(agentee_core::geom::dist(moved.points[0], to) < 0.03, "{:?}", moved.points[0]);
    let p = l.parts.iter().find(|p| p.reference == part).unwrap();
    assert_eq!(p.at.to_mm(), before);
}
