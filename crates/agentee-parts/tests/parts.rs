use agentee_parts::offer::{Break, Offer, parse_price};
use agentee_parts::spec::{self, Spec};
use agentee_parts::{BomLine, Distributor, Options, report};
use serde_json::json;
use std::collections::BTreeMap;
use std::path::Path;

fn offer(
    dist: &str,
    sku: &str,
    mpn: &str,
    stock: u64,
    breaks: &[(u32, f64)],
    attrs: &[(&str, &str)],
) -> Offer {
    Offer {
        distributor: dist.into(),
        sku: sku.into(),
        manufacturer: "Maker".into(),
        mpn: mpn.into(),
        description: String::new(),
        stock,
        currency: "EUR".into(),
        breaks: breaks.iter().map(|&(qty, price)| Break { qty, price }).collect(),
        min: 1,
        mult: 1,
        attributes: attrs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect::<BTreeMap<_, _>>(),
        ..Default::default()
    }
}

fn mlcc(sku: &str, mpn: &str, price: f64, attrs: &[(&str, &str)]) -> Offer {
    let mut o = offer(
        "Mouser",
        sku,
        mpn,
        100_000,
        &[(1, price), (10, price / 2.0), (100, price / 10.0)],
        attrs,
    );
    o.description = "Multilayer Ceramic Capacitors MLCC - SMD/SMT".into();
    o
}

#[test]
fn buys_up_to_the_next_break_when_that_costs_less() {
    let o = offer("Mouser", "1", "R", 10_000, &[(1, 0.10), (10, 0.02), (100, 0.004)], &[]);
    let c = o.cost(7).unwrap();
    assert_eq!((c.qty, c.unit), (10, 0.02));
    let c = o.cost(60).unwrap();
    assert_eq!(c.qty, 100);
    assert!((c.total - 0.4).abs() < 1e-9);
}

#[test]
fn honours_minimum_and_multiple() {
    let mut o = offer("Farnell", "1", "C", 10_000, &[(10, 0.05)], &[]);
    o.min = 10;
    o.mult = 5;
    assert_eq!(o.order_qty(3), 10);
    assert_eq!(o.order_qty(12), 15);
    assert_eq!(o.cost(3).unwrap().total, 0.5);
}

#[test]
fn reads_prices_in_both_decimal_styles() {
    assert_eq!(parse_price("$0.10"), Some(0.10));
    assert_eq!(parse_price("0,10 €"), Some(0.10));
    assert_eq!(parse_price("1.234,56 €"), Some(1234.56));
    assert_eq!(parse_price("$1,234.56"), Some(1234.56));
    assert_eq!(parse_price("£12"), Some(12.0));
}

#[test]
fn reads_values_and_ratings() {
    assert_eq!(spec::si("4.7k"), Some(4700.0));
    assert_eq!(spec::si("4k7"), Some(4700.0));
    assert_eq!(spec::si("10 kOhms"), Some(10_000.0));
    assert!((spec::si("100n").unwrap() - 1e-7).abs() < 1e-18);
    assert!((spec::si("0.1µF").unwrap() - 1e-7).abs() < 1e-18);
    assert_eq!(spec::si("50 VDC"), Some(50.0));
    assert_eq!(spec::si("1M"), Some(1e6));
    assert_eq!(spec::percent("± 1%"), Some(1.0));
    assert_eq!(spec::percent("10 %"), Some(10.0));
    assert_eq!(spec::dielectric("NP0"), Some("C0G".into()));
    assert_eq!(spec::chip_size("C_0603_1608Metric"), Some("0603"));
}

