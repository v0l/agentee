use agentee_core::project::Project;
use agentee_view::testplan::{bring_up_order, expected, plan_for};
use agentee_view::{RenderOptions, render_test_plan_png};
use std::path::Path;

const REG: &str = "name = \"LDO33\"\nreference = \"U\"\n\
[regulator]\noutput = \"OUT\"\ninput = \"IN\"\nenable = \"EN\"\nvout = \"3.3V\"\naccuracy = \"2%\"\n\
[[pins]]\nnumber = \"1\"\nname = \"IN\"\ntype = \"power_in\"\nat = [-7.62, 0.0]\nside = \"left\"\n\
[[pins]]\nnumber = \"2\"\nname = \"EN\"\ntype = \"input\"\nat = [-7.62, 2.54]\nside = \"left\"\n\
[[pins]]\nnumber = \"3\"\nname = \"OUT\"\ntype = \"power_out\"\nat = [7.62, 0.0]\nside = \"right\"\n";

const TP: &str = "name = \"TestPoint\"\nreference = \"TP\"\n\
[[pins]]\nnumber = \"1\"\nat = [0.0, 0.0]\nside = \"bottom\"\ntype = \"passive\"\n";

fn project() -> Project {
    let dir = std::env::temp_dir().join(format!("agentee-view-plan-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("symbols")).unwrap();
    std::fs::write(dir.join("symbols/LDO33.sym.toml"), REG).unwrap();
    std::fs::write(dir.join("symbols/TestPoint.sym.toml"), TP).unwrap();
    let part = |r: &str, s: &str, x: f64| {
        format!("[[parts]]\nref = \"{r}\"\nsymbol = \"{s}\"\nvalue = \"{s}\"\nat = [{x}, 25.4]\n")
    };
    let net = |n: &str, pins: &str| {
        format!("[[nets]]\nname = \"{n}\"\nclass = \"Default\"\npins = [{pins}]\n")
    };
    let mut s = String::from("name = \"t\"\nno_connect = [\"U1.2\"]\n");
    s += &part("U1", "LDO33", 25.4);
    s += &part("TP2", "TestPoint", 50.8);
    s += &part("TP10", "TestPoint", 76.2);
    s += &part("TP1", "TestPoint", 101.6);
    s += &net("VIN", "\"U1.1\", \"TP1.1\"");
    s += &net("3V3", "\"U1.3\", \"TP2.1\"");
    s += &net("MYSTERY", "\"TP10.1\"");
    std::fs::write(dir.join("t.sch.toml"), s).unwrap();
    Project::load(&dir).unwrap()
}

#[test]
fn pads_list_in_bring_up_order_with_the_expected_reading() {
    let p = project();
    let plan = plan_for(&p).unwrap();
    let rows = bring_up_order(&plan);
    let orders: Vec<u32> = rows.iter().map(|r| r.order).collect();
    assert!(orders.windows(2).all(|w| w[0] <= w[1]), "{orders:?}");
    let pos = |n: &str| rows.iter().position(|r| r.reference == n).unwrap();
    assert!(pos("TP2") < pos("TP1"), "known rails come before unknown pads");
    assert!(pos("TP1") < pos("TP10"), "ties fall back to natural reference order");
    let rail = rows.iter().find(|r| r.reference == "TP2").unwrap();
    assert_eq!(expected(rail), "3.234 to 3.366 V");
    let unknown = rows.iter().find(|r| r.reference == "TP10").unwrap();
    assert_eq!(expected(unknown), "unknown");
}

#[test]
fn the_test_plan_tab_renders() {
    let p = project();
    let opts = RenderOptions { width: 1200, height: 600, ..Default::default() };
    let png = render_test_plan_png(&p, &opts).unwrap();
    let mut r = png::Decoder::new(std::io::Cursor::new(png)).read_info().unwrap();
    let mut buf = vec![0; r.output_buffer_size().unwrap()];
    r.next_frame(&mut buf).unwrap();
    let distinct: std::collections::HashSet<&[u8]> = buf.chunks(4).collect();
    assert!(distinct.len() > 20);
    if let Ok(dir) = std::env::var("AGENTEE_PLAN_PNG") {
        std::fs::write(Path::new(&dir), render_test_plan_png(&p, &opts).unwrap()).unwrap();
    }
}
