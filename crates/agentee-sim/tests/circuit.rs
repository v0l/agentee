use agentee_core::circuit::CircuitResult;
use agentee_core::project::Project;

fn symbol(name: &str, reference: &str, pins: &[(&str, &str)]) -> String {
    let mut s = format!("name = \"{name}\"\nreference = \"{reference}\"\n");
    for (i, (number, pin)) in pins.iter().enumerate() {
        s += &format!(
            "[[pins]]\nnumber = \"{number}\"\nname = \"{pin}\"\ntype = \"passive\"\nat = [-7.62, {}]\nside = \"left\"\n",
            i as f64 * 2.54
        );
    }
    s
}

fn dual_opamp() -> String {
    let mut s = String::from("name = \"DUAL_OPAMP\"\nreference = \"U\"\n");
    let pins = [
        ("3", "+", "input", 1),
        ("2", "-", "input", 1),
        ("1", "", "output", 1),
        ("5", "+", "input", 2),
        ("6", "-", "input", 2),
        ("7", "", "output", 2),
        ("8", "V+", "power_in", 0),
        ("4", "V-", "power_in", 0),
    ];
    for (i, (number, name, kind, unit)) in pins.iter().enumerate() {
        s += &format!(
            "[[pins]]\nnumber = \"{number}\"\nname = \"{name}\"\ntype = \"{kind}\"\nat = [-7.62, {}]\nside = \"left\"\nunit = {unit}\n",
            i as f64 * 2.54
        );
    }
    s
}

fn symbols() -> Vec<(&'static str, String)> {
    vec![
        ("R", symbol("R", "R", &[("1", "~"), ("2", "~")])),
        ("C", symbol("C", "C", &[("1", "~"), ("2", "~")])),
        ("L", symbol("L", "L", &[("1", "~"), ("2", "~")])),
        ("D", symbol("D", "D", &[("1", "K"), ("2", "A")])),
        ("Q_NPN", symbol("Q_NPN", "Q", &[("1", "B"), ("2", "C"), ("3", "E")])),
        ("Q_NMOS", symbol("Q_NMOS", "Q", &[("1", "G"), ("2", "D"), ("3", "S")])),
        (
            "OPAMP",
            symbol("OPAMP", "U", &[("3", "+"), ("2", "-"), ("1", "~"), ("8", "V+"), ("4", "V-")]),
        ),
        ("DUAL_OPAMP", dual_opamp()),
        ("LDO", symbol("LDO", "U", &[("1", "VIN"), ("2", "GND"), ("3", "VOUT")])),
        ("CONN", symbol("CONN", "J", &[("1", "1")])),
        ("POT", symbol("POT", "RV", &[("1", "1"), ("2", "W"), ("3", "3")])),
    ]
}

