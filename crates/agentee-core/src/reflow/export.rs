use super::solve::Report;
use std::fmt::Write;

pub fn csv(r: &Report) -> Option<String> {
    let p = r.profile.as_ref()?;
    let mut s = String::new();
    let _ =
        writeln!(s, "# reflow {} paste {} liquidus {:.0} C", r.reflow, r.paste.name, p.liquidus_c);
    let _ = writeln!(
        s,
        "# limiter {} limit {:.0} C margin {:.1} C; coldest joint {} peaks {:.1} C needs {:.0} C",
        p.limiter,
        p.limiter_limit_c,
        p.limiter_margin_c,
        p.coldest_joint,
        p.coldest_joint_peak_c,
        p.required_joint_peak_c
    );
    let _ = writeln!(
        s,
        "time_s,oven_target_c,board_c,coldest_joint_c,hottest_part_c,liquidus_c,limit_c,limiter"
    );
    let n = p.series.time_s.len();
    for i in 0..n {
        let _ = writeln!(
            s,
            "{:.1},{:.1},{:.1},{:.1},{:.1},{:.0},{:.0},{}",
            p.series.time_s[i],
            p.series.oven_c[i],
            p.series.board_c[i],
            p.series.coldest_c.get(i).copied().unwrap_or(0.0),
            p.series.hottest_c.get(i).copied().unwrap_or(0.0),
            p.liquidus_c,
            p.limiter_limit_c,
            p.limiter
        );
    }
    Some(s)
}

fn escape(t: &str) -> String {
    t.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
}

