use crate::{Mesh, MeshBuilder};
use std::collections::HashMap;

pub const MM_PER_UNIT: f32 = 2.54;

fn tokens(text: &str) -> Vec<&str> {
    let mut out = Vec::new();
    for line in text.lines() {
        let line = match line.find('#') {
            Some(i) => &line[..i],
            None => line,
        };
        let mut start = None;
        for (i, c) in line.char_indices() {
            let sep = c.is_whitespace() || c == ',';
            let punct = matches!(c, '{' | '}' | '[' | ']');
            if sep || punct {
                if let Some(s) = start.take() {
                    out.push(&line[s..i]);
                }
                if punct {
                    out.push(&line[i..i + 1]);
                }
            } else if start.is_none() {
                start = Some(i);
            }
        }
        if let Some(s) = start {
            out.push(&line[s..]);
        }
    }
    out
}

fn skip_block(t: &[&str], mut i: usize) -> usize {
    let mut depth = 0i32;
    while i < t.len() {
        match t[i] {
            "{" | "[" => depth += 1,
            "}" | "]" => {
                depth -= 1;
                if depth <= 0 {
                    return i + 1;
                }
            }
            _ => {}
        }
        i += 1;
    }
    i
}

fn material(t: &[&str], i: usize) -> ([f32; 3], usize) {
    let end = skip_block(t, i);
    let mut colour = [0.6, 0.6, 0.6];
    let mut k = i;
    while k < end {
        if t[k] == "diffuseColor" && k + 3 < end {
            for c in 0..3 {
                colour[c] = t[k + 1 + c].parse().unwrap_or(0.6);
            }
        }
        k += 1;
    }
    (colour, end)
}

fn numbers(t: &[&str], i: usize) -> (Vec<f32>, usize) {
    let mut out = Vec::new();
    let mut k = i;
    while k < t.len() && t[k] != "[" {
        k += 1;
    }
    k += 1;
    while k < t.len() && t[k] != "]" {
        if let Ok(v) = t[k].parse::<f32>() {
            out.push(v);
        }
        k += 1;
    }
    (out, k + 1)
}

pub fn parse(text: &str) -> Mesh {
    let t = tokens(text);
    let mut defs: HashMap<&str, [f32; 3]> = HashMap::new();
    let mut mesh = MeshBuilder::default();
    let mut i = 0;
    while i < t.len() {
        if t[i] != "Shape" {
            i += 1;
            continue;
        }
        let end = skip_block(&t, i + 1);
        let (mut colour, mut points, mut index) = ([0.6f32; 3], Vec::new(), Vec::new());
        let mut k = i + 1;
        while k < end {
            match t[k] {
                "DEF" if k + 2 < end && t[k + 2] == "Appearance" => {
                    let (c, next) = material(&t, k + 3);
                    defs.insert(t[k + 1], c);
                    colour = c;
                    k = next;
                }
                "DEF" if k + 2 < end && t[k + 2] == "Material" => {
                    let (c, next) = material(&t, k + 3);
                    defs.insert(t[k + 1], c);
                    colour = c;
                    k = next;
                }
                "USE" if k + 1 < end => {
                    if let Some(c) = defs.get(t[k + 1]) {
                        colour = *c;
                    }
                    k += 2;
                }
                "Material" => {
                    let (c, next) = material(&t, k + 1);
                    colour = c;
                    k = next;
                }
                "coordIndex" => {
                    let (v, next) = numbers(&t, k + 1);
                    index = v.into_iter().map(|x| x as i64).collect();
                    k = next;
                }
                "point" => {
                    let (v, next) = numbers(&t, k + 1);
                    points =
                        v.chunks_exact(3).map(|c| [c[0], c[1], c[2]]).collect::<Vec<[f32; 3]>>();
                    k = next;
                }
                _ => k += 1,
            }
        }
        let mut face: Vec<usize> = Vec::new();
        for ix in index.iter().chain(std::iter::once(&-1)) {
            if *ix < 0 {
                if face.len() >= 3 && face.iter().all(|f| *f < points.len()) {
                    for j in 1..face.len() - 1 {
                        let p = [points[face[0]], points[face[j]], points[face[j + 1]]]
                            .map(|p| p.map(|v| v * MM_PER_UNIT));
                        mesh.push_flat(colour, p);
                    }
                }
                face.clear();
            } else {
                face.push(*ix as usize);
            }
        }
        i = end;
    }
    mesh.finish()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_two_triangle_face_with_a_shared_material_reads_in_millimetres() {
        let text = "#VRML V2.0 utf8\nShape { appearance Appearance {material DEF PIN Material { diffuseColor 0.8 0.7 0.1 } }\n}\nShape { geometry IndexedFaceSet { coordIndex [0,1,2,-1,3,0,2,-1]\ncoord Coordinate { point [0 0 0,1 0 0,1 1 0,0 1 0] } }\nappearance Appearance{material USE PIN }\n}";
        let m = parse(text);
        assert_eq!(m.parts.len(), 1);
        assert_eq!(m.parts[0].positions.len(), 6);
        assert!((m.parts[0].colour[0] - 0.8).abs() < 1e-6);
        assert!((m.parts[0].positions[1][0] - 2.54).abs() < 1e-6);
    }
}