#[test]
fn parses_a_mouser_part_search() {
    let v = json!({
        "Errors": [],
        "SearchResults": { "NumberOfResult": 1, "Parts": [{
            "MouserPartNumber": "81-GRM188R71H104KA3D",
            "Manufacturer": "Murata Electronics",
            "ManufacturerPartNumber": "GRM188R71H104KA93D",
            "Description": "Multilayer Ceramic Capacitors MLCC - SMD/SMT 0603 0.1uF 50volts X7R 10%",
            "Availability": "1,234,567 In Stock",
            "AvailabilityInStock": "1234567",
            "LifecycleStatus": null,
            "Min": "1", "Mult": "1",
            "PriceBreaks": [
                { "Quantity": 1, "Price": "0,10 €", "Currency": "EUR" },
                { "Quantity": 10, "Price": "0,032 €", "Currency": "EUR" }
            ],
            "ProductAttributes": [
                { "AttributeName": "Packaging", "AttributeValue": "Reel" },
                { "AttributeName": "Packaging", "AttributeValue": "Cut Tape" }
            ],
            "ProductDetailUrl": "https://www.mouser.ie/ProductDetail/81-GRM188R71H104KA3D",
            "SuggestedReplacement": ""
        }]}
    });
    let parts = agentee_parts::mouser::parse(&v);
    assert_eq!(parts.len(), 1);
    let p = &parts[0];
    assert_eq!(p.sku, "81-GRM188R71H104KA3D");
    assert_eq!(p.stock, 1_234_567);
    assert_eq!(p.currency, "EUR");
    assert_eq!(p.breaks[1], Break { qty: 10, price: 0.032 });
    assert_eq!(p.attributes["Packaging"], "Reel, Cut Tape");
    assert_eq!(p.replacement, None);
    assert!(p.active());
}

#[test]
fn parses_a_farnell_search_and_prices_in_the_store_currency() {
    let v = json!({ "manufacturerPartNumberSearchReturn": { "numberOfResults": 1, "products": [{
        "sku": "8820023",
        "displayName": "SMD Multilayer Ceramic Capacitor, 0.1 µF, 50 V, 0603 [1608 Metric]",
        "productStatus": "STOCKED",
        "translatedManufacturerPartNumber": "GRM188R71H104KA93D",
        "brandName": "MURATA",
        "translatedMinimumOrderQuality": 10,
        "stock": { "level": 52000 },
        "prices": [{ "from": 10, "to": 99, "cost": 0.0441 }, { "from": 100, "to": 499, "cost": 0.0213 }],
        "attributes": [
            { "attributeLabel": "Capacitance", "attributeValue": "0.1", "attributeUnit": "µF" },
            { "attributeLabel": "Capacitor Case / Package", "attributeValue": "0603 [1608 Metric]" }
        ]
    }]}});
    let parts = agentee_parts::farnell::parse(&v, "ie.farnell.com", "EUR");
    let p = &parts[0];
    assert_eq!((p.sku.as_str(), p.stock, p.min), ("8820023", 52_000, 10));
    assert_eq!(p.manufacturer, "MURATA");
    assert_eq!(p.attributes["Capacitance"], "0.1µF");
    assert_eq!(p.cost(3).unwrap().total, 0.441);
    assert_eq!(agentee_parts::farnell::store_currency("uk.farnell.com"), "GBP");
    assert_eq!(agentee_parts::farnell::store_currency("www.newark.com"), "USD");
}

