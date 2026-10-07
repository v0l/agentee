use agentee_core::rules::{self, Placed, Planned, Rule};

fn sdr() -> agentee_core::Project {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/sdr");
    agentee_core::Project::load(&root).unwrap()
}

#[test]
fn what_tie_plans_passes_the_rules_the_check_runs() {
    let p = sdr();
    let l = &p.layouts[0].item;
    let b = &p.boards.iter().find(|x| x.name == l.board).unwrap().item;
    let r = agentee_core::tie::tie(l, b, &["GND".into()]).unwrap();
    assert!(r.tied >= 5, "{}", r.tied);
    let stitched = |v: &&agentee_core::layout::Via| {
        matches!(v.source, agentee_core::layout::ViaSource::Stitch(_))
    };
    let fixed: Vec<_> = l.vias.iter().filter(|v| !stitched(v)).cloned().collect();
    let cx = agentee_core::drc::Ctx::new(
        b,
        &l.copper,
        &l.outline,
        &l.board_cutouts,
        &l.parts,
        &l.tracks,
        &fixed,
        &[],
        &l.nets,
    );
    let base = Placed::new(&cx);
    let net = |name: &str| l.nets.iter().position(|n| n.name == name).unwrap();
    let tracks = r
        .tracks
        .iter()
        .map(|t| agentee_core::layout::Track {
            source: usize::MAX,
            net: net(&t.net),
            layer: t.layer.clone(),
            width: t.width.unwrap_or(l.nets[net(&t.net)].width),
            points: t.points.clone(),
        })
        .collect();
    let vias = r
        .vias
        .iter()
        .map(|v| {
            let spec = b.vias.iter().find(|x| x.name == v.via).unwrap();
            agentee_core::layout::Via::of(spec, net(&v.net), v.at, &l.copper)
        })
        .collect();
    let plan = Planned::new(&base, tracks, vias);
    let broken = rules::legal(&plan).err().unwrap_or_default();
    assert!(broken.is_empty(), "{:?}", &broken[..broken.len().min(3)]);
}

#[test]
fn a_planned_track_is_judged_after_kept_vias_the_same_as_alone() {
    let p = sdr();
    let l = &p.layouts[0].item;
    let b = &p.boards.iter().find(|x| x.name == l.board).unwrap().item;
    let cx = agentee_core::drc::Ctx::of_layout(b, l);
    let base = Placed::new(&cx);
    let n = l.nets.iter().position(|n| n.name == "3V3").unwrap();
    let crossing = agentee_core::layout::Track {
        source: usize::MAX,
        net: n,
        layer: "F.Cu".into(),
        width: 0.25,
        points: vec![[6.58, 26.1], [7.3932, 25.2868]],
    };
    let kept = agentee_core::layout::Via::of(&b.vias[0], n, [60.0, 60.0], &l.copper);
    let mut alone = Vec::new();
    rules::NetClearance.eval(&Planned::new(&base, vec![crossing.clone()], vec![]), &mut alone);
    let mut after = Vec::new();
    rules::NetClearance.eval(
        &Planned::after(
            &base,
            &rules::Plan { vias: vec![kept], ..Default::default() },
            rules::Plan { tracks: vec![crossing], ..Default::default() },
        ),
        &mut after,
    );
    assert!(!alone.is_empty());
    assert_eq!(alone, after);
}

fn lna() -> agentee_core::Project {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/lna");
    agentee_core::Project::load(&root).unwrap()
}

#[test]
fn every_registered_rule_judges_a_plan_by_what_it_adds() {
    let p = lna();
    let l = &p.layouts[0].item;
    let b = &p.boards[0].item;
    let cx = agentee_core::drc::Ctx::of_layout(b, l);
    let base = Placed::new(&cx);
    let mut placed = Vec::new();
    rules::everything(&base, &mut placed);
    let ruled: Vec<&agentee_core::Diagnostic> =
        p.layouts[0].diags.iter().filter(|d| d.rule.is_some()).collect();
    assert!(placed.len() <= ruled.len(), "{} against {}", placed.len(), ruled.len());
    let net = l.nets.iter().position(|n| n.name == "VCC").unwrap();
    let thin = agentee_core::layout::Track {
        source: usize::MAX,
        net,
        layer: "F.Cu".into(),
        width: 0.01,
        points: vec![[2.0, 2.0], [2.0, 4.0]],
    };
    let mut planned = Vec::new();
    rules::everything(&Planned::new(&base, vec![thin], vec![]), &mut planned);
    assert!(!planned.is_empty(), "a 0.01 mm track broke nothing");
    assert!(planned.iter().all(|v| !placed.iter().any(|b| b.detail == v.detail)));
}
