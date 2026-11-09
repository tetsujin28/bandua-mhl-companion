//! Recherche et lecture des MHL.
//!
//! Deux formats sont reconnus :
//! - **MHL 1.1** (« hashlist ») : un fichier `.mhl` dont les chemins sont relatifs
//!   au dossier qui le contient (Bandua, Silverstack, ShotPut Pro, Hedge…) ;
//! - **ASC MHL v2** : un dossier `ascmhl/` contenant une ou plusieurs générations
//!   `0001_….mhl` et le fichier de chaîne `ascmhl_chain.xml` ; les chemins sont
//!   relatifs au dossier parent de `ascmhl/`.

use crate::hash::{c4_of_bytes, Algo, Expected};
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};
use walkdir::WalkDir;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum Kind {
    #[serde(rename = "mhl1")]
    Mhl1,
    #[serde(rename = "ascmhl")]
    Asc,
}

#[derive(Debug, Clone, Serialize)]
pub struct Entry {
    /// Chemin relatif (séparateur '/') depuis `Package::root`
    pub rel: String,
    pub size: Option<u64>,
    /// Hash retenu pour le contrôle (le plus rapide disponible), `None` si le MHL n'en donne pas
    pub expected: Option<Expected>,
}

/// Un ensemble de fichiers décrit par un MHL (en général une carte).
#[derive(Debug, Clone, Serialize)]
pub struct Package {
    pub id: usize,
    pub kind: Kind,
    /// Fichier `.mhl` (MHL 1.1) ou dossier `ascmhl` (ASC MHL)
    pub manifest: PathBuf,
    /// Dossier auquel les chemins du MHL sont relatifs
    pub root: PathBuf,
    /// Nom affiché
    pub label: String,
    /// Emplacement relatif au dossier déposé (vide si c'est le même)
    pub location: String,
    pub total_files: usize,
    pub total_bytes: u64,
    pub algos: Vec<String>,
    /// Logiciel qui a écrit le MHL, s'il est indiqué
    pub tool: Option<String>,
    /// Nombre de générations (ASC MHL)
    pub generations: usize,
    /// Le fichier de chaîne ASC MHL ne correspond plus aux générations
    pub chain_broken: bool,
    #[serde(skip)]
    pub entries: Vec<Entry>,
}

fn local<'a>(n: &roxmltree::Node<'a, '_>) -> &'a str {
    n.tag_name().name()
}

fn child_text(n: &roxmltree::Node, name: &str) -> Option<String> {
    n.children()
        .find(|c| c.is_element() && local(c).eq_ignore_ascii_case(name))
        .and_then(|c| c.text())
        .map(|t| t.trim().to_string())
}

fn norm_rel(p: &str) -> String {
    let p = p.trim().replace('\\', "/");
    let p = p.strip_prefix("./").unwrap_or(&p);
    p.trim_start_matches('/').to_string()
}

fn pick(hashes: Vec<Expected>) -> Option<Expected> {
    hashes.into_iter().min_by_key(|e| e.algo.cost())
}

fn tool_of(doc: &roxmltree::Document) -> Option<String> {
    let ci = doc.descendants().find(|n| n.is_element() && local(n) == "creatorinfo")?;
    let t = ci.descendants().find(|n| n.is_element() && local(n) == "tool")?;
    let name = t.text().map(str::trim).unwrap_or("").to_string();
    let ver = t.attribute("version").unwrap_or("");
    let s = format!("{name} {ver}").trim().to_string();
    (!s.is_empty()).then_some(s)
}

fn read_xml(path: &Path) -> Result<String, String> {
    let bytes = fs::read(path).map_err(|e| format!("{} : {e}", path.display()))?;
    // certains logiciels écrivent un BOM UTF-8
    let bytes = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]).unwrap_or(&bytes);
    String::from_utf8(bytes.to_vec()).map_err(|_| format!("{} : encodage non UTF-8", path.display()))
}

/// MHL 1.1
pub fn parse_mhl1(path: &Path) -> Result<(Vec<Entry>, Option<String>), String> {
    let txt = read_xml(path)?;
    let doc = roxmltree::Document::parse(&txt).map_err(|e| format!("{} : XML illisible ({e})", path.display()))?;
    let mut out = vec![];
    for h in doc.descendants().filter(|n| n.is_element() && local(n) == "hash") {
        let Some(rel) = child_text(&h, "file") else { continue };
        let size = child_text(&h, "size").and_then(|s| s.parse().ok());
        let hashes = h
            .children()
            .filter(|c| c.is_element())
            .filter_map(|c| {
                let (algo, loose) = Algo::from_tag(local(&c))?;
                let v = c.text()?.trim().to_string();
                (!v.is_empty()).then_some(Expected { algo, value: v, loose })
            })
            .collect();
        out.push(Entry { rel: norm_rel(&rel), size, expected: pick(hashes) });
    }
    Ok((out, tool_of(&doc)))
}