#[test]
fn takes_farnell_value_units_from_the_display_name() {
    let product = |name: &str, label: &str, value: &str, volts: &str| {
        json!({ "sku": "1", "displayName": name, "attributes": [
            { "attributeLabel": label, "attributeValue": value },
            { "attributeLabel": "Voltage(DC)", "attributeValue": volts }
        ]})
    };
    let v = json!({ "premierFarnellPartNumberReturn": { "products": [
        product("MURATA - GRM1885C1H331JA01D - SMD Multilayer Ceramic Capacitor, 330 pF, 50 V, 0603 [1608 Metric]", "Capacitance", "330", "50"),
        product("KEMET - C1210C225K1RACTU - SMD Multilayer Ceramic Capacitor, 2.2 \u{b5}F, 100 V, 1210 [3225 Metric]", "Capacitance", "2.2", "100"),
        product("YAGEO - RC0603FR-0710KL - SMD Chip Resistor, 10 kohm, \u{b1} 1%, 100 mW, 0603 [1608 Metric]", "Resistance", "10", ""),
        product("YAGEO - RC0603FR-07100RL - SMD Chip Resistor, 100 ohm, \u{b1} 1%, 100 mW, 0603 [1608 Metric]", "Resistance", "100", ""),
        product("MURATA - GRM188R61A106KE69D - SMD Multilayer Ceramic Capacitor, 10 \u{b5}F, 10 V, 0603 [1608 Metric]", "Capacitance", "10", "10"),
    ]}});
    let parts = agentee_parts::farnell::parse(&v, "ie.farnell.com", "EUR");
    let value = |i: usize| {
        let p: &agentee_parts::offer::Offer = &parts[i];
        let a = p.attributes.get("Capacitance").or(p.attributes.get("Resistance")).unwrap();
        agentee_parts::spec::si(a).unwrap()
    };
    assert!((value(0) - 330e-12).abs() < 1e-15);
    assert!((value(1) - 2.2e-6).abs() < 1e-12);
    assert!((value(2) - 10e3).abs() < 1e-6);
    assert!((value(3) - 100.0).abs() < 1e-9);
    assert!((value(4) - 10e-6).abs() < 1e-12);
    assert_eq!(parts[4].attributes["Voltage(DC)"], "10");
}

#[test]
fn a_ceramic_must_keep_its_value_package_voltage_and_dielectric() {
    let current = mlcc(
        "c",
        "GRM188R71H104KA93D",
        0.1,
        &[
            ("Capacitance", "0.1 uF"),
            ("Voltage Rating DC", "50 VDC"),
            ("Dielectric", "X7R"),
            ("Tolerance", "10 %"),
            ("Case Code - in", "0603"),
        ],
    );
    let s = spec::classify("C11", "100n", "C_0603_1608Metric", Some(&current));
    assert!(
        matches!(&s, Spec::Capacitor { volts: Some(v), dielectric: Some(d), .. } if *v == 50.0 && d == "X7R")
    );
    let ok = mlcc(
        "a",
        "CL10B104KB8NNNC",
        0.05,
        &[
            ("Capacitance", "100 nF"),
            ("Voltage Rating DC", "50 VDC"),
            ("Dielectric", "X7R"),
            ("Tolerance", "10 %"),
            ("Case Code - in", "0603"),
        ],
    );
    let higher_v = mlcc(
        "b",
        "X",
        0.05,
        &[
            ("Capacitance", "100 nF"),
            ("Voltage Rating DC", "100 VDC"),
            ("Dielectric", "X7S"),
            ("Tolerance", "10 %"),
            ("Case Code - in", "0603"),
        ],
    );
    let x5r = mlcc(
        "d",
        "X",
        0.05,
        &[
            ("Capacitance", "100 nF"),
            ("Voltage Rating DC", "50 VDC"),
            ("Dielectric", "X5R"),
            ("Tolerance", "10 %"),
            ("Case Code - in", "0603"),
        ],
    );
    let low_v = mlcc(
        "e",
        "X",
        0.05,
        &[
            ("Capacitance", "100 nF"),
            ("Voltage Rating DC", "25 VDC"),
            ("Dielectric", "X7R"),
            ("Tolerance", "10 %"),
            ("Case Code - in", "0603"),
        ],
    );
    let small = mlcc(
        "f",
        "X",
        0.05,
        &[
            ("Capacitance", "100 nF"),
            ("Voltage Rating DC", "50 VDC"),
            ("Dielectric", "X7R"),
            ("Tolerance", "10 %"),
            ("Capacitor Case / Package", "0201 [0603 Metric]"),
        ],
    );
    let mut tant = ok.clone();
    tant.description = "Tantalum Capacitors - Solid SMD".into();
    assert!(spec::matches(&s, &ok));
    assert!(spec::matches(&s, &higher_v));
    assert!(!spec::matches(&s, &x5r), "X5R is not an X7R substitute");
    assert!(!spec::matches(&s, &low_v), "25 V is under the 50 V rating");
    assert!(!spec::matches(&s, &small), "0201 imperial is 0603 metric");
    assert!(!spec::matches(&s, &tant));
}