fn sim(
    name: &str,
    parts: &[(&str, &str, &str)],
    nets: &[(&str, &[&str])],
    spec: &str,
) -> CircuitResult {
    let dir = std::env::temp_dir().join(format!("agentee-circuit-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("symbols")).unwrap();
    for (n, text) in symbols() {
        std::fs::write(dir.join(format!("symbols/{n}.sym.toml")), text).unwrap();
    }
    let mut sch = String::from("name = \"t\"\n");
    for (i, (r, sym, value)) in parts.iter().enumerate() {
        let (sym, unit) = match sym.split_once('#') {
            Some((s, u)) => (s, format!("unit = {u}\n")),
            None => (*sym, String::new()),
        };
        sch += &format!(
            "\n[[parts]]\nref = \"{r}\"\nsymbol = \"{sym}\"\nvalue = \"{value}\"\nat = [{}, 25.4]\n{unit}",
            25.4 * (i + 1) as f64
        );
    }
    for (n, pins) in nets {
        let pins: Vec<String> = pins.iter().map(|p| format!("\"{p}\"")).collect();
        sch += &format!("\n[[nets]]\nname = \"{n}\"\npins = [{}]\n", pins.join(", "));
    }
    std::fs::write(dir.join("t.sch.toml"), sch).unwrap();
    std::fs::write(dir.join("t.sim.toml"), format!("name = \"t\"\nkind = \"circuit\"\n{spec}"))
        .unwrap();
    let p = Project::load(&dir).unwrap();
    let _ = std::fs::remove_dir_all(&dir);
    let entry = p.sims.iter().find(|s| s.name == "t").expect("the sim loads");
    let errors: Vec<String> = entry
        .diags
        .iter()
        .filter(|d| d.severity == agentee_core::Severity::Error)
        .map(|d| d.to_string())
        .collect();
    assert!(errors.is_empty(), "{errors:#?}");
    let spec = entry.item.circuit.as_ref().expect("a circuit spec");
    agentee_sim::circuit::run(spec, "t", 0)
}

fn close(a: f64, b: f64, tol: f64) -> bool {
    (a - b).abs() <= tol
}

#[test]
fn a_divider_halves_its_input() {
    let r = sim(
        "divider",
        &[("R1", "R", "10k"), ("R2", "R", "10k")],
        &[("VIN", &["R1.1"]), ("OUT", &["R1.2", "R2.1"]), ("GND", &["R2.2"])],
        "[[drives]]\nnet = \"VIN\"\ndc = \"5V\"\n\n[[checks]]\nv = \"OUT\"\nmin = 2.49\nmax = 2.51\n\n[[checks]]\ni = \"R1\"\nmin = \"0.249mA\"\nmax = \"0.251mA\"\n",
    );
    assert!(r.passed, "{:?}", r.failures);
    assert!(close(r.op["OUT"], 2.5, 1e-6));
}

#[test]
fn an_rc_low_pass_is_3db_down_at_its_corner() {
    let r = sim(
        "rc-ac",
        &[("R1", "R", "1k"), ("C1", "C", "159.155n")],
        &[("VIN", &["R1.1"]), ("OUT", &["R1.2", "C1.1"]), ("GND", &["C1.2"])],
        "[[drives]]\nnet = \"VIN\"\nac = 1\n\n[ac]\nfrom = \"10Hz\"\nto = \"100kHz\"\npoints = 50\n\n[[checks]]\nv = \"OUT\"\nat = \"1kHz\"\nmin = -3.1\nmax = -2.9\n\n[[checks]]\nphase = \"OUT\"\nat = \"1kHz\"\nmin = -46\nmax = -44\n\n[[checks]]\nv = \"OUT\"\nat = \"100kHz\"\nmax = -39\n",
    );
    assert!(r.passed, "{:?} {:?}", r.failures, r.checks);
}

#[test]
fn an_rc_charges_to_63_percent_in_one_time_constant() {
    let r = sim(
        "rc-tran",
        &[("R1", "R", "1k"), ("C1", "C", "1u")],
        &[("VIN", &["R1.1"]), ("OUT", &["R1.2", "C1.1"]), ("GND", &["C1.2"])],
        "[[drives]]\nnet = \"VIN\"\nsteps = [[\"0s\", \"0V\"], [\"1us\", \"0V\"], [\"1.001us\", \"1V\"]]\n\n[tran]\nstop = \"5ms\"\nstep = \"10us\"\n\n[[checks]]\nv = \"OUT\"\nat = \"1.001ms\"\nmin = 0.62\nmax = 0.645\n\n[[checks]]\nv = \"OUT\"\nat = \"5ms\"\nmin = 0.99\n",
    );
    assert!(r.passed, "{:?} {:?}", r.failures, r.checks);
}

#[test]
fn a_silicon_diode_drops_about_0v65() {
    let r = sim(
        "diode",
        &[("R1", "R", "1k"), ("D1", "D", "1N4148")],
        &[("VIN", &["R1.1"]), ("A", &["R1.2", "D1.2"]), ("GND", &["D1.1"])],
        "[[drives]]\nnet = \"VIN\"\ndc = \"5V\"\n\n[[checks]]\nv = \"A\"\nmin = 0.55\nmax = 0.8\n",
    );
    assert!(r.passed, "{:?} {:?}", r.failures, r.checks);
}

#[test]
fn an_inverting_amplifier_has_gain_ten_and_clips_at_its_rails() {
    let parts = [("R1", "R", "1k"), ("R2", "R", "10k"), ("U1", "OPAMP", "TLV9001")];
    let nets: [(&str, &[&str]); 6] = [
        ("VIN", &["R1.1"]),
        ("INN", &["R1.2", "R2.1", "U1.2"]),
        ("OUT", &["R2.2", "U1.1"]),
        ("GND", &["U1.3"]),
        ("VP", &["U1.8"]),
        ("VN", &["U1.4"]),
    ];
    let common = "[[blocks]]\nref = \"U1\"\nkind = \"opamp\"\ngain = 1e5\ngbw = \"1MHz\"\n\n[[drives]]\nnet = \"VP\"\ndc = \"5V\"\n\n[[drives]]\nnet = \"VN\"\ndc = \"-5V\"\n";
    let r = sim(
        "inverting",
        &parts,
        &nets,
        &format!(
            "{common}\n[[drives]]\nnet = \"VIN\"\ndc = \"0.1V\"\nac = 1\n\n[ac]\nfrom = \"10Hz\"\nto = \"10MHz\"\n\n[[checks]]\nv = \"OUT\"\nmin = -1.01\nmax = -0.99\n\n[[checks]]\nv = \"OUT\"\nat = \"1kHz\"\nmin = 19.9\nmax = 20.1\n\n[[checks]]\nv = \"OUT\"\nat = \"1MHz\"\nmax = 0\n"
        ),
    );
    assert!(r.passed, "{:?} {:?}", r.failures, r.checks);
    let clipped = sim(
        "clip",
        &parts,
        &nets,
        &format!(
            "{common}\n[[drives]]\nnet = \"VIN\"\ndc = \"1V\"\n\n[[checks]]\nv = \"OUT\"\nmin = -5\nmax = -4.8\n"
        ),
    );
    assert!(clipped.passed, "{:?} {:?}", clipped.failures, clipped.checks);
}

#[test]
fn a_single_supply_dual_opamp_with_unnamed_outputs_settles() {
    let r = sim(
        "dual",
        &[
            ("R1", "R", "10k"),
            ("R2", "R", "10k"),
            ("U1", "DUAL_OPAMP#1", "LM358"),
            ("U1", "DUAL_OPAMP#2", "LM358"),
        ],
        &[
            ("VIN", &["U1.3"]),
            ("INN", &["U1.2", "R1.2", "R2.1"]),
            ("GND", &["R1.1", "U1.4"]),
            ("OUTA", &["U1.1", "R2.2", "U1.5"]),
            ("OUTB", &["U1.7", "U1.6"]),
            ("VP", &["U1.8"]),
        ],
        "[[blocks]]\nref = \"U1\"\nkind = \"opamp\"\ngain = 1e5\ngbw = \"1MHz\"\nheadroom = \"1.5V\"\n\n[[drives]]\nnet = \"VP\"\ndc = \"9V\"\n\n[[drives]]\nnet = \"VIN\"\ndc = \"2.2V\"\n\n[[checks]]\nv = \"OUTA\"\nmin = 4.39\nmax = 4.41\n\n[[checks]]\nv = \"OUTB\"\nmin = 4.39\nmax = 4.41\n",
    );
    assert!(r.passed, "{:?} {:?}", r.failures, r.checks);
}

#[test]
fn a_clipped_amplifier_comes_out_of_its_rail_when_the_input_crosses_zero() {
    let parts = [("R1", "R", "1k"), ("R2", "R", "10k"), ("U1", "OPAMP", "TLV9001")];
    let nets: [(&str, &[&str]); 6] = [
        ("VIN", &["R1.1"]),
        ("INN", &["R1.2", "R2.1", "U1.2"]),
        ("OUT", &["R2.2", "U1.1"]),
        ("GND", &["U1.3"]),
        ("VP", &["U1.8"]),
        ("VN", &["U1.4"]),
    ];
    let r = sim(
        "windup",
        &parts,
        &nets,
        "[[blocks]]\nref = \"U1\"\nkind = \"opamp\"\ngain = 1e5\ngbw = \"1MHz\"\n\n[[drives]]\nnet = \"VP\"\ndc = \"5V\"\n\n[[drives]]\nnet = \"VN\"\ndc = \"-5V\"\n\n[[drives]]\nnet = \"VIN\"\nsine = { amplitude = \"1V\", frequency = \"1kHz\" }\n\n[tran]\nstop = \"1ms\"\nstep = \"1us\"\n\n[[checks]]\nv = \"OUT\"\nat = \"0.25ms\"\nmax = -4.8\n\n[[checks]]\nv = \"OUT\"\nat = \"0.5ms\"\nmin = -0.2\nmax = 0.2\n",
    );
    assert!(r.passed, "{:?} {:?}", r.failures, r.checks);
}

#[test]
fn a_resistor_makes_its_thermal_noise() {
    let r = sim(
        "noise",
        &[("R1", "R", "1k"), ("R2", "R", "1Meg")],
        &[("VIN", &["R2.1"]), ("OUT", &["R1.1", "R2.2"]), ("GND", &["R1.2"])],
        "[[drives]]\nnet = \"VIN\"\nac = 1\n\n[noise]\noutput = \"OUT\"\nfrom = \"1kHz\"\nto = \"10kHz\"\n\n[[checks]]\nnoise = \"OUT\"\nat = \"1kHz\"\nmin = \"4.0n\"\nmax = \"4.1n\"\n",
    );
    assert!(r.passed, "{:?} {:?}", r.failures, r.checks);
    let rms = r.noise_rms.unwrap();
    let want = 4.07e-9 * (9000f64).sqrt();
    assert!(close(rms, want, want * 0.03), "{rms} vs {want}");
}

#[test]
fn an_ldo_regulates_then_drops_out() {
    let parts = [("U1", "LDO", "AP2112"), ("R1", "R", "100")];
    let nets: [(&str, &[&str]); 3] =
        [("VIN", &["U1.1"]), ("VOUT", &["U1.3", "R1.1"]), ("GND", &["U1.2", "R1.2"])];
    let block = "[[blocks]]\nref = \"U1\"\nkind = \"ldo\"\nvout = \"3.3V\"\ndropout = \"0.25V\"\niq = \"55uA\"\n";
    let r = sim(
        "ldo",
        &parts,
        &nets,
        &format!(
            "{block}\n[[drives]]\nnet = \"VIN\"\ndc = \"5V\"\n\n[[checks]]\nv = \"VOUT\"\nmin = 3.28\nmax = 3.31\n"
        ),
    );
    assert!(r.passed, "{:?} {:?}", r.failures, r.checks);
    let supply = r.currents["drive VIN"];
    assert!(close(supply, 0.033 + 55e-6, 0.002), "input current {supply}");
    let low = sim(
        "ldo-low",
        &parts,
        &nets,
        &format!(
            "{block}\n[[drives]]\nnet = \"VIN\"\ndc = \"3.3V\"\n\n[[checks]]\nv = \"VOUT\"\nmin = 2.95\nmax = 3.1\n"
        ),
    );
    assert!(low.passed, "{:?} {:?}", low.failures, low.checks);
}

#[test]
fn an_npn_common_emitter_biases_into_its_active_region() {
    let r = sim(
        "bjt",
        &[("R1", "R", "100k"), ("R2", "R", "1k"), ("Q1", "Q_NPN", "2N3904")],
        &[
            ("VCC", &["R1.1", "R2.1"]),
            ("B", &["R1.2", "Q1.1"]),
            ("C", &["R2.2", "Q1.2"]),
            ("GND", &["Q1.3"]),
        ],
        "[[drives]]\nnet = \"VCC\"\ndc = \"5V\"\n\n[[checks]]\nv = \"B\"\nmin = 0.6\nmax = 0.75\n\n[[checks]]\nv = \"C\"\nmin = 0.5\nmax = 1.5\n",
    );
    assert!(r.passed, "{:?} {:?}", r.failures, r.checks);
}

#[test]
fn an_nmos_switch_pulls_its_drain_low() {
    let r = sim(
        "nmos",
        &[("R1", "R", "1k"), ("Q1", "Q_NMOS", "2N7002")],
        &[("VCC", &["R1.1"]), ("D", &["R1.2", "Q1.2"]), ("G", &["Q1.1"]), ("GND", &["Q1.3"])],
        "[[drives]]\nnet = \"VCC\"\ndc = \"5V\"\n\n[[drives]]\nnet = \"G\"\ndc = \"5V\"\n\n[[checks]]\nv = \"D\"\nmax = 0.2\n",
    );
    assert!(r.passed, "{:?} {:?}", r.failures, r.checks);
}

#[test]
fn an_lc_tank_rings_at_its_resonance() {
    let r = sim(
        "lc",
        &[("R1", "R", "1"), ("L1", "L", "10u"), ("C1", "C", "1u")],
        &[
            ("VIN", &["R1.1"]),
            ("X", &["R1.2", "L1.1"]),
            ("OUT", &["L1.2", "C1.1"]),
            ("GND", &["C1.2"]),
        ],
        "[[drives]]\nnet = \"VIN\"\nac = 1\n\n[ac]\nfrom = \"10kHz\"\nto = \"100kHz\"\npoints = 400\n",
    );
    assert!(r.passed, "{:?}", r.failures);
    let g = &r.gain_db.iter().find(|c| c.name == "OUT").unwrap().values;
    let peak = (0..g.len()).max_by(|&a, &b| g[a].total_cmp(&g[b])).unwrap();
    let f0 = 1.0 / (std::f64::consts::TAU * (10e-6f64 * 1e-6).sqrt());
    let q = (10e-6f64 / 1e-6).sqrt();
    let fp = f0 * (1.0 - 1.0 / (2.0 * q * q)).sqrt();
    assert!(close(r.freqs[peak], fp, fp * 0.01), "{} vs {fp}", r.freqs[peak]);
    let at = (0..g.len())
        .min_by(|&a, &b| (r.freqs[a] - f0).abs().total_cmp(&(r.freqs[b] - f0).abs()))
        .unwrap();
    assert!(close(g[at], 20.0 * q.log10(), 0.1), "{} dB at f0", g[at]);
}

#[test]
fn a_part_with_no_model_is_an_error() {
    let dir = std::env::temp_dir().join(format!("agentee-circuit-{}-nomodel", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("symbols")).unwrap();
    for (n, text) in symbols() {
        std::fs::write(dir.join(format!("symbols/{n}.sym.toml")), text).unwrap();
    }
    std::fs::write(
        dir.join("t.sch.toml"),
        "name = \"t\"\n[[parts]]\nref = \"U1\"\nsymbol = \"LDO\"\nvalue = \"X\"\nat = [25.4, 25.4]\n[[nets]]\nname = \"GND\"\npins = [\"U1.2\"]\n",
    )
    .unwrap();
    std::fs::write(dir.join("t.sim.toml"), "name = \"t\"\nkind = \"circuit\"\n").unwrap();
    let p = Project::load(&dir).unwrap();
    let _ = std::fs::remove_dir_all(&dir);
    let entry = p.sims.iter().find(|s| s.name == "t").unwrap();
    assert!(
        entry.diags.iter().any(|d| d.message.contains("has no circuit model")),
        "{:?}",
        entry.diags
    );
}

#[test]
fn a_second_block_for_the_same_part_is_an_error() {
    let dir =
        std::env::temp_dir().join(format!("agentee-circuit-{}-twoblocks", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("symbols")).unwrap();
    for (n, text) in symbols() {
        std::fs::write(dir.join(format!("symbols/{n}.sym.toml")), text).unwrap();
    }
    std::fs::write(
        dir.join("t.sch.toml"),
        "name = \"t\"\n[[parts]]\nref = \"R1\"\nsymbol = \"R\"\nvalue = \"1k\"\nat = [25.4, 25.4]\n[[nets]]\nname = \"GND\"\npins = [\"R1.1\", \"R1.2\"]\n",
    )
    .unwrap();
    std::fs::write(
        dir.join("t.sim.toml"),
        "name = \"t\"\nkind = \"circuit\"\n[[blocks]]\nref = \"R1\"\nkind = \"resistor\"\nresistance = \"1k\"\n[[blocks]]\nref = \"R1\"\nkind = \"resistor\"\nresistance = \"2k\"\n",
    )
    .unwrap();
    let p = Project::load(&dir).unwrap();
    let _ = std::fs::remove_dir_all(&dir);
    let entry = p.sims.iter().find(|s| s.name == "t").unwrap();
    assert!(
        entry.diags.iter().any(|d| d.message.contains("already has a block")),
        "{:?}",
        entry.diags
    );
}

#[test]
fn a_half_wave_rectifier_peaks_a_diode_drop_below_the_sine() {
    let r = sim(
        "rectifier",
        &[("D1", "D", "1N4148"), ("R1", "R", "10k"), ("C1", "C", "10u")],
        &[("VIN", &["D1.2"]), ("OUT", &["D1.1", "R1.1", "C1.1"]), ("GND", &["R1.2", "C1.2"])],
        "[[drives]]\nnet = \"VIN\"\nsine = { amplitude = \"5V\", frequency = \"50Hz\" }\n\n[tran]\nstop = \"100ms\"\nstep = \"50us\"\n\n[[checks]]\nv = \"OUT\"\nat = \"85ms\"\nmin = 3.8\nmax = 4.6\n",
    );
    assert!(r.passed, "{:?} {:?}", r.failures, r.checks);
    let out = &r.waves.iter().find(|c| c.name == "OUT").unwrap().values;
    let tail: Vec<f64> =
        r.times.iter().zip(out).filter(|(t, _)| **t > 0.06).map(|(_, v)| *v).collect();
    let (lo, hi) = tail.iter().fold((f64::MAX, f64::MIN), |(a, b), &v| (a.min(v), b.max(v)));
    assert!(hi < 4.6 && hi > 4.1, "peak {hi}");
    assert!(hi - lo > 0.4 && hi - lo < 0.9, "ripple {}", hi - lo);
}

#[test]
fn a_sine_through_a_divider_reads_its_peak_rms_frequency_and_thd() {
    let r = sim(
        "divider-sine",
        &[("R1", "R", "10k"), ("R2", "R", "10k")],
        &[("VIN", &["R1.1"]), ("OUT", &["R1.2", "R2.1"]), ("GND", &["R2.2"])],
        "[[drives]]\nnet = \"VIN\"\nsine = { offset = \"2V\", amplitude = \"1V\", frequency = \"1kHz\" }\n\n[tran]\nstop = \"10ms\"\nstep = \"5us\"\n\n[[checks]]\npeak = \"OUT\"\nmin = 1.49\nmax = 1.51\n\n[[checks]]\ntrough = \"OUT\"\nfrom = \"1ms\"\nto = \"9ms\"\nmin = 0.49\nmax = 0.51\n\n[[checks]]\nrms = \"OUT\"\nmin = 0.35\nmax = 0.357\n\n[[checks]]\nfreq = \"OUT\"\nfrom = \"1ms\"\nmin = \"999Hz\"\nmax = \"1001Hz\"\n\n[[checks]]\nthd = \"OUT\"\nmax = \"0.1%\"\n",
    );
    assert!(r.passed, "{:?} {:?}", r.failures, r.checks);
    assert_eq!(r.checks[1].label, "trough(OUT) from 1ms to 9ms");
}

#[test]
fn a_span_check_needs_a_transient_and_no_at() {
    let dir = std::env::temp_dir().join(format!("agentee-circuit-bad-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("symbols")).unwrap();
    for (n, text) in symbols() {
        std::fs::write(dir.join(format!("symbols/{n}.sym.toml")), text).unwrap();
    }
    std::fs::write(
        dir.join("t.sch.toml"),
        "name = \"t\"\n[[parts]]\nref = \"R1\"\nsymbol = \"R\"\nvalue = \"1k\"\nat = [25.4, 25.4]\n[[nets]]\nname = \"A\"\npins = [\"R1.1\"]\n[[nets]]\nname = \"GND\"\npins = [\"R1.2\"]\n",
    )
    .unwrap();
    std::fs::write(
        dir.join("t.sim.toml"),
        "name = \"t\"\nkind = \"circuit\"\n[[drives]]\nnet = \"A\"\ndc = \"1V\"\n[[checks]]\npeak = \"A\"\nat = \"1ms\"\nmax = 2\n[[checks]]\nrms = \"A\"\nmax = 2\n[[checks]]\nv = \"A\"\nfrom = \"1ms\"\nmax = 2\n",
    )
    .unwrap();
    let p = Project::load(&dir).unwrap();
    let _ = std::fs::remove_dir_all(&dir);
    let errors: Vec<String> = p.sims[0].diags.iter().map(|d| d.message.clone()).collect();
    assert!(errors.iter().any(|e| e.contains("not `at`")), "{errors:?}");
    assert!(errors.iter().any(|e| e.contains("need a [tran]")), "{errors:?}");
    assert!(errors.iter().any(|e| e.contains("`from` and `to` go with")), "{errors:?}");
}

#[test]
fn a_potentiometer_divides_at_its_position_and_sags_under_load() {
    let pot = "[[drives]]\nnet = \"VIN\"\ndc = \"10V\"\n\n[[blocks]]\nref = \"RV1\"\nkind = \"potentiometer\"\nposition = 0.25\n";
    let open = sim(
        "pot",
        &[("RV1", "POT", "10k")],
        &[("VIN", &["RV1.1"]), ("W", &["RV1.2"]), ("GND", &["RV1.3"])],
        pot,
    );
    assert!(close(open.op["W"], 7.5, 1e-6), "{:?}", open.op);
    let loaded = sim(
        "pot-load",
        &[("RV1", "POT", "10k"), ("R1", "R", "7.5k")],
        &[("VIN", &["RV1.1"]), ("W", &["RV1.2", "R1.1"]), ("GND", &["RV1.3", "R1.2"])],
        pot,
    );
    let lower = 7500.0 * 7500.0 / 15000.0;
    assert!(close(loaded.op["W"], 10.0 * lower / (2500.0 + lower), 1e-6), "{:?}", loaded.op);
}
