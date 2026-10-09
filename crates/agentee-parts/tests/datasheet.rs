use agentee_parts::datasheet::{self, Cache, Passage};
use std::path::PathBuf;

fn scratch(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("agentee-ds-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

const PDF: &[u8] = b"%PDF-1.4\n%fake\n";

#[test]
fn finds_a_pin_row_by_whole_word_and_case() {
    let pages = vec![
        "Contents\n 11.1 STALLGUARD4 VS. STALLGUARD2 .... 55\n".to_string(),
        "Pin   Number   Description\nVSA    5        Analog supply\nVS     22, 28   Motor supply voltage\nGND    3        Ground\n".to_string(),
    ];
    let hits = datasheet::find(&pages, "VS", 0);
    assert_eq!(
        hits,
        vec![
            Passage { page: 1, line: 2, text: " 11.1 STALLGUARD4 VS. STALLGUARD2 .... 55".into() },
            Passage { page: 2, line: 3, text: "VS     22, 28   Motor supply voltage".into() },
        ]
    );
    let around = datasheet::find(&pages, "VS", 1);
    assert_eq!(around[1].line, 2);
    assert_eq!(around[1].text.lines().count(), 3);
    assert_eq!(datasheet::find(&pages, "motor supply", 0).len(), 1);
    assert!(datasheet::find(&pages, "vs", 0).len() >= 2);
}

#[test]
fn reads_the_revision_and_skips_the_contents() {
    let pages = vec![
        "TPS7A91\n6 Revision history . . . . . . 13\nSBVS282A – DECEMBER 2016 – REVISED SEPTEMBER 2026\n".to_string(),
    ];
    assert_eq!(
        datasheet::revision(&pages).as_deref(),
        Some("SBVS282A – DECEMBER 2016 – REVISED SEPTEMBER 2026")
    );
}

#[test]
fn caches_by_maker_and_part_number_and_keeps_pinned_copies() {
    let dir = scratch("cache");
    let cache = Cache::open(Some(&dir)).unwrap();
    assert!(cache.store(None, "X", b"<html>", None, "test").is_err());
    let (e, c) = cache
        .store(
            Some("Texas Instruments"),
            "TPS7A9101DSKR",
            PDF,
            Some("https://ti.com/a.pdf"),
            "symbol",
        )
        .unwrap();
    assert!(e.dir.ends_with("texas-instruments/tps7a9101dskr"));
    assert!(e.pdf(&c).exists());
    assert_eq!(c.sha256, datasheet::sha256(PDF));
    let (_, other) = cache.store(None, "AP2112K-3.3TRG1", b"%PDF-1.5 other", None, "user").unwrap();
    assert_eq!(cache.find("tps7a9101-dskr").len(), 1);
    assert_eq!(cache.find("TPS7A9101").len(), 1);
    assert!(cache.find("1").is_empty());
    assert!(cache.by_hash(&c.sha256[..12]).is_some());
    cache.pin(&c.sha256, "library:LDO", true).unwrap();
    let purged = cache.purge().unwrap();
    assert_eq!((purged.removed, purged.kept), (1, 1));
    assert!(cache.by_hash(&other.sha256).is_none());
    assert!(cache.find("TPS7A9101DSKR").len() == 1);
    cache.pin(&c.sha256, "library:LDO", false).unwrap();
    assert_eq!(cache.purge().unwrap().removed, 1);
    assert!(cache.entries().is_empty());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn urls_without_a_scheme_or_with_entities_are_fetchable() {
    assert_eq!(
        datasheet::with_scheme("www.qorvo.com/products/d/da001910"),
        "https://www.qorvo.com/products/d/da001910"
    );
    assert_eq!(datasheet::with_scheme("https://a.com/x?a=1&amp;b=2"), "https://a.com/x?a=1&b=2");
    assert!(datasheet::is_url("www.ti.com/lit/ds/symlink/tps7a91.pdf"));
    assert!(!datasheet::is_url("~"));
    assert!(!datasheet::is_url("10 uF X5R"));
}

#[test]
fn today_is_an_iso_date() {
    let t = datasheet::today();
    assert_eq!(t.len(), 10);
    assert!(t.starts_with("20"));
    assert_eq!(&t[4..5], "-");
}