#[test]
fn a_c0g_stays_c0g() {
    let s = Spec::Capacitor {
        farads: 330e-12,
        volts: Some(50.0),
        dielectric: Some("C0G".into()),
        tolerance: Some(5.0),
        package: "0603".into(),
    };
    let x7r = mlcc(
        "a",
        "X",
        0.01,
        &[
            ("Capacitance", "330 pF"),
            ("Voltage Rating DC", "50 VDC"),
            ("Dielectric", "X7R"),
            ("Tolerance", "5 %"),
            ("Case Code - in", "0603"),
        ],
    );
    let np0 = mlcc(
        "b",
        "X",
        0.01,
        &[
            ("Capacitance", "330 pF"),
            ("Voltage Rating DC", "50 VDC"),
            ("Dielectric", "NP0"),
            ("Tolerance", "5 %"),
            ("Case Code - in", "0603"),
        ],
    );
    assert!(!spec::matches(&s, &x7r));
    assert!(spec::matches(&s, &np0));
}

#[test]
fn a_resistor_search_skips_thermistors_and_looser_tolerance() {
    let s = spec::classify("R8", "10k", "R_0603_1608Metric", None);
    assert_eq!(s, Spec::Resistor { ohms: 10_000.0, tolerance: 1.0, package: "0603".into() });
    let mut good = offer(
        "Mouser",
        "a",
        "RC0603FR-0710KL",
        1,
        &[(1, 0.1)],
        &[("Resistance", "10 kOhms"), ("Tolerance", "1 %"), ("Case Code - in", "0603")],
    );
    good.description = "Thick Film Resistors - SMD 1/10W 10K ohm 1% 0603".into();
    let mut ntc = good.clone();
    ntc.description = "NTC Thermistors 10K 0603".into();
    let mut loose = good.clone();
    loose.attributes.insert("Tolerance".into(), "5 %".into());
    assert!(spec::matches(&s, &good));
    assert!(!spec::matches(&s, &ntc));
    assert!(!spec::matches(&s, &loose));
}

#[test]
fn generic_discretes_match_by_name_and_package() {
    let s = spec::classify("D1", "S1D", "D_SMA", None);
    assert_eq!(s, Spec::Generic { name: "S1D".into(), package: "SMA".into() });
    let sma =
        offer("Mouser", "a", "S1D-E3/61T", 1, &[(1, 0.1)], &[("Package / Case", "DO-214AC-2")]);
    let smb =
        offer("Mouser", "b", "S1DB-13-F", 1, &[(1, 0.1)], &[("Package / Case", "SMB (DO-214AA)")]);
    assert!(spec::matches(&s, &sma));
    assert!(!spec::matches(&s, &smb));
    let q = spec::classify("Q4", "2N7002", "SOT-23", None);
    let five = offer("Mouser", "c", "2N7002DW", 1, &[(1, 0.1)], &[("Package / Case", "SOT-23-5")]);
    let three =
        offer("Mouser", "d", "2N7002K-7", 1, &[(1, 0.1)], &[("Package / Case", "SOT-23-3")]);
    assert!(!spec::matches(&q, &five));
    assert!(spec::matches(&q, &three));
    assert_eq!(spec::classify("U1", "LM5164", "HSOP-8", None), Spec::Specific);
    assert_eq!(spec::classify("C1", "47u/100V", "CP_Elec_10x10.5", None), Spec::Specific);
}

struct Fake {
    name: &'static str,
    parts: Vec<Offer>,
    search: Vec<Offer>,
}

