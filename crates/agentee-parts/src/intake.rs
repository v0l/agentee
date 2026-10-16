use serde::Serialize;

#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct Bag {
    pub distributor: String,
    pub mpn: Option<String>,
    pub sku: Option<String>,
    pub qty: u64,
}

const SEPARATORS: [&str; 6] = ["\u{1d}", "<GS>", "{GS}", "^]", "\u{241d}", "<0x1D>"];
const RECORD_ENDS: [&str; 6] = ["\u{1e}", "\u{4}", "<RS>", "{RS}", "<EOT>", "{EOT}"];

fn lcsc(code: &str) -> Option<Bag> {
    let body = code.trim().strip_prefix('{')?.strip_suffix('}')?;
    let mut bag = Bag { distributor: "LCSC".into(), ..Default::default() };
    for pair in body.split(',') {
        let Some((k, v)) = pair.split_once(':') else { continue };
        let v = v.trim();
        match k.trim() {
            "pc" if !v.is_empty() => bag.sku = Some(v.to_string()),
            "pm" if !v.is_empty() => bag.mpn = Some(v.to_string()),
            "qty" => bag.qty = v.parse().ok()?,
            _ => {}
        }
    }
    (bag.qty > 0 && (bag.mpn.is_some() || bag.sku.is_some())).then_some(bag)
}

fn mh10(code: &str) -> Option<Bag> {
    let mut text = code.trim().to_string();
    for s in RECORD_ENDS {
        text = text.replace(s, "\u{1d}");
    }
    for s in SEPARATORS {
        text = text.replace(s, "\u{1d}");
    }
    let start = text.find("[)>")?;
    let fields: Vec<&str> = text[start + 3..].split('\u{1d}').map(str::trim).collect();
    let mut bag = Bag::default();
    let mut mouser = false;
    for f in fields {
        if let Some(v) = f.strip_prefix("1P") {
            bag.mpn = Some(v.to_string());
        } else if let Some(v) = f.strip_prefix("30P") {
            bag.sku = Some(v.to_string());
            bag.distributor = "DigiKey".into();
        } else if let Some(v) = f.strip_prefix('P').filter(|v| !v.is_empty()) {
            if bag.sku.is_none() {
                bag.sku = Some(v.to_string());
            }
            bag.distributor = "DigiKey".into();
        } else if let Some(v) = f.strip_prefix('Q') {
            bag.qty = v.parse().ok()?;
        } else if f.starts_with("1V") || f.starts_with("14K") {
            mouser = true;
        }
    }
    if bag.distributor.is_empty() {
        bag.distributor = if mouser { "Mouser" } else { "unknown" }.into();
    }
    (bag.qty > 0 && (bag.mpn.is_some() || bag.sku.is_some())).then_some(bag)
}

pub fn parse(code: &str) -> Option<Bag> {
    lcsc(code).or_else(|| mh10(code))
}

#[cfg(test)]
mod tests {
    use super::*;

    const DIGIKEY: &str = "[)>\u{1e}06\u{1d}P296-LM358BIDDFRCT-ND\u{1d}1PLM358BIDDFR\u{1d}K\u{1d}1K72991337\u{1d}10K85781337\u{1d}11K1\u{1d}4LPH\u{1d}Q10\u{1d}11ZPICK\u{1d}12Z15221337\u{1d}13Z361337\u{1d}20Z0000000000";
    const MOUSER: &str = "[)>\u{1e}06\u{1d}KP0-1337\u{1d}14K011\u{1d}1PMC34063ADR\u{1d}Q3\u{1d}11K073121337\u{1d}4LMX\u{1d}1VTI\u{1e}\u{4}";
    const MOUSER_OLD: &str = ">[)>06\u{1d}K21421337\u{1d}14K033\u{1d}1PLDK320ADU33R\u{1d}Q32\u{1d}11K060931337\u{1d}4LCN\u{1d}1VSTMicro";
    const LCSC: &str = "{pbn:PICK2009291337,on:SO2009291337,pc:C312270,pm:ST-1-102-A01-T000-RS,qty:2,mc:,cc:1,pdi:34421807}";

    #[test]
    fn reads_each_distributors_bag() {
        assert_eq!(
            parse(DIGIKEY),
            Some(Bag {
                distributor: "DigiKey".into(),
                mpn: Some("LM358BIDDFR".into()),
                sku: Some("296-LM358BIDDFRCT-ND".into()),
                qty: 10,
            })
        );
        let m = parse(MOUSER).unwrap();
        assert_eq!(
            (m.distributor.as_str(), m.mpn.as_deref(), m.qty),
            ("Mouser", Some("MC34063ADR"), 3)
        );
        let m = parse(MOUSER_OLD).unwrap();
        assert_eq!((m.mpn.as_deref(), m.qty), (Some("LDK320ADU33R"), 32));
        assert_eq!(
            parse(LCSC),
            Some(Bag {
                distributor: "LCSC".into(),
                mpn: Some("ST-1-102-A01-T000-RS".into()),
                sku: Some("C312270".into()),
                qty: 2,
            })
        );
    }

    #[test]
    fn a_scanner_that_types_the_separators_still_reads() {
        let typed = DIGIKEY.replace('\u{1d}', "<GS>").replace('\u{1e}', "<RS>");
        assert_eq!(parse(&typed), parse(DIGIKEY));
        assert_eq!(parse("not a bag"), None);
        assert_eq!(parse("[)>06<GS>1PLM358<GS>K12"), None);
    }
}