/// Générations d'un dossier `ascmhl`, triées.
fn generations(dir: &Path) -> Vec<PathBuf> {
    let mut v: Vec<PathBuf> = fs::read_dir(dir)
        .map(|rd| {
            rd.filter_map(|e| e.ok())
                .map(|e| e.path())
                .filter(|p| p.extension().is_some_and(|x| x.eq_ignore_ascii_case("mhl")))
                .filter(|p| !p.file_name().unwrap().to_string_lossy().starts_with("._"))
                .collect()
        })
        .unwrap_or_default();
    v.sort();
    v
}

/// ASC MHL : fusionne toutes les générations (la plus récente l'emporte).
/// Renvoie aussi l'état de la chaîne (`true` si elle est rompue).
pub fn parse_asc(dir: &Path) -> Result<(Vec<Entry>, Option<String>, usize, bool), String> {
    let gens = generations(dir);
    if gens.is_empty() {
        return Err(format!("{} : aucune génération .mhl", dir.display()));
    }
    // chemin → (taille, hash par algorithme)
    let mut files: BTreeMap<String, (Option<u64>, BTreeMap<Algo, Expected>)> = BTreeMap::new();
    let mut tool = None;
    for g in &gens {
        let txt = read_xml(g)?;
        let doc = roxmltree::Document::parse(&txt).map_err(|e| format!("{} : XML illisible ({e})", g.display()))?;
        tool = tool_of(&doc).or(tool);
        for h in doc.descendants().filter(|n| n.is_element() && local(n) == "hash") {
            let Some(pnode) = h.children().find(|c| c.is_element() && local(c) == "path") else { continue };
            let Some(rel) = pnode.text() else { continue };
            let rel = norm_rel(rel);
            let size = pnode.attribute("size").and_then(|s| s.trim().parse().ok());
            let e = files.entry(rel).or_insert((None, BTreeMap::new()));
            if size.is_some() {
                e.0 = size;
            }
            for c in h.children().filter(|c| c.is_element()) {
                let Some((algo, loose)) = Algo::from_tag(local(&c)) else { continue };
                // une vérification ratée enregistrée ne remplace pas le hash d'origine
                if c.attribute("action").is_some_and(|a| a.eq_ignore_ascii_case("failed")) {
                    continue;
                }
                if let Some(v) = c.text().map(str::trim).filter(|v| !v.is_empty()) {
                    e.1.insert(algo, Expected { algo, value: v.to_string(), loose });
                }
            }
        }
    }
    let entries = files
        .into_iter()
        .map(|(rel, (size, h))| Entry { rel, size, expected: pick(h.into_values().collect()) })
        .collect();
    Ok((entries, tool, gens.len(), chain_broken(dir)))
}

/// Compare chaque génération à l'empreinte C4 notée dans `ascmhl_chain.xml`.
/// Sans fichier de chaîne, rien à signaler.
fn chain_broken(dir: &Path) -> bool {
    let Ok(txt) = read_xml(&dir.join("ascmhl_chain.xml")) else { return false };
    let Ok(doc) = roxmltree::Document::parse(&txt) else { return true };
    for hl in doc.descendants().filter(|n| n.is_element() && local(n) == "hashlist") {
        let (Some(p), Some(c4)) = (child_text(&hl, "path"), child_text(&hl, "c4")) else { continue };
        match fs::read(dir.join(norm_rel(&p))) {
            Ok(bytes) if c4_of_bytes(&bytes) == c4 => {}
            _ => return true,
        }
    }
    false
}

pub fn is_junk_name(name: &str) -> bool {
    matches!(
        name,
        ".DS_Store" | ".Spotlight-V100" | ".Trashes" | ".fseventsd" | ".TemporaryItems" | ".DocumentRevisions-V100"
            | "System Volume Information" | "$RECYCLE.BIN" | "Thumbs.db" | "desktop.ini"
    ) || name.starts_with("._")
}

/// Profondeur maximale de recherche des MHL sous le dossier déposé.
const MAX_DEPTH: usize = 10;