impl Distributor for Fake {
    fn name(&self) -> &'static str {
        self.name
    }
    fn by_mpn(&self, mpns: &[String]) -> Result<Vec<Offer>, String> {
        Ok(self.parts.iter().filter(|o| mpns.iter().any(|m| m == &o.mpn)).cloned().collect())
    }
    fn search(&self, _: &str) -> Result<Vec<Offer>, String> {
        Ok(self.search.clone())
    }
}

#[test]
fn reports_the_cheapest_stocked_offer_and_cheaper_equivalents() {
    let attrs = [
        ("Capacitance", "100 nF"),
        ("Voltage Rating DC", "50 VDC"),
        ("Dielectric", "X7R"),
        ("Tolerance", "10 %"),
        ("Case Code - in", "0603"),
    ];
    let mouser = Fake {
        name: "Mouser",
        parts: vec![mlcc("81-GRM", "GRM188R71H104KA93D", 0.10, &attrs)],
        search: vec![
            mlcc("187-CL10", "CL10B104KB8NNNC", 0.04, &attrs),
            mlcc(
                "x-25v",
                "BAD",
                0.001,
                &[
                    ("Capacitance", "100 nF"),
                    ("Voltage Rating DC", "16 VDC"),
                    ("Dielectric", "X7R"),
                    ("Tolerance", "10 %"),
                    ("Case Code - in", "0603"),
                ],
            ),
        ],
    };
    let mut farnell_offer = mlcc("8820023", "GRM188R71H104KA93D", 0.08, &attrs);
    farnell_offer.distributor = "Farnell".into();
    let farnell = Fake { name: "Farnell", parts: vec![farnell_offer], search: vec![] };
    let lines = vec![
        BomLine {
            refs: vec!["C11".into(), "C13".into()],
            value: "100n".into(),
            footprint: "C_0603_1608Metric".into(),
            manufacturer: Some("Murata".into()),
            mpn: Some("GRM188R71H104KA93D".into()),
            ..Default::default()
        },
        BomLine {
            refs: vec!["U1".into()],
            value: "LM5164".into(),
            footprint: "HSOP-8".into(),
            manufacturer: None,
            mpn: Some("LM5164DDAR".into()),
            ..Default::default()
        },
    ];
    let r = report(
        &lines,
        &[&mouser, &farnell],
        &Options { boards: 1, alternatives: true, max_alternatives: 3 },
    );
    let caps = &r.lines[0];
    assert_eq!(caps.need, 2);
    assert_eq!(caps.chosen.as_ref().unwrap().distributor, "Farnell");
    let chosen = caps.chosen.as_ref().unwrap();
    assert_eq!(chosen.breaks.len(), 3);
    assert_eq!((chosen.breaks[1].qty, chosen.min, chosen.mult), (10, 1, 1));
    let alt = &caps.alternatives[0];
    assert_eq!(alt.part.mpn, "CL10B104KB8NNNC");
    assert!((alt.saves.unwrap() - (0.16 - 0.08)).abs() < 1e-9);
    assert!(caps.alternatives.iter().all(|a| a.part.mpn != "BAD"));
    assert_eq!(r.lines[1].notes, vec!["not found at Mouser or Farnell".to_string()]);
    let t = &r.totals["EUR"];
    assert_eq!(t.lines_priced, 1);
    assert!((t.chosen - 0.16).abs() < 1e-9 && (t.cheapest - 0.08).abs() < 1e-9);
}

