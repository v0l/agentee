use std::path::{Path, PathBuf};
use std::process::Command;

fn bin() -> &'static str {
    env!("CARGO_BIN_EXE_agentee")
}

fn project() -> PathBuf {
    let d = std::env::temp_dir().join(format!("agentee-polarity-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(d.join("symbols")).unwrap();
    let lna = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/lna");
    std::fs::copy(lna.join("symbols/R.sym.toml"), d.join("symbols/R.sym.toml")).unwrap();
    std::fs::copy(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/MMBT3906.sym.toml"),
        d.join("symbols/MMBT3906.sym.toml"),
    )
    .unwrap();
    std::fs::write(
        d.join("t.sch.toml"),
        "name = \"t\"\n\n[[parts]]\nref = \"Q1\"\nsymbol = \"MMBT3906\"\nvalue = \"MMBT3906\"\nat = [5.08, 0.0]\n\n\
         [[parts]]\nref = \"R1\"\nsymbol = \"R\"\nvalue = \"1k\"\nat = [10.16, 0.0]\n\n\
         [[nets]]\nname = \"VCC\"\npins = [\"Q1.2\"]\n\n[[nets]]\nname = \"BASE\"\npins = [\"Q1.1\"]\n\n\
         [[nets]]\nname = \"OUT\"\npins = [\"Q1.3\", \"R1.1\"]\n\n[[nets]]\nname = \"GND\"\npins = [\"R1.2\"]\n",
    )
    .unwrap();
    std::fs::write(
        d.join("s.sim.toml"),
        "name = \"s\"\nkind = \"circuit\"\ndescription = \"pnp follower\"\n\n\
         [[drives]]\nnet = \"VCC\"\ndc = \"10V\"\n\n[[drives]]\nnet = \"BASE\"\ndc = \"9V\"\n",
    )
    .unwrap();
    d
}

#[test]
fn a_pnp_named_only_in_its_symbol_description_simulates_as_pnp() {
    let d = project();
    let out = Command::new(bin()).args(["sim", "s"]).current_dir(&d).output().unwrap();
    let text = String::from_utf8_lossy(&out.stdout);
    let v: serde_json::Value = serde_json::from_str(&text)
        .unwrap_or_else(|e| panic!("{e}: {text}{}", String::from_utf8_lossy(&out.stderr)));
    let vout = v["op"]["OUT"].as_f64().unwrap();
    assert!(vout > 9.5, "OUT is {vout} V, an NPN would sit near 8.3 V");
}
