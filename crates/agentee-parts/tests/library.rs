use agentee_core::project::Project;
use agentee_parts::library::{self, Enforce, Library, PartFile, Stock};
use std::path::{Path, PathBuf};

fn lna() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/lna")
}

fn scratch(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("agentee-lib-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn entry(lib: &Path, name: &str, part: PartFile, symbol: &str, footprint: Option<&str>) {
    let dir = lib.join(name);
    std::fs::create_dir_all(&dir).unwrap();
    let sym = std::fs::read_to_string(lna().join("symbols").join(symbol)).unwrap();
    std::fs::write(dir.join(&part.symbol), sym).unwrap();
    if let (Some(src), Some(dst)) = (footprint, &part.footprint) {
        let fp = std::fs::read_to_string(lna().join("footprints").join(src)).unwrap();
        std::fs::write(dir.join(dst), fp).unwrap();
    }
    library::write_entry(&dir, &part).unwrap();
}

fn connectors(dir: &Path) {
    entry(
        dir,
        "JST-PH-2",
        PartFile {
            role: "connector".into(),
            mpn: Some("S2B-PH-K-S".into()),
            manufacturer: Some("JST".into()),
            rating: Some("2 A".into()),
            preferred: true,
            notes: "power input".into(),
            symbol: "JST-PH-2.sym.toml".into(),
            stock: vec![Stock {
                distributor: "Farnell".into(),
                sku: "9491902".into(),
                stock: 5000,
                currency: "EUR".into(),
                breaks: vec![[1.0, 0.2], [100.0, 0.08]],
                min: 1,
                mult: 1,
                checked: "2026-10-09".into(),
                ..Default::default()
            }],
            ..Default::default()
        },
        "Conn_01x02.sym.toml",
        None,
    );
    entry(
        dir,
        "JST-XH-4",
        PartFile {
            role: "connector".into(),
            mpn: Some("B4B-XH-A".into()),
            symbol: "Conn.sym.toml".into(),
            ..Default::default()
        },
        "Conn_01x02.sym.toml",
        None,
    );
    entry(
        dir,
        "R0402",
        PartFile {
            role: "resistor".into(),
            generic: true,
            symbol: "R.sym.toml".into(),
            footprint: Some("R_0402.fp.toml".into()),
            ..Default::default()
        },
        "R.sym.toml",
        Some("R_0402_1005Metric.fp.toml"),
    );
}

#[test]
fn lists_parts_by_role_and_words_with_their_stock() {
    let dir = scratch("list");
    connectors(&dir);
    let lib = Library::open(None, Some(&dir));
    assert!(lib.errors.is_empty(), "{:?}", lib.errors);
    let names = |v: Vec<&library::Entry>| v.iter().map(|e| e.name.clone()).collect::<Vec<_>>();
    assert_eq!(names(lib.filter(Some("connectors"), &[])), ["JST-PH-2", "JST-XH-4"]);
    assert_eq!(names(lib.filter(None, &["2-pin".into(), "power".into()])), ["JST-PH-2"]);
    let ph = lib.get("s2b-ph-k-s").unwrap();
    assert_eq!(ph.pin_count, Some(2));
    assert_eq!(ph.symbol_name.as_deref(), Some("Conn_01x02"));
    let best = ph.best_stock().unwrap();
    assert_eq!(best.unit_at(1), Some(0.2));
    assert_eq!(best.line(), "Farnell 9491902: 5000 in stock, 0.2000 EUR each, checked 2026-10-09");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn check_names_the_library_part_for_a_connector_outside_it() {
    let dir = scratch("check");
    connectors(&dir);
    let lib = Library::open(None, Some(&dir));
    let p = Project::load(&lna()).unwrap();
    let sch = &p.schematics[0];
    let d = library::check(&lib, &sch.item, &sch.path);
    let refs: Vec<&str> = d.iter().map(|x| x.at.as_str()).collect();
    assert_eq!(refs, ["J3"], "{d:#?}");
    assert!(d[0].message.contains("use JST-PH-2"), "{}", d[0].message);

    std::fs::write(dir.join(library::CONFIG_FILE), "check = \"off\"\n").unwrap();
    let off = Library::open(None, Some(&dir));
    assert_eq!(off.config.check, Enforce::Off);
    assert!(library::check(&off, &sch.item, &sch.path).is_empty());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_project_library_overrides_the_global_one() {
    let global = scratch("global");
    connectors(&global);
    let root = scratch("project");
    let local = library::project_dir(&root);
    entry(
        &local,
        "JST-PH-2",
        PartFile {
            role: "connector".into(),
            mpn: Some("S2B-PH-SM4-TB".into()),
            symbol: "Conn.sym.toml".into(),
            ..Default::default()
        },
        "Conn_01x02.sym.toml",
        None,
    );
    let lib = Library::open(Some(&root), Some(&global));
    let ph = lib.get("JST-PH-2").unwrap();
    assert_eq!((ph.scope.as_str(), ph.part.mpn.as_deref()), ("project", Some("S2B-PH-SM4-TB")));
    assert_eq!(lib.entries.len(), 3);
    let _ = std::fs::remove_dir_all(&global);
    let _ = std::fs::remove_dir_all(&root);
}
