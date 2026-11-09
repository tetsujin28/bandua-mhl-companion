//! Reconnaissance des caméras et regroupement des fichiers en « clips » pour le rapport.
//! La copie, elle, porte toujours sur la carte entière, à l'identique.

use serde::Serialize;
use std::collections::BTreeMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, PartialOrd, Ord)]
pub enum Brand {
    Red,
    Arri,
    Blackmagic,
    Sony,
    Canon,
    GoPro,
    Dji,
    Generic,
}

impl Brand {
    pub fn label(self) -> &'static str {
        match self {
            Brand::Red => "RED",
            Brand::Arri => "ARRI",
            Brand::Blackmagic => "Blackmagic",
            Brand::Sony => "Sony",
            Brand::Canon => "Canon",
            Brand::GoPro => "GoPro",
            Brand::Dji => "DJI",
            Brand::Generic => "Générique",
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct Clip {
    /// Nom affiché (ex. A001_C003_1106AB pour un .RDC RED)
    pub name: String,
    /// Chemin relatif du clip (dossier ou fichier) depuis la racine de la carte
    pub rel_path: String,
    pub is_folder: bool,
    pub brand: Brand,
    /// Indices des fichiers (dans la liste de la carte) appartenant au clip
    pub files: Vec<usize>,
    pub bytes: u64,
}

const VIDEO_EXT: &[&str] = &[
    "mov", "mp4", "mxf", "braw", "r3d", "ari", "arx", "mts", "m2ts", "avi", "mkv", "crm", "insv",
    "dng", "cine",
];
/// Fichiers annexes copiés mais jamais listés comme clips.
const IGNORED_EXT: &[&str] = &["lrv", "thm", "xml", "rmd", "sidecar", "bin", "bim", "cpi", "mpl"];

fn ext(path: &str) -> String {
    path.rsplit_once('.').map(|(_, e)| e.to_ascii_lowercase()).unwrap_or_default()
}
fn file_name(path: &str) -> &str {
    path.rsplit('/').next().unwrap_or(path)
}
fn stem(name: &str) -> &str {
    name.rsplit_once('.').map(|(s, _)| s).unwrap_or(name)
}
fn parent(path: &str) -> &str {
    path.rsplit_once('/').map(|(p, _)| p).unwrap_or("")
}

/// Motif de nommage ARRI : A001C001_230101_R1AB
fn looks_arri(name: &str) -> bool {
    let b = name.as_bytes();
    b.len() >= 8
        && b[0].is_ascii_uppercase()
        && b[1..4].iter().all(u8::is_ascii_digit)
        && b[4] == b'C'
        && b[5..8].iter().all(u8::is_ascii_digit)
}

fn guess_brand(rel: &str) -> Brand {
    let up = rel.to_ascii_uppercase();
    let e = ext(rel);
    if up.contains(".RDC/") || e == "r3d" {
        Brand::Red
    } else if e == "braw" {
        Brand::Blackmagic
    } else if e == "ari" || e == "arx" || looks_arri(file_name(rel)) {
        Brand::Arri
    } else if up.contains("M4ROOT") || up.contains("XDROOT") || up.starts_with("PRIVATE/") {
        Brand::Sony
    } else if up.contains("GOPRO") {
        Brand::GoPro
    } else if up.contains("DJI") {
        Brand::Dji
    } else if e == "crm" || up.contains("CONTENTS/CLIPS") || up.contains("CANON") {
        Brand::Canon
    } else {
        Brand::Generic
    }
}

/// Regroupe les fichiers (chemins relatifs avec '/', tailles) en clips.
pub fn group_clips(files: &[(String, u64)]) -> Vec<Clip> {
    // clé = chemin du clip ; valeur = clip en construction
    let mut clips: BTreeMap<String, Clip> = BTreeMap::new();

    // Dossiers contenant des séquences d'images (ARRIRAW .ari, DNG, etc.)
    let mut seq_count: BTreeMap<&str, usize> = BTreeMap::new();
    for (rel, _) in files {
        let e = ext(rel);
        if e == "ari" || e == "dng" {
            *seq_count.entry(parent(rel)).or_default() += 1;
        }
    }

    for (i, (rel, size)) in files.iter().enumerate() {
        let e = ext(rel);
        let up = rel.to_ascii_uppercase();

        // 1. RED : tout ce qui est sous un dossier .RDC appartient au clip
        if let Some(pos) = up.find(".RDC/") {
            let clip_path = &rel[..pos + 4];
            let c = clips.entry(clip_path.to_string()).or_insert_with(|| Clip {
                name: stem(file_name(clip_path)).to_string(),
                rel_path: clip_path.to_string(),
                is_folder: true,
                brand: Brand::Red,
                files: vec![],
                bytes: 0,
            });
            c.files.push(i);
            c.bytes += size;
            continue;
        }

        // 2. Séquences d'images : le dossier parent est le clip
        let par = parent(rel);
        if (e == "ari" || e == "dng") && seq_count.get(par).copied().unwrap_or(0) > 1 {
            let c = clips.entry(par.to_string()).or_insert_with(|| Clip {
                name: file_name(par).to_string(),
                rel_path: par.to_string(),
                is_folder: true,
                brand: if e == "ari" { Brand::Arri } else { guess_brand(rel) },
                files: vec![],
                bytes: 0,
            });
            c.files.push(i);
            c.bytes += size;
            continue;
        }

        // 3. Fichiers vidéo : un fichier = un clip
        if VIDEO_EXT.contains(&e.as_str()) && !IGNORED_EXT.contains(&e.as_str()) {
            let name = file_name(rel);
            // ignorer les fichiers cachés macOS (._A001.mov)
            if name.starts_with("._") {
                continue;
            }
            clips.insert(
                rel.clone(),
                Clip {
                    name: stem(name).to_string(),
                    rel_path: rel.clone(),
                    is_folder: false,
                    brand: guess_brand(rel),
                    files: vec![i],
                    bytes: *size,
                },
            );
        }
    }

    let mut out: Vec<Clip> = clips.into_values().collect();
    out.sort_by(|a, b| a.rel_path.cmp(&b.rel_path));
    out
}

/// Marque dominante de la carte (par volume de données).
pub fn dominant_brand(clips: &[Clip]) -> Brand {
    let mut by: BTreeMap<Brand, u64> = BTreeMap::new();
    for c in clips {
        *by.entry(c.brand).or_default() += c.bytes.max(1);
    }
    by.into_iter().max_by_key(|(_, v)| *v).map(|(b, _)| b).unwrap_or(Brand::Generic)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn f(p: &str) -> (String, u64) {
        (p.to_string(), 100)
    }
    #[test]
    fn red_card() {
        let files = vec![
            f("A001_1106AB.RDM/A001_C001_1106XY.RDC/A001_C001_1106XY_001.R3D"),
            f("A001_1106AB.RDM/A001_C001_1106XY.RDC/A001_C001_1106XY_002.R3D"),
            f("A001_1106AB.RDM/A001_C001_1106XY.RDC/A001_C001_1106XY.RMD"),
            f("A001_1106AB.RDM/A001_C002_1106ZZ.RDC/A001_C002_1106ZZ_001.R3D"),
        ];
        let clips = group_clips(&files);
        assert_eq!(clips.len(), 2);
        assert_eq!(clips[0].name, "A001_C001_1106XY");
        assert_eq!(clips[0].files.len(), 3);
        assert_eq!(dominant_brand(&clips), Brand::Red);
    }
    #[test]
    fn arri_sequence_and_braw() {
        let files = vec![
            f("A002C001_231106_R1AB/A002C001_231106_R1AB.0000001.ari"),
            f("A002C001_231106_R1AB/A002C001_231106_R1AB.0000002.ari"),
            f("A003C004_231106_R2CD.mxf"),
            f("A001_11061533_C001.braw"),
            f("A001_11061533_C001.sidecar"),
            f("DCIM/100GOPRO/GX010001.MP4"),
            f("DCIM/100GOPRO/GL010001.LRV"),
        ];
        let clips = group_clips(&files);
        let names: Vec<_> = clips.iter().map(|c| (c.name.as_str(), c.brand)).collect();
        assert!(names.contains(&("A002C001_231106_R1AB", Brand::Arri)));
        assert!(names.contains(&("A003C004_231106_R2CD", Brand::Arri)));
        assert!(names.contains(&("A001_11061533_C001", Brand::Blackmagic)));
        assert!(names.contains(&("GX010001", Brand::GoPro)));
        assert_eq!(clips.len(), 4);
    }
}