#[test]
fn the_bom_has_one_line_per_part_number_and_skips_what_is_not_fitted() {
    let dir = std::env::temp_dir().join(format!("agentee-parts-bom-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("symbols")).unwrap();
    std::fs::create_dir_all(dir.join("footprints")).unwrap();
    let lna = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/lna");
    for f in ["symbols/R.sym.toml", "footprints/R_0402_1005Metric.fp.toml"] {
        std::fs::copy(lna.join(f), dir.join(f)).unwrap();
    }
    std::fs::write(
        dir.join("t.sch.toml"),
        "name = \"t\"\n[[parts]]\nref = \"R1\"\nsymbol = \"R\"\nvalue = \"10k\"\nat = [10.16, 10.16]\nfields = { mfr = \"Yageo\", mpn = \"RC0402FR-0710KL\" }\n[[parts]]\nref = \"R2\"\nsymbol = \"R\"\nvalue = \"10k\"\nat = [20.32, 10.16]\nfields = { mfr = \"Yageo\", mpn = \"RC0402FR-0710KL\" }\n[[parts]]\nref = \"R10\"\nsymbol = \"R\"\nvalue = \"10k\"\nat = [50.8, 10.16]\nfields = { mfr = \"Yageo\", mpn = \"RC0402FR-0710KL\" }\n[[parts]]\nref = \"R3\"\nsymbol = \"R\"\nvalue = \"1k\"\nat = [30.48, 10.16]\n[[parts]]\nref = \"R4\"\nsymbol = \"R\"\nvalue = \"1k\"\nat = [40.64, 10.16]\nfields = { assembly = \"no\" }\n",
    )
    .unwrap();
    let p = agentee_core::Project::load(&dir).unwrap();
    let lines = agentee_parts::bom(&p.schematics[0].item);
    assert_eq!(lines.len(), 2);
    assert_eq!(lines[0].refs, vec!["R1", "R2", "R10"]);
    assert_eq!(lines[0].mpn.as_deref(), Some("RC0402FR-0710KL"));
    assert_eq!(lines[1].refs, vec!["R3"]);
    assert_eq!(lines[1].mpn, None);
}

#[test]
fn a_maker_written_before_the_part_number_is_split_off() {
    let lna = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/lna");
    let p = agentee_core::Project::load(&lna).unwrap();
    let lines = agentee_parts::bom(&p.schematics[0].item);
    let c4 = lines.iter().find(|l| l.refs == ["C4"]).unwrap();
    assert_eq!(c4.manufacturer.as_deref(), Some("Murata"));
    assert_eq!(c4.mpn.as_deref(), Some("GRM155R61A105KE15D"));
    let u1 = lines.iter().find(|l| l.refs == ["U1"]).unwrap();
    assert_eq!(u1.manufacturer.as_deref(), Some("Qorvo"));
    assert_eq!(u1.mpn.as_deref(), Some("SPF5189Z"));
    let c6 = lines.iter().find(|l| l.refs == ["C6"]).unwrap();
    assert_eq!(c6.manufacturer, None);
    assert_eq!(c6.mpn.as_deref(), Some("10 uF X5R 10 V 0402"));
}

struct Broken;

impl Distributor for Broken {
    fn name(&self) -> &'static str {
        "Mouser"
    }
    fn by_mpn(&self, _: &[String]) -> Result<Vec<Offer>, String> {
        Err("Mouser: Invalid unique identifier. (API Key)".into())
    }
    fn search(&self, _: &str) -> Result<Vec<Offer>, String> {
        panic!("searched after the key was refused")
    }
}

#[test]
fn a_refused_key_is_reported_once_and_stops_the_searches() {
    let lines = vec![BomLine {
        refs: vec!["R10".into(), "R8".into()],
        value: "10k".into(),
        footprint: "R_0603_1608Metric".into(),
        manufacturer: None,
        mpn: Some("RC0603FR-0710KL".into()),
        ..Default::default()
    }];
    let r = report(&lines, &[&Broken], &Options::default());
    assert_eq!(r.errors, vec!["Mouser: Invalid unique identifier. (API Key)".to_string()]);
    assert_eq!(r.lines[0].notes, vec!["not looked up, every distributor call failed".to_string()]);
}

#[test]
fn reads_the_dc_rating_past_an_empty_ac_one() {
    let attrs = |v: &'static str| {
        vec![
            ("Capacitance", "0.1µF"),
            ("Capacitance Tolerance", "± 10%"),
            ("Capacitor Case / Package", "0603 [1608 Metric]"),
            ("Dielectric Characteristic", "X7R"),
            ("Voltage(AC)", "-"),
            ("Voltage(DC)", v),
        ]
    };
    let current = mlcc("a", "GRM188R71H104KA93D", 0.1, &attrs("50"));
    let spec = agentee_parts::spec::classify("C1", "100n", "C_0603_1608Metric", Some(&current));
    let agentee_parts::spec::Spec::Capacitor { volts, .. } = &spec else { panic!("{spec:?}") };
    assert_eq!(*volts, Some(50.0));
    let lower = mlcc("b", "0603B104K250CT", 0.05, &attrs("25"));
    assert!(!agentee_parts::spec::matches(&spec, &lower));
    assert!(agentee_parts::spec::matches(
        &spec,
        &mlcc("c", "C0603C104K5RACTU", 0.05, &attrs("50"))
    ));
}