pub fn svg(r: &Report) -> Option<String> {
    let p = r.profile.as_ref()?;
    let (w, h) = (900.0, 540.0);
    let (ml, mr, mt, mb) = (60.0, 20.0, 50.0, 50.0);
    let t_max = p.series.time_s.last().copied().unwrap_or(1.0).max(1.0);
    let y_max = (p.oven_peak_c.max(p.limiter_limit_c) + 20.0 / 1.0).ceil();
    let x = |t: f64| ml + (w - ml - mr) * t / t_max;
    let y = |c: f64| h - mb - (h - mt - mb) * c / y_max;
    let mut s = String::new();
    let _ = write!(
        s,
        "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{w}\" height=\"{h}\" viewBox=\"0 0 {w} {h}\" font-family=\"sans-serif\" font-size=\"12\">"
    );
    let _ = write!(s, "<rect width=\"{w}\" height=\"{h}\" fill=\"#ffffff\"/>");
    let _ = write!(
        s,
        "<text x=\"{ml}\" y=\"20\" font-size=\"15\" font-weight=\"bold\">{} reflow, {} (limiter {} at {:.0} C)</text>",
        escape(&r.reflow),
        escape(&r.paste.name),
        escape(&p.limiter),
        p.limiter_limit_c
    );
    let band = |s: &mut String, lo: f64, hi: f64, fill: &str, label: &str| {
        let _ = write!(
            s,
            "<rect x=\"{ml}\" y=\"{:.1}\" width=\"{:.1}\" height=\"{:.1}\" fill=\"{fill}\" opacity=\"0.35\"/>",
            y(hi),
            w - ml - mr,
            y(lo) - y(hi)
        );
        let _ = write!(
            s,
            "<text x=\"{:.1}\" y=\"{:.1}\" fill=\"#333\">{}</text>",
            ml + 6.0,
            y(hi) + 13.0,
            escape(label)
        );
    };
    band(
        &mut s,
        p.required_joint_peak_c,
        p.peak_window_c[1].min(p.limiter_limit_c),
        "#7bc47f",
        "peak window",
    );
    band(&mut s, p.preheat_c[0], p.preheat_c[1], "#f4d35e", "preheat and soak");
    let _ = write!(
        s,
        "<rect x=\"{ml}\" y=\"{mt}\" width=\"{:.1}\" height=\"{:.1}\" fill=\"none\" stroke=\"#444\"/>",
        w - ml - mr,
        h - mt - mb
    );
    let line = |s: &mut String, c: f64, color: &str, label: &str, dash: &str| {
        let _ = write!(
            s,
            "<line x1=\"{ml}\" x2=\"{:.1}\" y1=\"{:.1}\" y2=\"{:.1}\" stroke=\"{color}\" stroke-dasharray=\"{dash}\"/>",
            w - mr,
            y(c),
            y(c)
        );
        let _ = write!(
            s,
            "<text x=\"{:.1}\" y=\"{:.1}\" fill=\"{color}\" text-anchor=\"end\">{}</text>",
            w - mr - 4.0,
            y(c) - 4.0,
            escape(label)
        );
    };
    line(&mut s, p.liquidus_c, "#1d70b8", &format!("liquidus {:.0} C", p.liquidus_c), "6 3");
    line(
        &mut s,
        p.limiter_limit_c,
        "#c0392b",
        &format!("limiter {} max {:.0} C", p.limiter, p.limiter_limit_c),
        "2 3",
    );
    let path = |s: &mut String, v: &[f64], color: &str, wd: f64| {
        let pts: Vec<String> = v
            .iter()
            .enumerate()
            .map(|(i, c)| format!("{:.1},{:.1}", x(p.series.time_s[i]), y(*c)))
            .collect();
        let _ = write!(
            s,
            "<polyline fill=\"none\" stroke=\"{color}\" stroke-width=\"{wd}\" points=\"{}\"/>",
            pts.join(" ")
        );
    };
    path(&mut s, &p.series.oven_c, "#888888", 1.5);
    path(&mut s, &p.series.board_c, "#555555", 1.0);
    path(&mut s, &p.series.coldest_c, "#1d70b8", 2.0);
    path(&mut s, &p.series.hottest_c, "#c0392b", 2.0);
    for i in 0..=5 {
        let c = y_max * i as f64 / 5.0;
        let _ = write!(
            s,
            "<text x=\"{:.1}\" y=\"{:.1}\" text-anchor=\"end\">{:.0}</text>",
            ml - 6.0,
            y(c) + 4.0,
            c
        );
        let t = t_max * i as f64 / 5.0;
        let _ = write!(
            s,
            "<text x=\"{:.1}\" y=\"{:.1}\" text-anchor=\"middle\">{:.0}</text>",
            x(t),
            h - mb + 16.0,
            t
        );
    }
    let _ = write!(
        s,
        "<text x=\"{:.1}\" y=\"{:.1}\" text-anchor=\"middle\">time, s</text>",
        (ml + w - mr) / 2.0,
        h - mb + 34.0
    );
    let _ = write!(
        s,
        "<text x=\"14\" y=\"{:.1}\" transform=\"rotate(-90 14 {:.1})\" text-anchor=\"middle\">temperature, C</text>",
        (mt + h - mb) / 2.0,
        (mt + h - mb) / 2.0
    );
    let ly = h - 14.0;
    let mut lx = ml;
    for (color, label) in [
        ("#888888", "oven target".to_string()),
        ("#555555", "board".to_string()),
        ("#1d70b8", format!("coldest joint {}", p.coldest_joint)),
        ("#c0392b", format!("hottest part, limiter {}", p.limiter)),
    ] {
        let _ = write!(
            s,
            "<line x1=\"{lx}\" x2=\"{:.1}\" y1=\"{ly}\" y2=\"{ly}\" stroke=\"{color}\" stroke-width=\"3\"/>",
            lx + 20.0
        );
        let _ = write!(
            s,
            "<text x=\"{:.1}\" y=\"{:.1}\">{}</text>",
            lx + 24.0,
            ly + 4.0,
            escape(&label)
        );
        lx += 40.0 + label.len() as f64 * 6.5;
    }
    s.push_str("</svg>");
    Some(s)
}
