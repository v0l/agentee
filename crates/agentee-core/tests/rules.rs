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

#[test]
fn every_spot_in_a_green_zone_passes_the_rules() {
    for (project, layout_net) in [(lna(), "VCC"), (sdr(), "GND")] {
        let l = &project.layouts[0].item;
        let b = &project.boards.iter().find(|x| x.name == l.board).unwrap().item;
        let cx = agentee_core::drc::Ctx::new(
            b,
            &l.copper,
            &l.outline,
            &l.board_cutouts,
            &l.parts,
            &l.tracks,
            &l.vias,
            &[],
            &l.nets,
        );
        let base = Placed::new(&cx);
        let net = l.nets.iter().position(|n| n.name == layout_net).unwrap();
        let via = agentee_core::layout::Via::of(&b.vias[0], net, [0.0, 0.0], &l.copper);
        let mut ob = agentee_core::graphic::Bounds::EMPTY;
        l.outline.iter().for_each(|p| ob.add(*p));
        let c = ob.center();
        let mut window = agentee_core::graphic::Bounds::EMPTY;
        window.add([c[0] - 4.0, c[1] - 4.0]);
        window.add([c[0] + 4.0, c[1] + 4.0]);
        let zone = rules::green(&base, &rules::Template::via(&via), &window, 0.05);
        let spots = zone.spots();
        assert!(!spots.is_empty(), "nothing is green");
        let step = (spots.len() / 300).max(1);
        let mut bad = Vec::new();
        for at in spots.iter().step_by(step) {
            let v = agentee_core::layout::Via { at: *at, ..via.clone() };
            if let Err(e) = rules::legal(&Planned::new(&base, vec![], vec![v])) {
                bad.push((*at, e[0].rule, e[0].detail.clone(), e[0].other.clone()));
            }
        }
        assert!(
            bad.is_empty(),
            "{} green spots break a rule: {:?}",
            bad.len(),
            &bad[..bad.len().min(3)]
        );
        let free = rules::Zone::new(&window, 0.05, &l.copper).spots().len();
        assert!(spots.len() < free, "the zone forbade nothing");
    }
}

#[test]
fn a_moved_part_is_judged_where_it_lands() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/lna");
    let p = agentee_core::Project::load(&root).unwrap();
    let l = &p.layouts[0].item;
    let b = &p.boards.iter().find(|x| x.name == l.board).unwrap().item;
    let cx = agentee_core::drc::Ctx::new(
        b,
        &l.copper,
        &l.outline,
        &l.board_cutouts,
        &l.parts,
        &l.tracks,
        &l.vias,
        &[],
        &l.nets,
    );
    let base = Placed::new(&cx);
    let mut alone = Vec::new();
    rules::check(&base, &mut alone);
    let at = |r: &str| l.parts.iter().position(|p| p.reference == r).unwrap();
    let stay = |i: usize| rules::Move {
        part: i,
        at: l.parts[i].at.to_mm(),
        rotation: l.parts[i].rotation,
        bottom: l.parts[i].bottom,
    };
    for i in 0..l.parts.len() {
        let mut out = Vec::new();
        let plan = rules::Plan { parts: vec![stay(i)], ..Default::default() };
        rules::check(&Planned::after(&base, &rules::Plan::default(), plan), &mut out);
        assert!(out.len() <= alone.len(), "{}: {:?}", l.parts[i].reference, out);
    }
    let (u, r) = (at("U1"), at("R1"));
    let onto = rules::Move { at: l.parts[r].at.to_mm(), ..stay(u) };
    let plan = rules::Plan { parts: vec![onto], ..Default::default() };
    assert!(rules::legal(&Planned::after(&base, &rules::Plan::default(), plan)).is_err());
    let hidden = Planned::hiding(
        &base,
        &[r],
        &rules::Plan::default(),
        rules::Plan { parts: vec![onto], ..Default::default() },
    );
    let mut out = Vec::new();
    rules::check(&hidden, &mut out);
    assert!(out.iter().all(|v| !v.subject.contains("R1.") && !v.other.contains("R1.")), "{out:?}");
}

#[test]
fn a_dropped_net_takes_all_its_connections_off_the_count() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/lna");
    let p = agentee_core::Project::load(&root).unwrap();
    let l = &p.layouts[0].item;
    let b = &p.boards.iter().find(|x| x.name == l.board).unwrap().item;
    let pads: Vec<_> = l
        .parts
        .iter()
        .flat_map(|q| &q.pads)
        .filter(|q| q.net.is_some() && q.copper.iter().any(|c| c == "F.Cu"))
        .collect();
    let a = pads[0];
    let other = pads.iter().find(|q| q.net != a.net).unwrap();
    let centre =
        |q: &agentee_core::layout::PlacedPad| agentee_core::drc::rings_bounds(&q.outlines).center();
    let name = l.nets[a.net.unwrap()].name.clone();
    let mut per_net = std::collections::HashMap::new();
    per_net.insert(name.clone(), 2);
    let r = agentee_core::route::RouteResult {
        connections: 3,
        routed: 3,
        tracks: vec![agentee_core::route::RoutedTrack {
            net: name,
            layer: "F.Cu".into(),
            width: None,
            points: vec![centre(a), centre(other)],
        }],
        per_net,
        ..Default::default()
    };
    let held = agentee_core::route::hold_to_rules(l, b, r);
    assert!(held.tracks.is_empty());
    assert_eq!(held.routed, 1);
}