#[test]
fn order_sheets_put_what_each_site_lacks_last() {
    let lines = vec![
        BomLine {
            refs: vec!["C1".into()],
            value: "100n".into(),
            footprint: "C_0603_1608Metric".into(),
            mpn: Some("CAP".into()),
            ..Default::default()
        },
        BomLine {
            refs: vec!["J1".into(), "J2".into()],
            value: "XH".into(),
            footprint: "JST_XH".into(),
            mpn: Some("HEADER".into()),
            buy_with: vec![("HOUSING".into(), 1), ("CRIMP".into(), 2)],
            ..Default::default()
        },
        BomLine {
            refs: vec!["U1".into()],
            value: "MODULE".into(),
            mpn: Some("MODULE".into()),
            spares: Some(0),
            ..Default::default()
        },
        BomLine {
            refs: vec!["J3".into()],
            value: "USB-C".into(),
            mpn: Some("USBC".into()),
            lcsc: Some("C165948".into()),
            ..Default::default()
        },
    ];
    let items = agentee_parts::order::items(&lines, 1, true);
    let qty: Vec<(Option<&str>, u32)> = items.iter().map(|i| (i.mpn.as_deref(), i.qty)).collect();
    assert_eq!(
        qty,
        vec![
            (Some("CAP"), 10),
            (Some("HEADER"), 2),
            (Some("MODULE"), 1),
            (Some("USBC"), 1),
            (Some("HOUSING"), 3),
            (Some("CRIMP"), 5),
        ]
    );
    let mouser = Fake {
        name: "Mouser",
        parts: vec![
            offer("Mouser", "m-cap", "CAP", 1000, &[(1, 0.10), (10, 0.02)], &[]),
            offer("Mouser", "m-hdr", "HEADER", 0, &[(1, 0.20)], &[]),
            offer("Mouser", "m-mod", "MODULE", 50, &[(1, 5.0)], &[]),
            offer("Mouser", "m-hsg", "HOUSING", 500, &[(1, 0.05)], &[]),
            offer("Mouser", "m-crimp", "CRIMP", 500, &[(1, 0.04)], &[]),
        ],
        search: vec![],
    };
    let farnell = Fake {
        name: "Farnell",
        parts: vec![
            offer("Farnell", "f-cap", "CAP", 1000, &[(1, 0.05)], &[]),
            offer("Farnell", "f-hdr", "HEADER", 100, &[(1, 0.25)], &[]),
            offer("Farnell", "f-crimp", "CRIMP", 500, &[(1, 0.03)], &[]),
        ],
        search: vec![],
    };
    let (rows, errors) = agentee_parts::order::plan(items, &[&mouser, &farnell]);
    assert!(errors.is_empty());
    let source: Vec<Option<&str>> = rows.iter().map(|r| r.source.as_deref()).collect();
    assert_eq!(
        source,
        vec![
            Some("Mouser"),
            Some("Farnell"),
            Some("Mouser"),
            None,
            Some("Mouser"),
            Some("Farnell")
        ]
    );
    let order = |site: &str| -> Vec<(String, String)> {
        agentee_parts::order::sorted_for(&rows, site)
            .iter()
            .map(|r| (r.item.mpn.clone().unwrap(), r.status(site)))
            .collect()
    };
    let s = |a: &str, b: &str| (a.to_string(), b.to_string());
    assert_eq!(
        order("Mouser"),
        vec![
            s("CAP", "order"),
            s("HOUSING", "order"),
            s("MODULE", "order"),
            s("CRIMP", "buying at Farnell"),
            s("HEADER", "short, 0 in stock"),
            s("USBC", "not listed"),
        ]
    );
    assert_eq!(
        order("Farnell"),
        vec![
            s("HEADER", "order"),
            s("CRIMP", "order"),
            s("CAP", "buying at Mouser"),
            s("HOUSING", "not listed"),
            s("USBC", "not listed"),
            s("MODULE", "not listed"),
        ]
    );
    assert_eq!(rows[3].elsewhere(), "LCSC C165948");
    let sheet = agentee_parts::order::site_sheet(&rows, "Farnell");
    assert!(sheet.contains(",order: for J1 J2,"), "{sheet}");
    let sheet = agentee_parts::order::site_sheet(&rows, "Mouser");
    let second = sheet.lines().nth(1).unwrap();
    assert!(second.starts_with("CAP,CAP,m-cap,Maker,order: 100n C1,10,"), "{second}");
}

