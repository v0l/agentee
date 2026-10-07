use std::path::{Path, PathBuf};
use std::process::Command;

fn bin() -> &'static str {
    env!("CARGO_BIN_EXE_agentee")
}

fn dir(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("agentee-edit-{}-{tag}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(d.join("symbols")).unwrap();
    std::fs::create_dir_all(d.join("footprints")).unwrap();
    let lna = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/lna");
    for f in [
        "symbols/R.sym.toml",
        "symbols/C.sym.toml",
        "footprints/R_0402_1005Metric.fp.toml",
        "footprints/C_0402_1005Metric.fp.toml",
    ] {
        std::fs::copy(lna.join(f), d.join(f)).unwrap();
    }
    d
}

fn run(d: &Path, args: &[&str]) -> (String, String, bool) {
    let out = Command::new(bin()).args(args).current_dir(d).output().unwrap();
    (
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
        out.status.success(),
    )
}

fn edit(d: &Path, args: &[&str]) -> String {
    let mut all = vec!["edit"];
    all.extend_from_slice(args);
    let (out, err, ok) = run(d, &all);
    assert!(ok, "agentee {all:?} failed: {err}{out}");
    out
}

fn sch(d: &Path, file: &str) -> String {
    std::fs::read_to_string(d.join(file)).unwrap()
}

#[test]
fn adds_parts_and_nets_by_pin_name() {
    let d = dir("sch");
    std::fs::write(
        d.join("b.board.toml"),
        "name = \"b\"\nfab = \"jlcpcb\"\n[outline]\nsize = [30, 20]\n[stackup]\npreset = \"jlcpcb-2l-1.6mm\"\n\
         [[netclasses]]\nname = \"Default\"\ntrack_width = \"0.2mm\"\nclearance = \"0.15mm\"\n\
         [[netclasses]]\nname = \"Signal\"\ntrack_width = \"0.2mm\"\nclearance = \"0.15mm\"\n\
         [[netclasses]]\nname = \"Power\"\ntrack_width = \"0.5mm\"\nclearance = \"0.15mm\"\n",
    )
    .unwrap();
    std::fs::write(d.join("t.sch.toml"), "name = \"t\"\nboard = \"b\"\n").unwrap();

    edit(&d, &["sch", "t", "add", "R1", "R", "1k", "--footprint", "R_0402_1005Metric"]);
    edit(&d, &["sch", "t", "add", "R2", "R", "10k"]);
    edit(&d, &["sch", "t", "net", "MID", "R1.2", "R2.1", "--class", "Signal"]);
    edit(&d, &["sch", "t", "nc", "R2.2"]);

    let text = sch(&d, "t.sch.toml");
    assert!(text.contains("ref = \"R1\""), "{text}");
    assert!(text.contains("symbol = \"R\""), "{text}");
    assert!(text.contains("pins = [\"R1.2\", \"R2.1\"]"), "{text}");
    assert!(text.contains("class = \"Signal\""), "{text}");
    assert!(text.contains("no_connect = [\"R2.2\"]"), "{text}");
    assert!(text.contains("at = [5.08, 0.0]"), "{text}");
}

#[test]
fn pin_names_resolve_to_numbers() {
    let d = dir("pinnames");
    std::fs::write(d.join("t.sch.toml"), "name = \"t\"\n").unwrap();
    let out = edit(&d, &["sch", "t", "add", "R1", "R", "1k"]);
    assert!(!out.contains("\"R1.2\""), "no facts in text mode: {out}");
    edit(&d, &["sch", "t", "net", "A", "R1.2"]);
    let text = sch(&d, "t.sch.toml");
    assert!(text.contains("pins = [\"R1.2\"]"), "{text}");
}

#[test]
fn one_load_for_a_whole_script() {
    let d = dir("script");
    std::fs::write(d.join("t.sch.toml"), "name = \"t\"\n").unwrap();
    let script = d.join("build.txt");
    std::fs::write(
        &script,
        "# a divider\nadd R1 R 1k --footprint R_0402_1005Metric\nadd R2 R 10k\n\
         net MID R1.2 R2.1\nnc R2.2\nnote \"voltage divider\"\n",
    )
    .unwrap();
    let out = run(&d, &["edit", "sch", script.to_str().unwrap()]);
    assert!(out.2, "{}", out.1);
    let text = sch(&d, "t.sch.toml");
    assert!(text.contains("ref = \"R2\""), "{text}");
    assert!(text.contains("pins = [\"R1.2\", \"R2.1\"]"), "{text}");
    assert!(text.contains("description = \"voltage divider\""), "{text}");
}

#[test]
fn a_script_names_its_item_when_there_are_several() {
    let d = dir("named-script");
    std::fs::write(d.join("a.sch.toml"), "name = \"a\"\n").unwrap();
    std::fs::write(d.join("b.sch.toml"), "name = \"b\"\n").unwrap();

    let (_, err, ok) = run(&d, &["edit", "sch", "-"]);
    assert!(!ok);
    assert!(err.contains("agentee edit sch NAME -") && err.contains("a, b"), "{err}");

    let mut child = Command::new(bin())
        .args(["edit", "sch", "b", "-"])
        .current_dir(&d)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    std::io::Write::write_all(
        child.stdin.as_mut().unwrap(),
        b"add R1 R 1k\nadd R2 R 10k\nnet MID R1.2 R2.1\n",
    )
    .unwrap();
    let out = child.wait_with_output().unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    assert!(sch(&d, "b.sch.toml").contains("pins = [\"R1.2\", \"R2.1\"]"));
    assert_eq!(sch(&d, "a.sch.toml"), "name = \"a\"\n");

    let script = d.join("build.txt");
    std::fs::write(&script, "nc R2.2\n").unwrap();
    edit(&d, &["sch", "b", script.to_str().unwrap()]);
    assert!(sch(&d, "b.sch.toml").contains("no_connect = [\"R2.2\"]"));
}

#[test]
fn a_pin_moves_when_it_joins_another_net() {
    let d = dir("move");
    std::fs::write(d.join("t.sch.toml"), "name = \"t\"\n").unwrap();
    edit(&d, &["sch", "t", "add", "R1", "R", "1k"]);
    edit(&d, &["sch", "t", "add", "R2", "R", "10k"]);
    edit(&d, &["sch", "t", "net", "A", "R1.1", "R1.2"]);
    edit(&d, &["sch", "t", "net", "B", "R2.1", "R1.2"]);
    let text = sch(&d, "t.sch.toml");
    assert!(text.contains("pins = [\"R1.1\"]"), "{text}");
    assert!(text.contains("pins = [\"R2.1\", \"R1.2\"]"), "{text}");
    let count = |net: &str| -> usize {
        sch(&d, "t.sch.toml")
            .split("name = \"")
            .find(|chunk| chunk.starts_with(&format!("{net}\"")))
            .and_then(|chunk| chunk.split_once("pins = ["))
            .and_then(|(_, rest)| rest.split_once(']'))
            .map(|(pins, _)| pins.split(',').count())
            .unwrap_or(0)
    };
    assert_eq!(count("A"), 1, "R1.1 is the only pin left on A");
    assert_eq!(count("B"), 2, "R1.2 moved to B");
}

#[test]
fn comments_and_formatting_survive() {
    let d = dir("keep");
    std::fs::write(
        d.join("t.sch.toml"),
        "# the top sheet\nname = \"t\"  # trailing note\n\n[[parts]]\nref = \"R1\"\n\
         symbol = \"R\"\nvalue = \"1k\"\nat = [10.16, 20.32]\n",
    )
    .unwrap();
    edit(&d, &["sch", "t", "net", "A", "R1.1"]);
    let text = sch(&d, "t.sch.toml");
    assert!(text.contains("# the top sheet"), "{text}");
    assert!(text.contains("# trailing note"), "{text}");
    assert!(text.contains("at = [10.16, 20.32]"), "{text}");
}

#[test]
fn board_and_layout_edits_write_the_keys() {
    let d = dir("pcb");
    std::fs::write(
        d.join("b.board.toml"),
        "name = \"b\"\nfab = \"jlcpcb\"\n[outline]\nsize = [30, 20]\n[stackup]\npreset = \"jlcpcb-2l-1.6mm\"\n\
         [[vias]]\nname = \"std\"\ndrill = \"0.3mm\"\ndiameter = \"0.6mm\"\n\
         [[netclasses]]\nname = \"Default\"\ntrack_width = \"0.2mm\"\nclearance = \"0.15mm\"\nvia = \"std\"\n",
    )
    .unwrap();
    std::fs::write(
        d.join("t.sch.toml"),
        "name = \"t\"\nboard = \"b\"\n[[parts]]\nref = \"R1\"\nsymbol = \"R\"\nvalue = \"1k\"\nat = [10.16, 20.32]\n\
         footprint = \"R_0402_1005Metric\"\n[[parts]]\nref = \"R2\"\nsymbol = \"R\"\nvalue = \"10k\"\nat = [20.32, 20.32]\n\
         footprint = \"R_0402_1005Metric\"\n\
         [[nets]]\nname = \"A\"\nclass = \"Default\"\npins = [\"R1.1\", \"R2.1\"]\n\
         [[nets]]\nname = \"GND\"\nclass = \"Default\"\nstyle = \"power\"\npins = [\"R1.2\", \"R2.2\"]\n",
    )
    .unwrap();
    std::fs::write(d.join("t.pcb.toml"), "name = \"t\"\nboard = \"b\"\nschematic = \"t\"\n")
        .unwrap();

    edit(
        &d,
        &[
            "board",
            "b",
            "class",
            "Power",
            "--track-width",
            "0.5mm",
            "--current",
            "1A",
            "--via",
            "std",
        ],
    );
    let board = std::fs::read_to_string(d.join("b.board.toml")).unwrap();
    assert!(board.contains("name = \"Power\""), "{board}");
    assert!(board.contains("current = \"1A\""), "{board}");

    edit(&d, &["board", "b", "outline", "--size", "40,25", "--corner-radius", "2mm"]);
    let board = std::fs::read_to_string(d.join("b.board.toml")).unwrap();
    assert!(board.contains("size = [40.0, 25.0]"), "{board}");

    let script = d.join("pcb.txt");
    std::fs::write(
        &script,
        "place R1 10,10\nplace R2 12,10\ntrack A F.Cu 10,9.5 12,9.5\nvia GND 11,8 --via std\n         zone GND --layers F.Cu --priority 1\n",
    )
    .unwrap();
    let out = run(&d, &["edit", "pcb", script.to_str().unwrap()]);
    assert!(out.0.contains("A"), "the track and zone did not land: {}\n{}", out.0, out.1);
    let pcb = std::fs::read_to_string(d.join("t.pcb.toml")).unwrap();
    for want in [
        "ref = \"R1\"",
        "at = [10.0, 10.0]",
        "[[tracks]]",
        "points = [[10.0, 9.5], [12.0, 9.5]]",
        "[[vias]]",
        "via = \"std\"",
        "[[zones]]",
        "priority = 1",
    ] {
        assert!(pcb.contains(want), "missing {want} in\n{pcb}");
    }

    run(&d, &["edit", "pcb", "t", "title", "Buggy Guard v1.0"]);
    let pcb = std::fs::read_to_string(d.join("t.pcb.toml")).unwrap();
    assert!(pcb.contains("title = \"Buggy Guard v1.0\""), "{pcb}");
    let first_table = pcb.find('[').unwrap();
    assert!(pcb.find("title =").unwrap() < first_table, "title must stay a top level key\n{pcb}");
    run(&d, &["edit", "pcb", "t", "title", "Buggy Guard v1.1", "--at", "20,4", "--size", "2mm"]);
    let pcb = std::fs::read_to_string(d.join("t.pcb.toml")).unwrap();
    assert!(
        pcb.contains("title = { text = \"Buggy Guard v1.1\", at = [20.0, 4.0], size = \"2mm\" }"),
        "{pcb}"
    );
    run(&d, &["edit", "pcb", "t", "title", "--clear"]);
    assert!(!std::fs::read_to_string(d.join("t.pcb.toml")).unwrap().contains("title"));
}

#[test]
fn bad_input_is_refused_and_nothing_is_written() {
    let d = dir("bad");
    std::fs::write(d.join("t.sch.toml"), "name = \"t\"\n").unwrap();
    let before = sch(&d, "t.sch.toml");
    for args in [
        vec!["sch", "t", "add", "R1", "NoSuchSymbol"],
        vec!["sch", "t", "net", "A", "R1.1"],
        vec!["sch", "t", "add", "R1", "R", "1k", "--at", "north"],
        vec!["sch", "nope", "add", "R1", "R", "1k"],
        vec!["sch", "t", "add", "R1", "R", "1k", "--colour", "red"],
    ] {
        let mut all = vec!["edit"];
        all.extend(args.iter().copied());
        let out = Command::new(bin()).args(&all).current_dir(&d).output().unwrap();
        assert!(!out.status.success(), "agentee {all:?} should have failed");
        assert!(!String::from_utf8_lossy(&out.stderr).is_empty(), "{all:?} said nothing");
        assert_eq!(before, sch(&d, "t.sch.toml"), "{all:?} wrote the file anyway");
    }
}

#[test]
fn a_class_is_allowed_before_the_board_exists() {
    let d = dir("nobody");
    std::fs::write(d.join("t.sch.toml"), "name = \"t\"\n").unwrap();
    edit(&d, &["sch", "t", "add", "R1", "R", "1k"]);
    edit(&d, &["sch", "t", "net", "VCC", "R1.1", "--class", "Power"]);
    assert!(sch(&d, "t.sch.toml").contains("class = \"Power\""));
    std::fs::write(
        d.join("b.board.toml"),
        "name = \"b\"\nfab = \"jlcpcb\"\n[outline]\nsize = [30, 20]\n[stackup]\npreset = \"jlcpcb-2l-1.6mm\"\n\
         [[netclasses]]\nname = \"Default\"\ntrack_width = \"0.2mm\"\nclearance = \"0.15mm\"\n",
    )
    .unwrap();
    let (out, err, ok) = run(&d, &["edit", "sch", "t", "net", "VCC", "R1.1", "--class", "Nope"]);
    assert!(!ok, "a class the board does not have must be refused");
    assert!(err.contains("Nope"), "{err}");
}

#[test]
fn an_unknown_item_is_refused_with_the_names_it_has() {
    let d = dir("two");
    std::fs::write(d.join("a.sch.toml"), "name = \"a\"\n").unwrap();
    std::fs::write(d.join("b.sch.toml"), "name = \"b\"\n").unwrap();
    let (_, err, ok) = run(&d, &["edit", "sch", "nope", "add", "R1", "R", "1k"]);
    assert!(!ok, "an unknown schematic must fail");
    assert!(err.contains("no schematic named `nope`"), "{err}");
    assert!(err.contains("there is a, b"), "{err}");
    assert_eq!(sch(&d, "a.sch.toml"), "name = \"a\"\n");
    assert_eq!(sch(&d, "b.sch.toml"), "name = \"b\"\n");
}

#[test]
fn a_part_on_a_child_sheet_can_be_placed() {
    let d = dir("hier");
    std::fs::write(
        d.join("b.board.toml"),
        "name = \"b\"\nfab = \"jlcpcb\"\n[outline]\nsize = [30, 20]\n[stackup]\npreset = \"jlcpcb-2l-1.6mm\"\n\
         [[vias]]\nname = \"std\"\ndrill = \"0.3mm\"\ndiameter = \"0.6mm\"\n\
         [[netclasses]]\nname = \"Default\"\ntrack_width = \"0.2mm\"\nclearance = \"0.15mm\"\nvia = \"std\"\n",
    )
    .unwrap();
    std::fs::write(
        d.join("top.sch.toml"),
        "name = \"top\"\nboard = \"b\"\nsheets = [\"power\", \"rf\"]\n",
    )
    .unwrap();
    std::fs::write(
        d.join("power.sch.toml"),
        "name = \"power\"\nboard = \"b\"\nsheets = [\"reg\"]\n\
         [[parts]]\nref = \"U1\"\nsymbol = \"R\"\nvalue = \"1k\"\nat = [10.16, 20.32]\n",
    )
    .unwrap();
    std::fs::write(
        d.join("reg.sch.toml"),
        "name = \"reg\"\nboard = \"b\"\n[[parts]]\nref = \"U2\"\nsymbol = \"R\"\nvalue = \"1k\"\n\
         at = [20.32, 20.32]\n",
    )
    .unwrap();
    std::fs::write(d.join("rf.sch.toml"), "name = \"rf\"\nboard = \"b\"\n").unwrap();
    std::fs::write(d.join("p.pcb.toml"), "name = \"p\"\nboard = \"b\"\nschematic = \"top\"\n")
        .unwrap();

    let (out, _, _) = run(&d, &["edit", "pcb", "p", "--json", "place", "U1", "12,8"]);
    assert!(out.contains("\"U1 placed at 12,8\""), "{out}");
    assert!(out.contains("sheet power"), "the sheet should be named: {out}");
    let (out, err, _) = run(&d, &["edit", "pcb", "p", "place", "U2", "14,8"]);
    assert!(!err.starts_with("error:"), "a part two sheets down must place: {err}");
    assert!(out.contains("U2 placed"), "{out}");
    assert!(sch(&d, "p.pcb.toml").contains("ref = \"U2\""));
    assert!(!sch(&d, "p.pcb.toml").contains("power"));

    let (_, err, ok) = run(&d, &["edit", "pcb", "p", "place", "R9", "1,1"]);
    assert!(!ok, "a part of no sheet must fail");
    assert!(err.contains("not a part of schematic `top` or of any sheet it lists"), "{err}");
}

#[test]
fn a_net_on_a_child_sheet_is_known_to_the_layout() {
    let d = dir("hiernet");
    std::fs::write(
        d.join("b.board.toml"),
        "name = \"b\"\nfab = \"jlcpcb\"\n[outline]\nsize = [30, 20]\n[stackup]\npreset = \"jlcpcb-2l-1.6mm\"\n\
         [[vias]]\nname = \"std\"\ndrill = \"0.3mm\"\ndiameter = \"0.6mm\"\n\
         [[netclasses]]\nname = \"Default\"\ntrack_width = \"0.2mm\"\nclearance = \"0.15mm\"\nvia = \"std\"\n\
         [[netclasses]]\nname = \"Power\"\ntrack_width = \"0.5mm\"\nclearance = \"0.15mm\"\nvia = \"std\"\n",
    )
    .unwrap();
    std::fs::write(d.join("top.sch.toml"), "name = \"top\"\nboard = \"b\"\nsheets = [\"power\"]\n")
        .unwrap();
    std::fs::write(
        d.join("power.sch.toml"),
        "name = \"power\"\nboard = \"b\"\n[[parts]]\nref = \"U1\"\nsymbol = \"R\"\nvalue = \"1k\"\nat = [10.16, 20.32]\n\
         [[nets]]\nname = \"VCC\"\nclass = \"Power\"\npins = [\"U1.1\"]\n\
         [[nets]]\nname = \"GND\"\nclass = \"Power\"\nstyle = \"power\"\npins = [\"U1.2\"]\n",
    )
    .unwrap();
    std::fs::write(d.join("p.pcb.toml"), "name = \"p\"\nboard = \"b\"\nschematic = \"top\"\n")
        .unwrap();

    for args in [
        vec!["track", "VCC", "F.Cu", "5,5", "6,6"],
        vec!["via", "GND", "7,7"],
        vec!["zone", "VCC", "--layers", "F.Cu"],
    ] {
        let mut all = vec!["edit", "pcb", "p"];
        all.extend(args.iter().copied());
        let (out, err, _) = run(&d, &all);
        assert!(!err.starts_with("error:"), "agentee {all:?} refused a real net: {err}");
        assert!(out.contains("VCC") || out.contains("GND"), "{out}");
    }
    let pcb = sch(&d, "p.pcb.toml");
    assert!(pcb.contains("net = \"VCC\""), "{pcb}");
    assert!(pcb.contains("net = \"GND\""), "{pcb}");

    let before = sch(&d, "p.pcb.toml");
    let (_, err, ok) = run(&d, &["edit", "pcb", "p", "track", "VCD", "F.Cu", "5,5", "6,6"]);
    assert!(!ok, "a net of no sheet must be refused, not invented");
    assert!(err.contains("`VCD` is not a net of top or its sheets"), "{err}");
    assert!(err.contains("it has GND, VCC"), "{err}");
    assert_eq!(before, sch(&d, "p.pcb.toml"), "the file changed on a refused net");
}

#[test]
fn help_lists_the_commands() {
    let d = dir("help");
    let (out, _, ok) = run(&d, &["edit", "sch", "help"]);
    assert!(ok, "edit sch help");
    for want in ["add", "net", "connect", "nc", "move", "set"] {
        assert!(out.contains(want), "sch help misses {want}: {out}");
    }
    let (out, _, _) = run(&d, &["edit", "pcb", "help"]);
    for want in ["place", "track", "via", "zone", "pair", "stitch", "fanout", "watermark", "title"]
    {
        assert!(out.contains(want), "pcb help misses {want}: {out}");
    }
    let (out, _, _) = run(&d, &["edit", "board", "help"]);
    for want in ["class", "unclass", "via", "outline", "cutout", "stackup"] {
        assert!(out.contains(want), "board help misses {want}: {out}");
    }
}

#[test]
fn list_shows_the_item() {
    let d = dir("list");
    std::fs::write(d.join("t.sch.toml"), "name = \"t\"\n").unwrap();
    edit(&d, &["sch", "t", "add", "R1", "R", "1k"]);
    edit(&d, &["sch", "t", "net", "A", "R1.1", "R1.2"]);
    let (out, _, ok) = run(&d, &["edit", "sch", "t", "--list"]);
    assert!(ok);
    assert!(out.contains("\"R1\""), "{out}");
    assert!(out.contains("\"A\""), "{out}");
    assert!(out.contains("\"R1.2\""), "{out}");
}

#[test]
fn keeps_every_repeated_field() {
    let d = dir("fields");
    std::fs::write(d.join("t.sch.toml"), "name = \"t\"\n").unwrap();
    edit(
        &d,
        &[
            "sch",
            "t",
            "add",
            "R1",
            "R",
            "1k",
            "--field",
            "mfr=Yageo",
            "--field",
            "mpn=RC0402FR-071KL",
        ],
    );
    let s = sch(&d, "t.sch.toml");
    assert!(s.contains("mfr = \"Yageo\"") && s.contains("mpn = \"RC0402FR-071KL\""), "{s}");
    edit(
        &d,
        &["sch", "t", "set", "R1", "--field", "mfr=Vishay", "--field", "mpn=CRCW04021K00FKED"],
    );
    let s = sch(&d, "t.sch.toml");
    assert!(s.contains("mfr = \"Vishay\"") && s.contains("mpn = \"CRCW04021K00FKED\""), "{s}");
    edit(&d, &["sch", "t", "set", "R1", "--field", "spares=0"]);
    let s = sch(&d, "t.sch.toml");
    assert!(
        s.contains("fields = { mfr = \"Vishay\", mpn = \"CRCW04021K00FKED\", spares = \"0\" }"),
        "{s}"
    );
    let (_, err, ok) =
        run(&d, &["edit", "sch", "t", "set", "R1", "--value", "2k", "--value", "3k"]);
    assert!(!ok && err.contains("more than once"), "{err}");
}
