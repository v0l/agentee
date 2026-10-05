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
        },
        BomLine {
            refs: vec!["U1".into()],
            value: "LM5164".into(),
            footprint: "HSOP-8".into(),
            manufacturer: None,
            mpn: Some("LM5164DDAR".into()),
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
    }];
    let r = report(&lines, &[&Broken], &Options::default());
    assert_eq!(r.errors, vec!["Mouser: Invalid unique identifier. (API Key)".to_string()]);
    assert_eq!(r.lines[0].notes, vec!["not looked up, every distributor call failed".to_string()]);
}