#[test]
fn the_cost_table_prices_each_board_count_from_the_breaks() {
    let resistor =
        offer("Mouser", "603-R", "RC0603", 1_000, &[(1, 0.10), (10, 0.02), (100, 0.005)], &[]);
    let mut scarce = offer("Mouser", "595-U", "CHIP", 15, &[(1, 2.00), (10, 1.50)], &[]);
    scarce.manufacturer = "TI".into();
    let mut farnell = offer("Farnell", "123", "CHIP", 5_000, &[(1, 2.50), (100, 1.00)], &[]);
    farnell.manufacturer = "TI".into();
    let mouser = Fake { name: "Mouser", parts: vec![resistor, scarce], search: vec![] };
    let farnell = Fake { name: "Farnell", parts: vec![farnell], search: vec![] };
    let line = |refs: &[&str], mpn: Option<&str>| BomLine {
        refs: refs.iter().map(|r| r.to_string()).collect(),
        value: "x".into(),
        footprint: "f".into(),
        mpn: mpn.map(String::from),
        ..Default::default()
    };
    let lines = vec![
        line(&["R1", "R2", "R3"], Some("RC0603")),
        line(&["U1"], Some("CHIP")),
        line(&["J1"], None),
    ];
    let r = report(
        &lines,
        &[&mouser, &farnell],
        &Options { boards: 100, alternatives: false, max_alternatives: 0 },
    );
    let c = agentee_parts::cost::of("t", &[100, 1, 10, 10], &r);
    assert_eq!(c.boards, vec![1, 10, 100]);
    let cost = |row: usize, k: usize| {
        c.rows[row].costs[k].as_ref().map(|b| (b.distributor.clone(), b.buy, b.total))
    };
    assert_eq!(cost(0, 0).map(|x| x.1), Some(10), "three resistors cost less as ten");
    assert!((cost(0, 0).unwrap().2 - 0.2).abs() < 1e-9);
    assert_eq!(cost(1, 1).map(|x| x.0), Some("Mouser".to_string()));
    assert_eq!(cost(1, 2).map(|x| x.0), Some("Farnell".to_string()), "Mouser holds 15");
    assert!(c.rows[2].costs.iter().all(Option::is_none));
    assert!(c.rows[2].notes.contains(&"no mpn".to_string()));
    let total = |b: u32| c.totals.iter().find(|t| t.boards == b).unwrap();
    assert_eq!((total(1).priced, total(1).unpriced), (2, 1));
    assert!((total(100).total - (1.5 + 100.0)).abs() < 1e-9, "{}", total(100).total);
    assert!((total(100).per_board - 1.015).abs() < 1e-9);
    let t = agentee_parts::cost::text(&c);
    assert!(t.contains("per board EUR"), "{t}");
    assert!(t.contains("* bought from another distributor"), "{t}");
}