/// Cherche les MHL à partir d'un dossier ou d'un fichier déposé.
pub fn discover(path: &Path) -> Result<Vec<Package>, String> {
    let mut v1: Vec<PathBuf> = vec![];
    let mut asc: Vec<PathBuf> = vec![];
    let is_asc_dir = |p: &Path| p.file_name().is_some_and(|n| n.eq_ignore_ascii_case("ascmhl"));

    if path.is_file() {
        if !path.extension().is_some_and(|x| x.eq_ignore_ascii_case("mhl")) {
            return Err("not_mhl".into());
        }
        match path.parent() {
            Some(p) if is_asc_dir(p) => asc.push(p.to_path_buf()),
            _ => v1.push(path.to_path_buf()),
        }
    } else if path.is_dir() {
        if is_asc_dir(path) {
            asc.push(path.to_path_buf());
        } else {
            let walker = WalkDir::new(path)
                .follow_links(false)
                .max_depth(MAX_DEPTH)
                .sort_by_file_name()
                .into_iter()
                .filter_entry(|e| e.depth() == 0 || !is_junk_name(&e.file_name().to_string_lossy()));
            let mut it = walker;
            while let Some(entry) = it.next() {
                let Ok(entry) = entry else { continue };
                let p = entry.path();
                if entry.file_type().is_dir() && entry.depth() > 0 && is_asc_dir(p) {
                    asc.push(p.to_path_buf());
                    it.skip_current_dir();
                } else if entry.file_type().is_file() && p.extension().is_some_and(|x| x.eq_ignore_ascii_case("mhl")) {
                    v1.push(p.to_path_buf());
                }
            }
        }
    } else {
        return Err("not_found".into());
    }

    let base = if path.is_dir() { path.to_path_buf() } else { path.parent().unwrap_or(path).to_path_buf() };
    let location = |root: &Path| -> String {
        root.strip_prefix(&base)
            .ok()
            .map(|r| r.components().map(|c| c.as_os_str().to_string_lossy().into_owned()).collect::<Vec<_>>().join("/"))
            .unwrap_or_else(|| root.display().to_string())
    };

    let mut out = vec![];
    let mut asc_roots = BTreeSet::new();
    for dir in asc {
        let root = dir.parent().unwrap_or(&dir).to_path_buf();
        let (entries, tool, gens, broken) = parse_asc(&dir)?;
        asc_roots.insert(root.clone());
        out.push(make(Kind::Asc, dir.clone(), root.clone(), folder_name(&root), location(&root), entries, tool, gens, broken));
    }
    for f in v1 {
        let root = f.parent().unwrap_or(Path::new(".")).to_path_buf();
        // un dossier déjà décrit par un ASC MHL n'est pas contrôlé deux fois
        if asc_roots.contains(&root) {
            continue;
        }
        let (entries, tool) = parse_mhl1(&f)?;
        let label = f.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
        out.push(make(Kind::Mhl1, f.clone(), root.clone(), label, location(&root), entries, tool, 1, false));
    }
    out.retain(|p| p.total_files > 0);
    out.sort_by(|a, b| a.root.cmp(&b.root).then(a.label.cmp(&b.label)));
    for (i, p) in out.iter_mut().enumerate() {
        p.id = i;
    }
    Ok(out)
}

fn folder_name(p: &Path) -> String {
    p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| p.display().to_string())
}

/// Chemin absolu d'une entrée (les chemins absolus éventuels sont respectés).
pub fn resolve(root: &Path, rel: &str) -> PathBuf {
    let p = Path::new(rel);
    if p.is_absolute() || rel.starts_with("//") {
        return p.to_path_buf();
    }
    let mut out = root.to_path_buf();
    for part in rel.split('/').filter(|s| !s.is_empty() && *s != ".") {
        out.push(part);
    }
    out
}

#[allow(clippy::too_many_arguments)]
fn make(
    kind: Kind,
    manifest: PathBuf,
    root: PathBuf,
    label: String,
    location: String,
    entries: Vec<Entry>,
    tool: Option<String>,
    generations: usize,
    chain_broken: bool,
) -> Package {
    let total_bytes = entries
        .iter()
        .map(|e| e.size.or_else(|| fs::metadata(resolve(&root, &e.rel)).ok().map(|m| m.len())).unwrap_or(0))
        .sum();
    let algos: BTreeSet<&str> = entries.iter().filter_map(|e| e.expected.as_ref()).map(|e| e.algo.label()).collect();
    Package {
        id: 0,
        kind,
        manifest,
        root,
        label,
        location,
        total_files: entries.len(),
        total_bytes,
        algos: algos.into_iter().map(String::from).collect(),
        tool,
        generations,
        chain_broken,
        entries,
    }
}
