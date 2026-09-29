use agentee_core::geom::P;
use std::fmt::Write;

pub struct Gerber {
    function: String,
    apertures: Vec<String>,
    body: String,
    current: Option<usize>,
    dark: bool,
}

fn coord(v: f64) -> i64 {
    (v * 1e6).round() as i64
}

fn xy(p: P) -> String {
    format!("X{}Y{}", coord(p[0]), coord(-p[1]))
}

impl Gerber {
    pub fn new(function: &str) -> Gerber {
        Gerber {
            function: function.into(),
            apertures: Vec::new(),
            body: String::new(),
            current: None,
            dark: true,
        }
    }

    fn aperture(&mut self, def: String) -> usize {
        let id = match self.apertures.iter().position(|a| *a == def) {
            Some(i) => i,
            None => {
                self.apertures.push(def);
                self.apertures.len() - 1
            }
        };
        if self.current != Some(id) {
            let _ = writeln!(self.body, "D{}*", id + 10);
            self.current = Some(id);
        }
        id
    }

    pub fn polarity(&mut self, dark: bool) {
        if self.dark != dark {
            self.body += if dark { "%LPD*%\n" } else { "%LPC*%\n" };
            self.dark = dark;
        }
    }

    pub fn region(&mut self, pts: &[P]) {
        if pts.len() < 3 {
            return;
        }
        self.body += "G36*\n";
        let _ = writeln!(self.body, "{}D02*", xy(pts[0]));
        for p in &pts[1..] {
            let _ = writeln!(self.body, "{}D01*", xy(*p));
        }
        let _ = writeln!(self.body, "{}D01*", xy(pts[0]));
        self.body += "G37*\n";
    }

    pub fn stroke(&mut self, pts: &[P], width: f64) {
        if pts.len() < 2 || width <= 0.0 {
            return;
        }
        self.aperture(format!("C,{:.6}", width));
        let _ = writeln!(self.body, "{}D02*", xy(pts[0]));
        for p in &pts[1..] {
            let _ = writeln!(self.body, "{}D01*", xy(*p));
        }
    }

    pub fn flash_circle(&mut self, at: P, diameter: f64) {
        self.aperture(format!("C,{:.6}", diameter));
        let _ = writeln!(self.body, "{}D03*", xy(at));
    }

    pub fn is_empty(&self) -> bool {
        self.body.is_empty()
    }

    pub fn finish(self) -> String {
        let mut out = String::new();
        out += "%TF.GenerationSoftware,agentee,agentee,0.1*%\n";
        out += "%TF.SameCoordinates,Original*%\n";
        let _ = writeln!(out, "%TF.FileFunction,{}*%", self.function);
        out += "%TF.FilePolarity,Positive*%\n";
        out += "%FSLAX46Y46*%\n%MOMM*%\n%LPD*%\nG01*\n";
        for (i, a) in self.apertures.iter().enumerate() {
            let _ = writeln!(out, "%ADD{}{}*%", i + 10, a);
        }
        out += &self.body;
        out += "M02*\n";
        out
    }
}
